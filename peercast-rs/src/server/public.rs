//! 公開ディレクトリと共有のアセット (core/common/public.cpp の `PublicController`、assets.cpp の
//! `AssetsController`、mapper.cpp の `FileSystemMapper`)。判断は `crate::public` と `crate::mapper`。

use std::collections::BTreeMap;
use std::sync::Arc;

use super::error::{Error, Result};
use super::html::{Scope, Template};
use super::http::{rfc1123_time, Headers, Request, Response};
use super::peercast::Peercast;
use super::stream::{FileStream, StreamExt};
use super::sys;
use crate::json::Value as Json;
use crate::template::Value;

/// `FileSystemMapper`
pub struct Mapper {
    virtual_path: Vec<u8>,
    document_root: Vec<u8>,
}

impl Mapper {
    pub fn new(virtual_path: &[u8], document_root: &[u8]) -> Result<Mapper> {
        match sys::real_path(document_root) {
            Some(dr) => Ok(Mapper { virtual_path: virtual_path.to_vec(), document_root: dr }),
            None => Err(Error::general(format!("Document root `{}` inaccessible", String::from_utf8_lossy(document_root)))),
        }
    }

    /// `toLocalFilePath`: ファイルの場所と言語。なければ空
    pub fn local_file_path(&self, vpath: &[u8], langs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
        let file_path = match crate::mapper::local_path(&self.virtual_path, &self.document_root, vpath) {
            Some(p) => p,
            None => return (Vec::new(), Vec::new()),
        };
        let resolved = crate::mapper::candidates(&file_path, langs).into_iter().find_map(|(p, l)| sys::real_path(&p).map(|r| (r, l)));
        let (path, lang) = match resolved {
            Some(r) => r,
            None => {
                crate::log_error!("Cannot resolve path {}", String::from_utf8_lossy(&file_path));
                return (Vec::new(), Vec::new());
            }
        };
        if !crate::mapper::inside(&self.document_root, &path) {
            crate::log_error!("Possible directory traversal attack!");
            return (Vec::new(), Vec::new());
        }
        (path, lang)
    }
}

/// `jsonToAmf`
fn json_to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Int(i) => Value::Number(*i as f64),
        Json::UInt(u) => Value::Number(*u as f64),
        Json::Float(f) => Value::Number(*f),
        Json::Str(s) => Value::String(s.clone()),
        Json::Array(a) => Value::StrictArray(a.iter().map(json_to_value).collect()),
        Json::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), json_to_value(v))).collect()),
    }
}

fn json_at<'a>(v: &'a Json, path: &[&str]) -> Option<&'a Json> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p.as_bytes())?;
    }
    Some(cur)
}

/// json の数の比較 (`a < b`)
fn json_num(v: Option<&Json>) -> f64 {
    match v {
        Some(Json::Int(i)) => *i as f64,
        Some(Json::UInt(u)) => *u as f64,
        Some(Json::Float(f)) => *f,
        _ => 0.0,
    }
}

fn read_file(path: &[u8]) -> Result<Vec<u8>> {
    let mut f = FileStream::open_read(path)?;
    let len = f.length().max(0) as usize;
    f.read_n(len)
}

/// `PublicController::operator()`
pub fn public_controller(pc: &Arc<Peercast>, req: &Request) -> Result<Response> {
    let mapper = Mapper::new(b"/public", &[&pc.app.html_path[..], b"public"].concat())?;
    let langs = crate::public::acceptable_languages(&req.headers.get(b"Accept-Language"));
    let req_scope = || Scope::Request { host: req.headers.get(b"Host"), path: req.path.clone(), query: req.query_string.clone() };
    match crate::public::route(&req.path) {
        crate::public::Route::RedirectSlash => Ok(Response::redirect_to(b"/public/")),
        crate::public::Route::RedirectIndex => Ok(Response::redirect_to(b"/public/index.html")),
        crate::public::Route::IndexTxt => {
            let tip = pc.servmgr.settings().server_host.str();
            let body = crate::jrpc::channel_index(&mut super::jrpc_host::JrpcHost { pc }, tip.as_bytes())
                .map_err(|e| Error::general(String::from_utf8_lossy(&e).into_owned()))?;
            Ok(Response::ok(Headers::from(&[("Content-Type", b"text/plain")]), body))
        }
        crate::public::Route::Play => {
            let id = crate::servhs::Query::new(&req.query_string).get(b"id");
            let id = super::pcstr::cut(&id);
            let (_, found) = pc.servmgr.get_channel(pc, &id, true);
            if !found {
                return Ok(Response::not_found(b"File not found"));
            }
            let ch = match pc.chanmgr.find_channel_by_id(&crate::gnuid::from_str(&id)) {
                Some(c) => c,
                None => return Ok(Response::not_found(b"File not found")),
            };
            let (path, lang) = mapper.local_file_path(&req.path, &langs);
            let mut locals = BTreeMap::new();
            locals.insert(b"channel".to_vec(), ch.state(pc));
            let data = read_file(&path)?;
            let mut t = Template::new(pc, data, &req.query_string);
            t.prepend_scope(req_scope());
            t.prepend_scope(Scope::Generic(locals));
            let body = t.run()?;
            let mut h = Headers::new();
            h.set(b"Content-Type", b"text/html");
            if !lang.is_empty() {
                h.set(b"Content-Language", &lang);
            }
            h.set(b"Content-Length", body.len().to_string().as_bytes());
            Ok(Response::ok(h, body))
        }
        crate::public::Route::File => {
            let (path, lang) = mapper.local_file_path(&req.path, &langs);
            if path.is_empty() {
                return Ok(Response::not_found(b"File not found"));
            }
            crate::log_debug!("Writing `{}` lang={}", String::from_utf8_lossy(&path), String::from_utf8_lossy(&lang));
            let ty = crate::public::mime_type(&path);
            let r: Result<Vec<u8>> = if ty == b"text/html" {
                (|| {
                    let data = read_file(&path)?;
                    let mut locals = BTreeMap::new();
                    let channels = super::jrpc_host::invoke(pc, "getChannels").map_err(|e| Error::general(String::from_utf8_lossy(&e).into_owned()))?;
                    let mut broadcasting: Vec<Json> = match channels {
                        Json::Array(a) => a.into_iter().filter(|c| matches!(json_at(c, &["status", "isBroadcasting"]), Some(Json::Bool(true)))).collect(),
                        _ => Vec::new(),
                    };
                    broadcasting.sort_by(|a, b| {
                        json_num(json_at(a, &["status", "totalDirects"]))
                            .partial_cmp(&json_num(json_at(b, &["status", "totalDirects"])))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    locals.insert(b"broadcastingChannels".to_vec(), json_to_value(&Json::Array(broadcasting)));
                    let found = super::jrpc_host::invoke(pc, "getChannelsFound").map_err(|e| Error::general(String::from_utf8_lossy(&e).into_owned()))?;
                    locals.insert(b"channelsFound".to_vec(), json_to_value(&found));
                    let mut t = Template::new(pc, data, &req.query_string);
                    t.prepend_scope(super::html::root_scope(pc));
                    t.prepend_scope(req_scope());
                    t.prepend_scope(Scope::Generic(locals));
                    let (out, r) = t.run_partial();
                    match r {
                        Ok(()) => Ok(out),
                        Err(e) if e.is_stream() => {
                            crate::log_debug!("StreamException in operator()");
                            Ok(out)
                        }
                        Err(e) => Err(e),
                    }
                })()
            } else {
                match read_file(&path) {
                    Ok(d) => Ok(d),
                    Err(e) if e.is_stream() => {
                        crate::log_debug!("StreamException in operator()");
                        Ok(Vec::new())
                    }
                    Err(e) => Err(e),
                }
            };
            let body = r?;
            let mut h = Headers::new();
            h.set(b"Content-Type", ty);
            h.set(b"Content-Length", body.len().to_string().as_bytes());
            if !lang.is_empty() {
                h.set(b"Content-Language", &lang);
            }
            Ok(Response::ok(h, body))
        }
    }
}

/// ファイルの更新時刻 (なければ -1)
fn mtime(path: &[u8]) -> i64 {
    let m = sys::bytes_to_path(path).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    match m {
        Some(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        },
        None => -1,
    }
}

/// `AssetsController::operator()`
pub fn assets_controller(pc: &Arc<Peercast>, req: &Request) -> Result<Response> {
    let mapper = Mapper::new(b"/assets", &[&pc.app.html_path[..], b"assets"].concat())?;
    let (path, _) = mapper.local_file_path(&req.path, &[]);
    if path.is_empty() {
        return Ok(Response::not_found(b"File not found"));
    }
    let last_modified = mtime(&path);
    if crate::public::not_modified(last_modified, &req.headers.get(b"If-Modified-Since")) {
        let mut h = Headers::new();
        h.set(b"Last-Modified", rfc1123_time(last_modified).as_bytes());
        return Ok(Response::not_modified(h));
    }
    let body = read_file(&path)?;
    let mut h = Headers::new();
    h.set(b"Content-Type", crate::public::assets_mime_type(&path));
    h.set(b"Content-Length", body.len().to_string().as_bytes());
    if last_modified != -1 {
        h.set(b"Last-Modified", rfc1123_time(last_modified).as_bytes());
    }
    Ok(Response::ok(h, body))
}
