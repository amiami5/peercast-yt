//! HTML のページ (core/common/html.cpp の `HTML` と、template.cpp のスコープ
//! (`GenericScope`、`HTTPRequestScope`、`RootObjectScope`))。
//!
//! テンプレートの読み出しと評価は `crate::template`。ここは変数の値 (スコープ) と入出力を貸す。

use std::collections::BTreeMap;
use std::sync::Arc;

use super::error::{Error, Result};
use super::http::{rfc1123_time, HTTP_SC_OK, PCX_AGENT};
use super::pcstr;
use super::peercast::Peercast;
use super::state::{n, obj, s, Value};
use super::stream::{FileStream, Stream, StreamExt, StringStream};
use super::sys;
use crate::reader::Abort;
use crate::template::{self, Engine};

/// テンプレートの変数のスコープ (`Template::Scope`)
pub enum Scope {
    /// `GenericScope`
    Generic(BTreeMap<Vec<u8>, Value>),
    /// `HTTPRequestScope`
    Request { host: Vec<u8>, path: Vec<u8>, query: Vec<u8> },
    /// `RootObjectScope`
    Root(BTreeMap<Vec<u8>, Value>),
}

/// `writeObjectProperty`: `a.b.c` の形の名前で、オブジェクトの中を探す。
/// 最後の名前でないものはオブジェクト (ECMA 配列でない) でなければ見付からない。
fn object_property(obj: &Value, name: &[u8]) -> template::Result<Option<Value>> {
    match name.iter().position(|&c| c == b'.') {
        None => Ok(obj.object()?.get(name).cloned()),
        Some(i) => match obj.object()?.get(&name[..i]) {
            Some(v @ Value::Object(_)) => object_property(v, &name[i + 1..]),
            _ => Ok(None),
        },
    }
}

impl Scope {
    /// `writeVariable`
    fn lookup(&self, pc: &Peercast, name: &[u8]) -> template::Result<Option<Value>> {
        match self {
            Scope::Generic(vars) | Scope::Root(vars) => {
                if let Some(v) = vars.get(name) {
                    return Ok(Some(v.clone()));
                }
                if let Some(i) = name.iter().position(|&c| c == b'.') {
                    if let Some(o) = vars.get(&name[..i]) {
                        return object_property(o, &name[i + 1..]);
                    }
                }
                Ok(None)
            }
            Scope::Request { host, path, query } => Ok(match name {
                b"request.host" => Some(if host.is_empty() {
                    let h = pc.servmgr.settings().server_host;
                    s([h.ip_str().as_bytes(), b":", h.port.to_string().as_bytes()].concat())
                } else {
                    s(host)
                }),
                b"request.path" => Some(s(path)),
                b"request.queryString" => Some(s(query)),
                b"request.search" => Some(s(if query.is_empty() { Vec::new() } else { [&b"?"[..], query].concat() })),
                _ => None,
            }),
        }
    }
}

/// `RootObjectScope`: `servMgr` などの状態
pub fn root_scope(pc: &Arc<Peercast>) -> Scope {
    let log = super::log::with_buffer(|b| {
        obj(vec![
            ("dumpHTML", s(b.dump_html())),
            ("logListeners", super::state::arr(b.listener_ids().into_iter().map(|i| n(i as f64)).collect())),
        ])
    });
    let stats = obj(super::stats::state().into_iter().map(|(k, v)| (k, s(v))).collect());
    let mut m = BTreeMap::new();
    m.insert(b"servMgr".to_vec(), pc.servmgr.state(pc));
    m.insert(b"chanMgr".to_vec(), pc.chanmgr.state(pc));
    m.insert(b"stats".to_vec(), stats);
    m.insert(b"notificationBuffer".to_vec(), pc.notifications().state());
    m.insert(b"sys".to_vec(), obj(vec![("log", log), ("time", n(sys::get_time() as f64))]));
    m.insert(b"app".to_vec(), pc.app.state());
    m.insert(b"ypList".to_vec(), pc.yplist.state());
    Scope::Root(m)
}

/// `Template` と、テンプレートに貸す入出力 (`rusttemplate.h` の `TemplateHost`)
pub struct Template<'a> {
    pc: &'a Peercast,
    input: StringStream,
    pub out: Vec<u8>,
    /// 先頭が最初に探すスコープ
    scopes: Vec<Scope>,
    pub selected_fragment: Vec<u8>,
    current_fragment: Vec<u8>,
    /// コールバックの中で起きた誤り
    error: Option<Error>,
}

impl<'a> Template<'a> {
    /// `Template(args)`: `page` に引数を入れたスコープを置く
    pub fn new(pc: &'a Peercast, input: Vec<u8>, args: &[u8]) -> Template<'a> {
        let q = crate::servhs::Query::new(args);
        let page: BTreeMap<Vec<u8>, Value> = q.keys().map(|k| (k.clone(), s(q.get(k)))).collect();
        let mut vars = BTreeMap::new();
        vars.insert(b"page".to_vec(), Value::Object(page));
        Template {
            pc,
            input: StringStream::from(input),
            out: Vec::new(),
            scopes: vec![Scope::Generic(vars)],
            selected_fragment: Vec::new(),
            current_fragment: Vec::new(),
            error: None,
        }
    }

    /// `prependScope`
    pub fn prepend_scope(&mut self, scope: Scope) {
        self.scopes.insert(0, scope);
    }

    fn fail<T>(&mut self, e: Error) -> std::result::Result<T, Abort> {
        self.error = Some(e);
        Err(Abort)
    }

    /// `readTemplate`: 全部を読んで、出力を返す
    pub fn run(&mut self) -> Result<Vec<u8>> {
        let (out, r) = self.run_partial();
        r.map(|_| out)
    }

    /// `readTemplate`: 出力 (誤りのときはそこまでのもの) と、誤り
    pub fn run_partial(&mut self) -> (Vec<u8>, Result<()>) {
        let r = {
            let mut e = Engine::new(self);
            let r = e.read_template(true);
            e.finish(r)
        };
        let out = std::mem::take(&mut self.out);
        (out, self.convert(r.map(|_| ())))
    }

    /// `evalExpression(const std::string&)`
    pub fn eval_str(&mut self, expr: &[u8]) -> Result<Value> {
        let r = {
            let mut e = Engine::new(self);
            let r = e.eval_str(expr);
            e.finish(r)
        };
        match r {
            Ok(v) => Ok(v),
            Err(e) => Err(self.convert(Err(e)).unwrap_err()),
        }
    }

    fn convert(&mut self, r: template::Result<()>) -> Result<()> {
        match r {
            Ok(()) => Ok(()),
            Err(template::Error::Abort) => Err(self.error.take().unwrap_or_else(|| Error::general("Template: aborted"))),
            Err(template::Error::Stream(m)) => Err(Error::stream(String::from_utf8_lossy(&m).into_owned())),
            Err(template::Error::General(m))
            | Err(template::Error::Runtime(m))
            | Err(template::Error::OutOfRange(m))
            | Err(template::Error::InvalidArgument(m)) => Err(Error::general(String::from_utf8_lossy(&m).into_owned())),
        }
    }
}

impl template::Host for Template<'_> {
    fn read_char(&mut self) -> std::result::Result<u8, Abort> {
        match self.input.read_char() {
            Ok(c) => Ok(c),
            Err(e) => self.fail(e),
        }
    }

    fn eof(&mut self) -> std::result::Result<bool, Abort> {
        match self.input.eof() {
            Ok(b) => Ok(b),
            Err(e) => self.fail(e),
        }
    }

    fn position(&mut self) -> std::result::Result<i32, Abort> {
        Ok(self.input.position())
    }

    fn seek(&mut self, pos: i32) -> std::result::Result<(), Abort> {
        match self.input.seek_to(pos) {
            Ok(()) => Ok(()),
            Err(e) => self.fail(e),
        }
    }

    fn write(&mut self, data: &[u8]) -> std::result::Result<(), Abort> {
        self.out.extend_from_slice(data);
        Ok(())
    }

    fn lookup(&mut self, name: &[u8]) -> std::result::Result<Value, Abort> {
        let name = pcstr::cut(name);
        for sc in &self.scopes {
            match sc.lookup(self.pc, &name) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => {}
                Err(e) => {
                    let m = match e {
                        template::Error::Runtime(m) | template::Error::General(m) => m,
                        _ => b"lookup".to_vec(),
                    };
                    return self.fail(Error::general(String::from_utf8_lossy(&m).into_owned()));
                }
            }
        }
        Ok(match &name[..] {
            b"TRUE" => s("1"),
            b"FALSE" => s("0"),
            b"true" => Value::Bool(true),
            b"false" => Value::Bool(false),
            _ => Value::Null,
        })
    }

    fn push_scope(&mut self) {
        self.scopes.insert(0, Scope::Generic(BTreeMap::new()));
    }

    fn pop_scope(&mut self) {
        if !self.scopes.is_empty() {
            self.scopes.remove(0);
        }
    }

    fn front_is_generic(&mut self) -> bool {
        matches!(self.scopes.first(), Some(Scope::Generic(_)))
    }

    fn set_front(&mut self, name: &[u8], value: &Value) -> std::result::Result<(), Abort> {
        match self.scopes.first_mut() {
            Some(Scope::Generic(vars)) => {
                vars.insert(name.to_vec(), value.clone());
                Ok(())
            }
            _ => self.fail(Error::general("Cannot change this scope.")),
        }
    }

    fn regex_check(&mut self, pattern: &[u8]) -> std::result::Result<(), Abort> {
        match super::regex::Regex::new(pattern) {
            Ok(_) => Ok(()),
            Err(e) => self.fail(Error::general(e.to_string())),
        }
    }

    fn regex_match(&mut self, pattern: &[u8], subject: &[u8]) -> std::result::Result<bool, Abort> {
        match super::regex::Regex::new(pattern) {
            Ok(r) => Ok(r.is_match(subject)),
            Err(e) => self.fail(Error::general(e.to_string())),
        }
    }

    fn selected_fragment(&mut self) -> Vec<u8> {
        self.selected_fragment.clone()
    }

    fn current_fragment(&mut self) -> Vec<u8> {
        self.current_fragment.clone()
    }

    fn set_current_fragment(&mut self, f: &[u8]) {
        self.current_fragment = f.to_vec();
    }

    fn log_error(&mut self, msg: &str) {
        crate::log_error!("{}", msg);
    }
}

// ---------------------------------------------------------------- HTML

/// `HTML::writeOK`: 200 の応答のヘッダー (行の終わりは CRLF)
pub fn write_ok(out: &mut dyn Stream, content: &str, additional: &[(&str, String)]) -> Result<()> {
    let mut h = Vec::new();
    for l in [
        HTTP_SC_OK.to_string(),
        format!("Server: {}", PCX_AGENT),
        "Connection: close".to_string(),
        format!("Content-Type: {}", content),
        format!("Date: {}", rfc1123_time(sys::get_time() as i64)),
    ] {
        h.extend_from_slice(l.as_bytes());
        h.extend_from_slice(b"\r\n");
    }
    for (k, v) in additional {
        h.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
    }
    h.extend_from_slice(b"\r\n");
    out.write(&h)
}

/// `HTML::writeTemplate`: テンプレートを評価して書く。誤りは文言とファイル名を書く
pub fn write_template(pc: &Arc<Peercast>, out: &mut dyn Stream, file_name: &[u8], args: &[u8], scopes: Vec<Scope>) -> Result<()> {
    let r = (|| -> Result<Vec<u8>> {
        let mut f = FileStream::open_read(file_name)?;
        let len = f.length().max(0) as usize;
        let data = f.read_n(len)?;
        let mut t = Template::new(pc, data, args);
        t.prepend_scope(root_scope(pc));
        for sc in scopes {
            t.prepend_scope(sc);
        }
        t.selected_fragment = crate::servhs::Query::new(args).get(b"fragment");
        t.run()
    })();
    match r {
        Ok(body) => out.write(&body),
        Err(e) => out.write(&[e.msg.as_bytes(), b" : ", file_name].concat()),
    }
}

/// ファイルの更新時刻 (UNIX 時間)
fn mtime(path: &[u8]) -> Option<i64> {
    let p = sys::bytes_to_path(path)?;
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(match m.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    })
}

/// `HTML::writeRawFile`: ファイルをそのまま書く (読めなければ何も書かない)
pub fn write_raw_file(out: &mut dyn Stream, file_name: &[u8], mime: &str) -> Result<()> {
    let mut f = match FileStream::open_read(file_name) {
        Ok(f) => f,
        Err(e) if e.is_stream() => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut add = Vec::new();
    match mtime(file_name) {
        Some(t) => add.push(("Last-Modified", rfc1123_time(t))),
        None => crate::log_error!("Failed to get mtime of {}", String::from_utf8_lossy(file_name)),
    }
    let len = f.length();
    add.push(("Content-Length", len.to_string()));
    let r = (|| -> Result<()> {
        write_ok(out, mime, &add)?;
        f.write_to(out, len.max(0) as usize)
    })();
    match r {
        Err(e) if e.is_stream() => Ok(()),
        r => r,
    }
}

/// `CMD_redirect` のページ (`HTML::setRefreshURL`、`startHTML` など)。meta のタグは C++ 版と同じく
/// 511 バイトで切れる (`char buf[512]` に snprintf する)
pub fn refresh_page(url: &[u8]) -> Vec<u8> {
    let mut tag = [&b"meta http-equiv=\"refresh\" content=\"0;URL="[..], &crate::cgi::escape_html(url), b"\""].concat();
    tag.truncate(511);
    [
        &b"<html><head><title></title><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta><"[..],
        &tag,
        b"></meta></head><body><h3>Please wait...</h3></body></html>",
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_properties() {
        let inner = super::super::state::obj(vec![("c", s("1"))]);
        let v = super::super::state::obj(vec![("b", inner), ("s", s("x"))]);
        assert_eq!(object_property(&v, b"b.c").unwrap(), Some(s("1")));
        assert_eq!(object_property(&v, b"s.c").unwrap(), None);
        assert_eq!(object_property(&v, b"zz").unwrap(), None);
        assert!(object_property(&s("x"), b"a").is_err());
    }

    #[test]
    fn refresh_page_truncates_tag() {
        let p = refresh_page(b"http://a/");
        assert!(p.ends_with(b"<meta http-equiv=\"refresh\" content=\"0;URL=http://a/\"></meta></head><body><h3>Please wait...</h3></body></html>"));
        let long = vec![b'a'; 1000];
        let p = refresh_page(&long);
        let start = p.windows(5).position(|w| w == b"<meta").unwrap();
        let second = start + 1 + p[start + 1..].windows(5).position(|w| w == b"<meta").unwrap();
        let end = second + p[second..].iter().position(|&c| c == b'>').unwrap();
        assert_eq!(end - second - 1, 511);
    }
}
