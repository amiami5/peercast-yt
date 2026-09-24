//! 受け付けた接続の最初の要求の処理 (core/common/servhs.cpp)。要求の解釈と判断は `crate::servhs`。
//!
//! ソケットを渡してしまうもの (放送の受け付け、GIV、ストリーム、CIN) は `Conn` を受け取り、それ以外は
//! ソケットを読み書きする `Http` を受け取る。

use std::collections::BTreeMap;
use std::sync::Arc;

use super::chanhit::ChanHit;
use super::chaninfo::{self as ci, ChanInfo};
use super::channel;
use super::cookie::Cookie;
use super::error::{Error, Result};
use super::host::Host;
use super::html::{self, Scope};
use super::http::{
    http_error, http_error_msg, Headers, Http, Response, HTTP_SC_BADREQUEST, HTTP_SC_FORBIDDEN, HTTP_SC_FOUND, HTTP_SC_NOTFOUND, HTTP_SC_OK,
    HTTP_SC_SERVERERROR, HTTP_SC_UNAUTHORIZED, HTTP_SC_UNAVAILABLE, HTTP_SC_URITOOLONG, MAX_REQUEST_BODY, MIME_TEXT, MIME_XML, PCX_AGENT,
};
use super::pcstr::{self, PcString, StrType};
use super::peercast::Peercast;
use super::playlist::{self, PlayList};
use super::servent::{self as svt, is_localhost, Conn, Servent};
use super::servfilter as sf;
use super::servmgr::{AUTH_COOKIE, AUTH_HTTPBASIC};
use super::stream::{Stream, StreamExt, StringStream};
use super::sys;
use crate::servhs::{self, Query};

/// ソケットを持たない処理で使うもの
struct Ctx<'a> {
    pc: &'a Arc<Peercast>,
    sv: &'a Arc<Servent>,
    /// `sock->getLocalHost()`
    local_host: Host,
}

impl Ctx<'_> {
    fn host(&self) -> Host {
        self.sv.host()
    }

    fn allowed(&self, a: u32) -> bool {
        self.sv.is_allowed(self.pc, a)
    }

    fn filtered(&self, f: u32) -> bool {
        self.sv.is_filtered(self.pc, f)
    }

    fn html_path(&self) -> Vec<u8> {
        self.pc.servmgr.settings().html_path.clone()
    }
}

fn b(s: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(s)
}

/// `::String` に入れたもの (NUL まで、255 バイトまで)
fn cut(s: &[u8]) -> Vec<u8> {
    pcstr::cut(s)
}

/// `str::truncate_utf8(str::valid_utf8(s), 255)` を `String` に
fn utf8_field(s: &[u8]) -> PcString {
    let v = crate::utf8::valid(s);
    let v = crate::utf8::truncate(&v, 255).unwrap_or(v);
    PcString::with_type(&cut(&v), StrType::Ascii)
}

fn pcs(s: &[u8]) -> PcString {
    PcString::with_type(&cut(s), StrType::Ascii)
}

// ---------------------------------------------------------------- 入り口

/// `handshakeIncoming`
pub fn handshake_incoming(c: &mut Conn) -> Result<()> {
    c.sv.set_status(svt::S_HANDSHAKE);
    if c.pc.servmgr.flags.get("enableSSLServer") {
        let s = c.sock()?;
        let t = s.read_timeout();
        if s.read_ready(t) {
            let ch = s.peek_char()?;
            crate::log_trace!("peekChar -> {}", ch);
            if ch == 22 {
                // TLS のハンドシェイク
                let s = c.take_sock().ok_or_else(|| Error::stream("Not connected"))?;
                let s = s.upgrade_tls()?;
                c.set_sock(s);
            }
        }
    }
    let line = c.sock()?.read_line_buf(8192)?;
    if line.len() >= 8191 {
        return Err(http_error(HTTP_SC_URITOOLONG, 414));
    }
    let is_http = servhs::is_http(&line);
    let host = c.sock()?.host.str();
    if is_http {
        crate::log_trace!("HTTP from {} '{}'", host, b(&line));
    } else {
        crate::log_trace!("Connect from {} '{}'", host, b(&line));
    }
    let cmd_line = {
        let mut h = Http::new(c.sock()?);
        h.init_request(&line);
        h.cmd_line.clone()
    };
    handshake_http(c, &cmd_line, is_http)
}

/// `handshakeHTTP`
fn handshake_http(c: &mut Conn, line: &[u8], is_http: bool) -> Result<()> {
    crate::log_debug!("{} \"{}\"", c.sv.host().ip.str(), b(line));
    let password = c.pc.servmgr.settings().password.clone();
    match servhs::request_kind(line, &password) {
        servhs::RequestKind::Get => handshake_get(c, line),
        servhs::RequestKind::Post => handshake_post(c, line),
        servhs::RequestKind::Giv => handshake_giv(c, line),
        servhs::RequestKind::Pcp => {
            if !c.sv.is_allowed(c.pc, svt::ALLOW_NETWORK) || !c.sv.is_filtered(c.pc, sf::F_NETWORK) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            svt::process_incoming_pcp(c, true)
        }
        servhs::RequestKind::Source => handshake_source(c, line, is_http),
        servhs::RequestKind::Shoutcast => {
            if !c.sv.is_allowed(c.pc, svt::ALLOW_BROADCAST) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            c.sv.st().login_password = pcs(&password);
            {
                let s = c.sock()?;
                s.write_line("OK2")?;
                s.write_line("icy-caps:11")?;
                s.write_line("")?;
            }
            crate::log_debug!("ShoutCast client");
            handshake_icy(c, channel::SRC_SHOUTCAST, is_http)
        }
        servhs::RequestKind::Bad => Err(http_error(HTTP_SC_BADREQUEST, 400)),
    }
}

/// ソケットを使う処理のために、`Ctx` と `Http` を作る
fn with_http<R>(c: &mut Conn, line: &[u8], f: impl FnOnce(&Ctx, &mut Http) -> Result<R>) -> Result<R> {
    let local_host = c.sock()?.local_host().unwrap_or_else(|_| Host::none());
    let ctx = Ctx { pc: c.pc, sv: c.sv, local_host };
    let mut http = Http::new(c.sock()?);
    http.init_request(line);
    f(&ctx, &mut http)
}

// ---------------------------------------------------------------- GET

/// `handshakeGET`
fn handshake_get(c: &mut Conn, line: &[u8]) -> Result<()> {
    use servhs::GetKind as K;
    let full = &line[4.min(line.len())..];
    let (kind, cutpos) = servhs::get_route(full);
    let fn_: Vec<u8> = match cutpos {
        Some(p) if p >= 0 => full[..p as usize].to_vec(),
        _ => full.to_vec(),
    };
    let fn_ = fn_.as_slice();
    let from = |n: usize| fn_.get(n..).unwrap_or(b"").to_vec();

    match kind {
        K::Stream | K::Channel => {
            let (proto, allow, filter, arg, relay_allowed) = if kind == K::Stream {
                (ci::SP_HTTP, svt::ALLOW_DIRECT, sf::F_DIRECT, from(8), true)
            } else {
                (ci::SP_PCP, svt::ALLOW_NETWORK, sf::F_NETWORK, from(9), false)
            };
            if !is_localhost(&c.sv.host()) && (!c.sv.is_allowed(c.pc, allow) || !c.sv.is_filtered(c.pc, filter)) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            let relay = relay_allowed && (c.sv.is_private(c.pc) || has_valid_auth_token(c.pc, &arg));
            return svt::trigger_channel(c, &arg, proto, relay);
        }
        _ => {}
    }

    with_http(c, line, |ctx, http| match kind {
        K::Admin | K::AdminSlash => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            crate::log_debug!("Admin client");
            handshake_cmd(ctx, http, &from(if kind == K::Admin { 7 } else { 8 }))
        }
        K::HtmlIndex => {
            // PeerCastStation は "/" を "/html/index.html" に 301 でリダイレクトするので、キャッシュされた
            // ものを "/" に戻す
            http.read_headers()?;
            http.stream.write_line(HTTP_SC_FOUND)?;
            http.stream.write_line("Location: /")?;
            http.stream.write_line("")
        }
        K::Html => {
            let dir_name = cut(&from(1));
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            if handshake_auth(ctx, http, fn_, false)? {
                handshake_local_file(ctx, http, &dir_name)?;
            }
            Ok(())
        }
        K::AdminCgi => {
            if !ctx.allowed(svt::ALLOW_BROADCAST) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            if let Some(a) = servhs::admin_cgi(fn_) {
                for ch in ctx.pc.chanmgr.channels() {
                    let (status, ct, mount) = {
                        let st = ch.st();
                        (st.status, st.info.content_type.data.clone(), st.mount.data.clone())
                    };
                    if status != channel::S_BROADCASTING || ct != ci::T_MP3 {
                        continue;
                    }
                    let matched = match &a.mount {
                        Some(m) => mount == cut(m),
                        None => true,
                    };
                    if matched {
                        let mut info = ch.info();
                        info.track.title = pcs(&crate::cgi::unescape(&a.song));
                        if let Some(u) = &a.url {
                            if !u.is_empty() {
                                info.track.contact.set(u, StrType::Esc);
                            }
                        }
                        crate::log_info!("Channel Shoutcast update: {}", b(&a.song));
                        ch.update_info(ctx.pc, &info);
                    }
                }
            }
            Ok(())
        }
        K::Pls => {
            if !is_localhost(&ctx.host()) && (!ctx.allowed(svt::ALLOW_DIRECT) || !ctx.filtered(sf::F_DIRECT)) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            let arg = from(5);
            let relay = ctx.sv.is_private(ctx.pc) || has_valid_auth_token(ctx.pc, &arg);
            let (info, found) = ctx.pc.servmgr.get_channel(ctx.pc, &arg, relay);
            http.read_headers()?;
            if found {
                crate::log_debug!("User-Agent: {}", b(&http.headers.get(b"User-Agent")));
                handshake_pls(ctx, http, &info)
            } else {
                Err(http_error(HTTP_SC_NOTFOUND, 404))
            }
        }
        K::Api1 => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            http.read_headers()?;
            let v = super::jrpc_host::invoke(ctx.pc, "getVersionInfo").map_err(|e| Error::general(b(&e).into_owned()))?;
            let response = crate::json::dump(&v).map_err(|e| Error::general(b(&e).into_owned()))?;
            http.stream.write_line(HTTP_SC_OK)?;
            http.stream.write_line(format!("Content-Length: {}", response.len()))?;
            http.stream.write_line("")?;
            http.stream.write(&response)
        }
        K::Public | K::Assets => {
            http.read_headers()?;
            if kind == K::Public && !ctx.pc.servmgr.settings().public_directory_enabled {
                return Err(http_error(HTTP_SC_FORBIDDEN, 403));
            }
            let r = http.get_request().and_then(|req| {
                if kind == K::Public {
                    super::public::public_controller(ctx.pc, &req)
                } else {
                    super::public::assets_controller(ctx.pc, &req)
                }
            });
            match r {
                Ok(res) => http.send_response(res),
                Err(e) if e.is_http() => Err(e),
                Err(e) => {
                    crate::log_error!("Error: {}", e.msg);
                    Err(http_error(HTTP_SC_SERVERERROR, 500))
                }
            }
        }
        K::CgiBinFlv | K::CgiBin => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            if kind == K::CgiBinFlv {
                let authorized = ctx.sv.is_private(ctx.pc) || servhs::flv_valid_auth_token(fn_, &ctx.pc.chanmgr.broadcast_id());
                if !authorized || !ctx.filtered(sf::F_DIRECT) {
                    return Err(http_error(HTTP_SC_FORBIDDEN, 403));
                }
                http.read_headers()?;
                handshake_flv(ctx, http)
            } else if handshake_auth(ctx, http, fn_, false)? {
                handshake_bbs(http)
            } else {
                Ok(())
            }
        }
        K::Cmd => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            let q = Query::new(&from(5)).get(b"q");
            if q.is_empty() {
                return Err(http_error_msg(HTTP_SC_BADREQUEST, 400, "q missing"));
            }
            if handshake_auth(ctx, http, fn_, true)? {
                ctx.sv.st().ty = svt::T_COMMAND;
                http.read_headers()?;
                // HTTP/1.0 で Content-Length なし
                http.write_response_status("HTTP/1.0", 200)?;
                http.write_response_headers(&Headers::from(&[
                    ("Content-Type", b"text/plain; charset=utf-8"),
                    ("Connection", b"close"),
                ]))?;
                let sv = ctx.sv.clone();
                let cancel = move || !sv.thread.active() || !sv.has_sock();
                let mut out = super::commands::Out::new(&mut *http.stream);
                if let Err(e) = super::commands::system(ctx.pc, &mut out, &q, &cancel) {
                    crate::log_error!("Error: cmd '{}': {}", b(&q), e.msg);
                }
            }
            Ok(())
        }
        K::Other | K::Stream | K::Channel => {
            http.read_headers()?;
            let hp = ctx.html_path();
            http.stream.write_line(HTTP_SC_FOUND)?;
            http.stream.write_line([&b"Location: /"[..], &hp, b"/index.html"].concat())?;
            http.stream.write_line("")
        }
    })
}

/// `hasValidAuthToken`
fn has_valid_auth_token(pc: &Peercast, request_filename: &[u8]) -> bool {
    servhs::valid_auth_token(request_filename, &pc.chanmgr.broadcast_id())
}

// ---------------------------------------------------------------- POST

/// `handshakePOST`
fn handshake_post(c: &mut Conn, line: &[u8]) -> Result<()> {
    use servhs::PostKind as K;
    let (kind, args) = servhs::post_route(line).ok_or_else(|| http_error(HTTP_SC_BADREQUEST, 400))?;
    if kind == K::Push {
        if !c.sv.is_allowed(c.pc, svt::ALLOW_BROADCAST) {
            return Err(http_error(HTTP_SC_FORBIDDEN, 403));
        }
        if !c.sv.is_private(c.pc) {
            return Err(http_error(HTTP_SC_FORBIDDEN, 403));
        }
        return handshake_http_push(c, &args);
    }
    with_http(c, line, |ctx, http| match kind {
        K::Api1 => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            if handshake_auth(ctx, http, &args, true)? {
                handshake_jrpc(ctx, http)?;
            }
            Ok(())
        }
        K::Admin => {
            if !ctx.allowed(svt::ALLOW_HTML) {
                return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
            }
            http.read_headers()?;
            let req = http.get_request()?;
            crate::log_debug!("Admin (POST)");
            handshake_cmd(ctx, http, &req.body)
        }
        _ => {
            http.read_headers()?;
            Err(http_error(HTTP_SC_BADREQUEST, 400))
        }
    })
}

/// `handshakeJRPC`
fn handshake_jrpc(ctx: &Ctx, http: &mut Http) -> Result<()> {
    let lenstr = http.headers.get(b"Content-Length");
    let n = servhs::jrpc_body_length(&lenstr, MAX_REQUEST_BODY).map_err(|(s, code)| http_error(s, code))?;
    let body = match http.stream.read_n(n as usize) {
        Ok(b) => b,
        // 本体が短い
        Err(e) if e.is_eof() => return Err(http_error(HTTP_SC_BADREQUEST, 400)),
        Err(e) => return Err(e),
    };
    let body = &body[..body.iter().position(|&c| c == 0).unwrap_or(body.len())];
    let response = super::jrpc_host::call(ctx.pc, body).map_err(|e| Error::general(b(&e).into_owned()))?;
    http.stream.write_line(HTTP_SC_OK)?;
    http.stream.write_line(format!("Server: {}", PCX_AGENT))?;
    http.stream.write_line(format!("Content-Length: {}", response.len()))?;
    http.stream.write_line("Content-Type: application/json")?;
    http.stream.write_line("")?;
    http.stream.write(&response)
}

// ---------------------------------------------------------------- GIV、放送の受け付け

/// `handshakeGIV`
fn handshake_giv(c: &mut Conn, line: &[u8]) -> Result<()> {
    Http::new(c.sock()?).read_headers()?;
    if !c.sv.is_allowed(c.pc, svt::ALLOW_NETWORK) || !c.sv.is_filtered(c.pc, sf::F_NETWORK) {
        return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
    }
    let id = servhs::giv_id(line);
    let idstr = match line.iter().position(|&c| c == b'/') {
        Some(p) => b(&line[p..]).into_owned(),
        None => "(null)".to_string(),
    };
    let ipstr = c.sock()?.host.str();
    if ci::is_set(&id) {
        let ch = c.pc.chanmgr.find_channel_by_id(&id).ok_or_else(|| http_error(HTTP_SC_NOTFOUND, 404))?;
        let sock = c.take_sock().ok_or_else(|| Error::stream("Not connected"))?;
        if let Err(sock) = ch.accept_giv(sock) {
            c.set_sock(sock);
            return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
        }
        crate::log_debug!("Accepted GIV channel {} from: {}", idstr, ipstr);
    } else {
        let sock = c.take_sock().ok_or_else(|| Error::stream("Not connected"))?;
        if let Err(sock) = c.pc.servmgr.accept_giv(sock) {
            c.set_sock(sock);
            return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
        }
        crate::log_debug!("Accepted GIV PCP from: {}", ipstr);
    }
    Ok(())
}

/// `handshakeSOURCE`: Icecast の放送
fn handshake_source(c: &mut Conn, line: &[u8], is_http: bool) -> Result<()> {
    if !c.sv.is_allowed(c.pc, svt::ALLOW_BROADCAST) {
        return Err(http_error(HTTP_SC_UNAVAILABLE, 503));
    }
    let src = servhs::source(line);
    match &src.password {
        None => crate::log_debug!("ICE 1.0 client to {}", b(&src.mount)),
        Some(p) => {
            let pw = pcs(p);
            crate::log_debug!("ICY client: {} {}", b(&pw.data), b(&src.mount));
            c.sv.st().login_password = pw;
        }
    }
    c.sv.st().login_mount = pcs(&src.mount);
    handshake_icy(c, channel::SRC_ICECAST, is_http)
}

/// `setBroadcastIdChannelId`: 配信のチャンネル ID を決める (乱数か、名前などから)
pub fn set_broadcast_id_channel_id(pc: &Peercast, info: &mut ChanInfo, broadcast_id: &[u8; 16]) {
    info.bc_id = *broadcast_id;
    if pc.servmgr.flags.get("randomizeBroadcastingChannelID") {
        info.id = super::chanmgr::random_id();
    } else {
        let mut id = *broadcast_id;
        crate::gnuid::encode(&mut id, None, &info.name.data, &info.genre.data, info.bitrate as u8);
        info.id = id;
    }
}

/// `createChannelInfo`: HTTP Push の引数から
fn create_channel_info(pc: &Peercast, broadcast_id: &[u8; 16], broadcast_msg: &PcString, q: &Query, content_type: &[u8]) -> ChanInfo {
    let mut info = ChanInfo::new();
    let mut ty = ci::type_from_str(&cut(&q.get(b"type")));
    if ty == ci::T_UNKNOWN {
        ty = crate::chaninfo::type_from_mime(content_type);
    }
    info.set_content_type(ty);
    info.name = pcs(&q.get(b"name"));
    info.genre = pcs(&q.get(b"genre"));
    info.desc = pcs(&q.get(b"desc"));
    info.url = pcs(&q.get(b"url"));
    info.bitrate = crate::http::atoi(&q.get(b"bitrate"));
    let comment = q.get(b"comment");
    info.comment = if comment.is_empty() { broadcast_msg.clone() } else { pcs(&comment) };
    set_broadcast_id_channel_id(pc, &mut info, broadcast_id);
    info
}

/// `handshakeHTTPPush`: HTTP Push の放送
fn handshake_http_push(c: &mut Conn, args: &[u8]) -> Result<()> {
    let pc = c.pc;
    let q = Query::new(args);
    let headers = {
        let mut http = Http::new(c.sock()?);
        http.read_headers()?;
        http.headers.clone()
    };
    if q.get(b"name").is_empty() {
        crate::log_error!("handshakeHTTPPush: name parameter is mandatory");
        return Err(http_error(HTTP_SC_BADREQUEST, 400));
    }
    let (bid, msg) = {
        let s = pc.chanmgr.settings();
        (s.broadcast_id, s.broadcast_msg.clone())
    };
    let info = create_channel_info(pc, &bid, &msg, &q, &headers.get(b"Content-Type"));
    if let Some(old) = pc.chanmgr.find_channel_by_id(&info.id) {
        crate::log_info!("HTTP Push channel already active, closing old one");
        old.thread.shutdown();
    }
    let ch = pc.chanmgr.create_channel(pc, &info, None);
    let chunked = headers.get(b"Transfer-Encoding") == b"chunked";
    if q.get(b"ipv") == b"6" {
        ch.st().ip_version = channel::IP_V6;
        crate::log_info!("Channel IP version set to 6");
        pc.servmgr.check_firewall_ipv6();
    }
    let sock = c.take_sock().ok_or_else(|| Error::stream("Not connected"))?;
    ch.start_http_push(pc, sock, chunked);
    Ok(())
}

/// `handshakeICY`: ShoutCast と Icecast の放送
fn handshake_icy(c: &mut Conn, src_type: i32, is_http: bool) -> Result<()> {
    let pc = c.pc;
    let mut info = ChanInfo::new();
    if src_type == channel::SRC_SHOUTCAST {
        // ShoutCast の DSP は content-type を送らない
        info.content_type.assign(ci::T_MP3);
    }
    let mut pwd = c.sv.st().login_password.clone();
    {
        let mut http = Http::new(c.sock()?);
        while http.next_header()? {
            crate::log_debug!("ICY {}", b(&http.cmd_line));
            svt::read_icy_header(&http, &mut info, Some(&mut pwd));
        }
    }
    c.sv.st().login_password = pwd.clone();
    let password = pc.servmgr.settings().password.clone();
    if pwd.data != password && (!is_localhost(&c.sv.host()) || !pwd.is_empty()) {
        return Err(http_error(HTTP_SC_UNAUTHORIZED, 401));
    }
    // 始める前に正しい IP アドレスが要る
    pc.servmgr.check_firewall(pc)?;
    let mount = c.sv.st().login_mount.clone();
    if pc.servmgr.flags.get("randomizeBroadcastingChannelID") {
        info.id = super::chanmgr::random_id();
    } else {
        let mut id = pc.chanmgr.broadcast_id();
        crate::gnuid::encode(&mut id, None, &info.name.data, &mount.data, info.bitrate as u8);
        info.id = id;
    }
    crate::log_debug!("Incoming source: {} : {}", b(&info.name.data), b(&info.content_type.data));
    {
        let s = c.sock()?;
        if is_http {
            s.write_string(format!("{}\n\n", HTTP_SC_OK))?;
        } else {
            s.write_line("OK")?;
        }
    }
    if let Some(old) = pc.chanmgr.find_channel_by_id(&info.id) {
        crate::log_info!("ICY channel already active, closing old one");
        old.thread.shutdown();
    }
    {
        let s = pc.chanmgr.settings();
        info.comment = s.broadcast_msg.clone();
        info.bc_id = s.broadcast_id;
    }
    let ch = pc.chanmgr.create_channel(pc, &info, Some(&mount.data));
    let sock = c.take_sock().ok_or_else(|| Error::stream("Not connected"))?;
    ch.start_icy(pc, sock, src_type);
    Ok(())
}

// ---------------------------------------------------------------- プレイリスト

/// `handshakePLS`
fn handshake_pls(ctx: &Ctx, http: &mut Http, info: &ChanInfo) -> Result<()> {
    let url = local_url(ctx, &http.headers.get(b"Host"));
    let ty = PlayList::type_for(&info.content_type.data);
    let s = &mut *http.stream;
    s.write_line(HTTP_SC_OK)?;
    s.write_line(format!("Server: {}", PCX_AGENT))?;
    let content = match ty {
        playlist::T_PLS => "audio/x-mpegurl",
        playlist::T_RAM => "audio/x-pn-realaudio",
        _ => MIME_TEXT,
    };
    s.write_line(format!("Content-Type: {}", content))?;
    s.write_line("Content-Disposition: inline")?;
    s.write_line("Cache-Control: private")?;
    s.write_line("Connection: close")?;
    s.write_line("")?;
    let mut pls = PlayList::new(ty, 1);
    pls.add_channel(ctx.pc, &url, info);
    pls.write(s)
}

/// `getLocalURL`
fn local_url(ctx: &Ctx, host_header: &[u8]) -> Vec<u8> {
    if host_header.is_empty() {
        let server_host = ctx.pc.servmgr.settings().server_host;
        let mut h = if ctx.host().local_ip() { ctx.local_host } else { server_host };
        h.port = server_host.port;
        [&b"http://"[..], h.str().as_bytes()].concat()
    } else {
        [&b"http://"[..], host_header].concat()
    }
}

// ---------------------------------------------------------------- 認証

/// `handshakeAuth`: 認証できれば true。できなければ応答を書いて false
fn handshake_auth(ctx: &Ctx, http: &mut Http, args: &[u8], reject_cross_origin: bool) -> Result<bool> {
    http.read_headers()?;
    let hd = |n: &[u8]| http.headers.get(n);
    // 状態を変える API は、ほかのサイトのページからの要求 (CSRF) を受け付けない
    if reject_cross_origin && crate::http::is_cross_origin_request(&hd(b"Sec-Fetch-Site"), &hd(b"Origin"), &hd(b"Host")) {
        crate::log_warn!("Rejected cross-origin request");
        return Err(http_error(HTTP_SC_FORBIDDEN, 403));
    }
    if is_localhost(&ctx.host()) {
        // Host が localhost や IP アドレスでなければ DNS リバインディングかもしれないので、認証を省かない
        if crate::http::is_loopback_host_header(&hd(b"Host")) {
            return Ok(true);
        }
        crate::log_warn!("Host header is not a loopback name; not trusting localhost: {}", b(&hd(b"Host")));
    }
    let (password, auth_type, port) = {
        let s = ctx.pc.servmgr.settings();
        (s.password.clone(), s.auth_type, s.server_host.port)
    };
    if !password.is_empty() && Query::new(args).get(b"pass") == password {
        return Ok(true);
    }
    if auth_type == AUTH_HTTPBASIC {
        let a = hd(b"Authorization");
        if !a.is_empty() {
            let (_, pass) = super::http::parse_authorization_header(&a);
            if !password.is_empty() && pass == password {
                return Ok(true);
            }
        }
    } else if auth_type == AUTH_COOKIE {
        let arg = hd(b"Cookie");
        if !arg.is_empty() {
            crate::log_trace!("Got cookie: {}", b(&arg));
            match servhs::cookie_id(&arg, port) {
                servhs::CookieParse::Invalid => crate::log_error!("Invalid Cookie header: expected '='"),
                servhs::CookieParse::Found(id) => ctx.sv.st().cookie = Cookie::new(&id, ctx.host().ip),
                servhs::CookieParse::NotFound => {}
            }
            let cookie = ctx.sv.st().cookie.clone();
            if ctx.pc.servmgr.cookies().contains(&cookie) {
                crate::log_trace!("Cookie ID found");
                return Ok(true);
            }
        }
    }
    // 認証できなかった
    if auth_type == AUTH_HTTPBASIC {
        http.stream.write_line(HTTP_SC_UNAUTHORIZED)?;
        http.stream.write_line("WWW-Authenticate: Basic realm=\"PeerCast Admin\"")?;
        http.stream.write_line("")?;
    } else if auth_type == AUTH_COOKIE {
        let file = [&ctx.html_path()[..], b"/login.html"].concat();
        if hd(b"X-Requested-With") == b"XMLHttpRequest" {
            return Err(http_error(HTTP_SC_FORBIDDEN, 403));
        }
        handshake_local_file(ctx, http, &cut(&file))?;
    }
    Ok(false)
}

// ---------------------------------------------------------------- ローカルのファイル

/// `validFileOrThrow`: 文書のディレクトリの下のファイルか
fn valid_file(file_path: &[u8], document_root: &[u8]) -> Result<()> {
    let abs = match sys::real_path(file_path) {
        Some(p) => p,
        None => {
            crate::log_error!("Cannot determine absolute path: realPath: {}", b(file_path));
            return Err(http_error(HTTP_SC_NOTFOUND, 404));
        }
    };
    if !abs.starts_with(document_root) {
        crate::log_error!("Requested file is outside of the document root: {}", b(&abs));
        // ファイルがあることを知らせないために 404
        return Err(http_error(HTTP_SC_NOTFOUND, 404));
    }
    Ok(())
}

/// `handshakeLocalFile`: html などのファイル (テンプレートを評価する)
fn handshake_local_file(ctx: &Ctx, http: &mut Http, fn_: &[u8]) -> Result<()> {
    let pc = ctx.pc;
    let document_root = match sys::real_path(&pc.app.html_path) {
        Some(p) => [&p[..], b"/"].concat(),
        None => {
            crate::log_error!("documentRoot: realPath ({})", b(&pc.app.html_path));
            return Err(http_error(HTTP_SC_SERVERERROR, 500));
        }
    };
    let mut file_name = servhs::local_file_name(&document_root, fn_);
    crate::log_trace!("Writing HTML file: {}", b(&file_name));
    let mime = servhs::mime_type_for(&file_name).ok_or_else(|| http_error(HTTP_SC_NOTFOUND, 404))?;
    let mut out = StringStream::new();
    if mime == b"text/html" {
        let req = http.get_request()?;
        let mut locals: BTreeMap<Vec<u8>, super::state::Value> = BTreeMap::new();
        let lf = servhs::local_file(fn_);
        let chan_state = |id: &[u8]| pc.chanmgr.find_channel_by_id(&crate::gnuid::from_str(id)).map(|c| c.state(pc));
        match lf.page {
            servhs::LocalPage::Play => {
                // 視聴ページなら、チャンネルのリレーを始めておく
                if !lf.split_ok || lf.id.is_empty() {
                    return Err(http_error(HTTP_SC_BADREQUEST, 400));
                }
                let (_, found) = pc.servmgr.get_channel(pc, &lf.id, true);
                if !found {
                    return Err(http_error(HTTP_SC_NOTFOUND, 404));
                }
                let st = chan_state(&lf.id).ok_or_else(|| http_error(HTTP_SC_NOTFOUND, 404))?;
                locals.insert(b"channel".to_vec(), st);
            }
            servhs::LocalPage::RelayInfo => {
                if !lf.split_ok || lf.id.is_empty() {
                    return Err(http_error(HTTP_SC_BADREQUEST, 400));
                }
                locals.insert(b"channel".to_vec(), chan_state(&lf.id).unwrap_or(super::state::Value::Null));
            }
            servhs::LocalPage::Connections => {
                if lf.split_ok && !lf.id.is_empty() {
                    locals.insert(b"channel".to_vec(), chan_state(&lf.id).unwrap_or(super::state::Value::Null));
                }
            }
            servhs::LocalPage::Plain => {}
        }
        if let Some(p) = file_name.iter().position(|&c| c == b'?') {
            file_name.truncate(p);
        }
        valid_file(&file_name, &document_root)?;
        html::write_ok(&mut out, "text/html; charset=utf-8", &[])?;
        let scopes = vec![
            Scope::Request { host: req.headers.get(b"Host"), path: req.path.clone(), query: req.query_string.clone() },
            Scope::Generic(locals),
        ];
        html::write_template(pc, &mut out, &file_name, &req.query_string, scopes)?;
    } else {
        valid_file(&file_name, &document_root)?;
        html::write_raw_file(&mut out, &file_name, &b(mime))?;
    }
    http.stream.write(out.str())
}

// ---------------------------------------------------------------- /cgi-bin (もとは CGI スクリプト)

/// 掲示板ビューワー (`/cgi-bin/board.cgi`、`thread.cgi`、`post.cgi`)。ほかの `/cgi-bin/` は 404
fn handshake_bbs(http: &mut Http) -> Result<()> {
    let req = http.get_request().map_err(|_| http_error(HTTP_SC_BADREQUEST, 400))?;
    let script = match req.path.strip_prefix(&b"/cgi-bin/"[..]) {
        Some(s @ (b"board.cgi" | b"thread.cgi" | b"post.cgi")) => String::from_utf8_lossy(s).into_owned(),
        _ => return Err(http_error(HTTP_SC_NOTFOUND, 404)),
    };
    match crate::bbs::handle(&script, &req.query_string, &mut super::bbs_http::Fetcher) {
        Ok(reply) => {
            let mut res = Response::new(reply.status as i32, Headers::from(&[("Content-Type", reply.content_type.as_bytes())]));
            res.body = reply.body;
            http.send_response(res)
        }
        Err(e) => {
            crate::log_error!("{}: {}", script, e.0);
            Err(http_error(HTTP_SC_SERVERERROR, 500))
        }
    }
}

/// `/cgi-bin/flv.cgi`: チャンネルのストリームを ffmpeg で FLV (H.264) にして送る。接続が切れたら
/// ffmpeg を止める
fn handshake_flv(ctx: &Ctx, http: &mut Http) -> Result<()> {
    let req = http.get_request().map_err(|_| http_error(HTTP_SC_BADREQUEST, 400))?;
    if req.path != b"/cgi-bin/flv.cgi" {
        return Err(http_error(HTTP_SC_NOTFOUND, 404));
    }
    let port = ctx.pc.servmgr.settings().server_host.port;
    let args = match servhs::flv_ffmpeg_args(&req.query_string, port) {
        Some(a) => a,
        None => return Err(http_error(HTTP_SC_BADREQUEST, 400)),
    };
    let mut child = match std::process::Command::new("ffmpeg")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            crate::log_error!("failed to start ffmpeg: {}", e);
            return Err(http_error(HTTP_SC_SERVERERROR, 500));
        }
    };
    crate::log_debug!("ffmpeg started (pid = {})", child.id());
    let r = (|| -> Result<()> {
        let s = &mut *http.stream;
        s.write_line("HTTP/1.0 200 OK")?;
        s.write_line(format!("Server: {}", PCX_AGENT))?;
        s.write_line("Connection: close")?;
        s.write_line("Content-Type: video/x-flv")?;
        s.write_line("")?;
        let mut out = match child.stdout.take() {
            Some(o) => o,
            None => return Ok(()),
        };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match std::io::Read::read(&mut out, &mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => s.write(&buf[..n])?,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Ok(()),
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    crate::log_debug!("ffmpeg finished");
    r
}

// ---------------------------------------------------------------- 管理の要求 (/admin?cmd=...)

/// `handshakeCMD`
fn handshake_cmd(ctx: &Ctx, http: &mut Http, query: &[u8]) -> Result<()> {
    if !handshake_auth(ctx, http, query, true)? {
        return Ok(());
    }
    let cmd = Query::new(query).get(b"cmd");
    let mut jump: Vec<u8> = Vec::new();
    let r = run_cmd(ctx, http, &cmd, query, &mut jump);
    match r {
        Ok(()) => {}
        Err(e) if e.is_http() => {
            let s = &mut *http.stream;
            s.write_line(&e.msg)?;
            s.write_line(format!("Server: {}", PCX_AGENT))?;
            s.write_line("Content-Type: text/html; charset=utf-8")?;
            s.write_line("")?;
            s.write_string([&b"<h1>ERROR - "[..], &crate::cgi::escape_html(e.msg.as_bytes()), b"</h1>\n"].concat())?;
            if !e.detail.is_empty() {
                s.write_string([&b"<pre>"[..], &crate::cgi::escape_html(e.detail.as_bytes()), b"</pre>\n"].concat())?;
            }
            crate::log_error!("html: {}", e.msg);
        }
        Err(e) => return Err(e),
    }
    if !jump.is_empty() {
        http.stream.write_line(HTTP_SC_FOUND)?;
        http.stream.write_line([&b"Location: "[..], &jump].concat())?;
        http.stream.write_line("")?;
    }
    Ok(())
}

/// `jumpStr.sprintf(...)` (`::String` なので 255 バイトまで)
fn set_jump(jump: &mut Vec<u8>, s: impl AsRef<[u8]>) {
    *jump = cut(s.as_ref());
}

/// Referer があればそこへ、なければ `/{htmlPath}/{page}` へ
fn jump_back(ctx: &Ctx, http: &Http, jump: &mut Vec<u8>, page: &str) {
    let referer = http.headers.get(b"Referer");
    if !referer.is_empty() {
        set_jump(jump, referer);
    } else {
        set_jump(jump, [&b"/"[..], &ctx.html_path(), b"/", page.as_bytes()].concat());
    }
}

fn redirect_back(http: &mut Http, default: &[u8]) -> Result<()> {
    let referer = http.headers.get(b"Referer");
    http.send_response(Response::redirect_to(if referer.is_empty() { default } else { &referer }))
}

fn run_cmd(ctx: &Ctx, http: &mut Http, cmd: &[u8], query: &[u8], jump: &mut Vec<u8>) -> Result<()> {
    let pc = ctx.pc;
    let q = Query::new(query);
    let hp = ctx.html_path();
    let page = |p: &str| [&b"/"[..], &hp, b"/", p.as_bytes()].concat();
    match cmd {
        b"add_speedtest" => {
            let url = q.get(b"url");
            if url.is_empty() {
                return http.send_response(Response::bad_request(b"empty url"));
            }
            match pc.servmgr.uptest.add_url(&url) {
                Err(m) => http.send_response(Response::server_error(&m)),
                Ok(()) => redirect_back(http, b"/speedtest.html"),
            }
        }
        b"apply" => cmd_apply(ctx, http, query, jump),
        b"applyflags" => {
            for f in pc.servmgr.flags.sorted() {
                f.set(!q.get(f.name.as_bytes()).is_empty());
            }
            pc.save_settings();
            pc.notify_message(super::notif::NT_PEERCAST, "フラグ設定を保存しました。".as_bytes());
            set_jump(jump, page("flags.html"));
            Ok(())
        }
        b"bump" => {
            let idq = q.get(b"id");
            let id = if idq.is_empty() { [0; 16] } else { crate::gnuid::from_str(&idq) };
            let ip = q.get(b"ip");
            let designation = Host::from_str_ip(&ip, 7144);
            if let Some(ch) = pc.chanmgr.find_channel_by_id(&id) {
                if !ip.is_empty() {
                    let info = ch.info();
                    let the_hit = pc
                        .chanmgr
                        .with_hitlist(&info, |l| l.hits.iter().filter(|h| h.host == designation).last().cloned())
                        .flatten();
                    let mut st = ch.st();
                    match the_hit {
                        Some(h) if h.host.ip.is_set() => st.designated_host = h,
                        _ => {
                            // ヒットをでっちあげる
                            let mut h = ChanHit::new();
                            h.host = designation;
                            h.rhost[0] = designation;
                            st.designated_host = h;
                        }
                    }
                }
                ch.st().bump = true;
            }
            jump_back(ctx, http, jump, "channels.html");
            Ok(())
        }
        b"chanfeedlog" => {
            let index = crate::http::atoi(&q.get(b"index"));
            let feeds = pc.servmgr.channel_directory.feeds();
            let h = || Headers::from(&[("Content-Type", b"text/plain; charset=UTF-8")]);
            let res = match usize::try_from(index).ok().and_then(|i| feeds.get(i)) {
                Some(f) => {
                    let mut r = Response::new(200, h());
                    r.body = f.log.clone();
                    r
                }
                None => {
                    let mut r = Response::new(404, h());
                    r.body = format!(
                        "vector::_M_range_check: __n (which is {}) >= this->size() (which is {})",
                        index as i64 as u64,
                        feeds.len()
                    )
                    .into_bytes();
                    r
                }
            };
            http.send_response(res)
        }
        b"clear" => {
            for (curr, _) in servhs::cgi_args(query) {
                match curr.as_slice() {
                    b"hostcache" => pc.servmgr.clear_host_cache(super::servmgr::SH_SERVENT),
                    b"hitlists" => pc.chanmgr.clear_hit_lists(),
                    b"packets" => super::stats::clear_range(super::stats::Stat::PacketsStart, super::stats::Stat::PacketSend),
                    b"channels" => pc.chanmgr.close_idles(),
                    _ => {}
                }
            }
            jump_back(ctx, http, jump, "index.html");
            Ok(())
        }
        b"clearlog" => {
            super::log::with_buffer(|b| b.clear());
            set_jump(jump, page("viewlog.html"));
            Ok(())
        }
        b"control_rtmp" => {
            let action = q.get(b"action");
            if action == b"start" {
                if q.get(b"name").is_empty() {
                    return Err(http_error(HTTP_SC_BADREQUEST, 400));
                }
                let port = crate::http::atoi(&q.get(b"port")) as u16;
                {
                    let mut s = pc.servmgr.settings();
                    s.rtmp_port = port;
                    let info = &mut s.default_channel_info;
                    info.name = utf8_field(&q.get(b"name"));
                    info.genre = utf8_field(&q.get(b"genre"));
                    info.desc = utf8_field(&q.get(b"desc"));
                    info.url = utf8_field(&q.get(b"url"));
                    info.comment = utf8_field(&q.get(b"comment"));
                }
                pc.servmgr.rtmp_monitor.set_ip_version(if q.get(b"ipv") == b"6" { 6 } else { 4 });
                pc.servmgr.rtmp_monitor.enable();
                // サーバーのスレッドがプロセスを始めるのを待つ
                sys::sleep(500);
                set_jump(jump, page("rtmp.html"));
                Ok(())
            } else if action == b"stop" {
                pc.servmgr.rtmp_monitor.disable();
                set_jump(jump, page("rtmp.html"));
                Ok(())
            } else {
                Err(http_error(HTTP_SC_BADREQUEST, 400))
            }
        }
        b"delete_speedtest" | b"take_speedtest" | b"speedtest_cached_xml" => {
            let is = q.get(b"index");
            if !servhs::is_decimal(&is) {
                return http.send_response(Response::bad_request(b"invalid index"));
            }
            // std::stoi は int に収まらなければ例外を投げる
            let index: i32 = String::from_utf8_lossy(&is).parse().map_err(|_| Error::general("stoi"))?;
            let reg = &pc.servmgr.uptest;
            match cmd {
                b"delete_speedtest" => match reg.delete_by_index(index) {
                    Err(m) => http.send_response(Response::server_error(&m)),
                    Ok(()) => redirect_back(http, b"/speedtest.html"),
                },
                b"take_speedtest" => match reg.take_speedtest(index) {
                    Err(m) => http.send_response(Response::server_error(&m)),
                    Ok(()) => {
                        // 新しい計測値を読み込む
                        reg.force_update();
                        redirect_back(http, b"/speedtest.html")
                    }
                },
                _ => match reg.xml(index) {
                    Ok(xml) => {
                        let len = xml.len().to_string();
                        http.send_response(Response::ok(
                            Headers::from(&[("Content-Type", b"application/xml"), ("Content-Length", len.as_bytes())]),
                            xml,
                        ))
                    }
                    Err(m) => http.send_response(Response::server_error(&m)),
                },
            }
        }
        b"dump_hitlists" => {
            let buf = dump_hitlists(pc);
            let s = &mut *http.stream;
            s.write_line(HTTP_SC_OK)?;
            s.write_line(format!("Server: {}", PCX_AGENT))?;
            s.write_line(format!("Content-Length: {}", buf.len()))?;
            s.write_line("Content-Type: text/plain;charset=utf-8")?;
            s.write_line("")?;
            s.write_string(&buf)
        }
        b"fetch" => {
            let mut info = ChanInfo::new();
            let curl = q.get(b"url");
            info.name = utf8_field(&q.get(b"name"));
            info.desc = utf8_field(&q.get(b"desc"));
            info.genre = utf8_field(&q.get(b"genre"));
            info.url = utf8_field(&q.get(b"contact"));
            info.bitrate = crate::http::atoi(&q.get(b"bitrate"));
            info.set_content_type(&cut(&q.get(b"type")));
            // 配信元に接続できなかったときもチャンネルが分かるように、先に ID を決める
            set_broadcast_id_channel_id(pc, &mut info, &pc.chanmgr.broadcast_id());
            let ch = pc.chanmgr.create_channel(pc, &info, None);
            if q.get(b"ipv") == b"6" {
                ch.st().ip_version = channel::IP_V6;
                crate::log_info!("Channel IP version set to 6");
                pc.servmgr.check_firewall_ipv6();
            }
            ch.start_url(pc, &curl);
            set_jump(jump, page("relays.html"));
            Ok(())
        }
        b"fetch_feeds" => {
            let port = pc.servmgr.settings().server_host.port;
            pc.servmgr.channel_directory.update(port, super::directory::UpdateMode::Manual);
            // Referer に戻すと、admin?cmd=fetch_feeds でログインしたときに同じ URL に戻り続ける
            set_jump(jump, page("channels.html"));
            Ok(())
        }
        b"keep" => {
            let id = cgi_id(query);
            if let Some(ch) = pc.chanmgr.find_channel_by_id(&id) {
                let mut st = ch.st();
                st.stay_connected = !st.stay_connected;
            }
            jump_back(ctx, http, jump, "relays.html");
            Ok(())
        }
        b"login" => {
            let id = super::chanmgr::generate_id(0);
            let idstr = ci::id_str(&id);
            let cookie = Cookie::new(idstr.as_bytes(), ctx.host().ip);
            ctx.sv.st().cookie = cookie.clone();
            let (never, port) = {
                let mut cl = pc.servmgr.cookies();
                cl.add(cookie);
                (cl.never_expire, pc.servmgr.settings().server_host.port)
            };
            let s = &mut *http.stream;
            s.write_line(HTTP_SC_FOUND)?;
            if never {
                s.write_line(format!(
                    "Set-Cookie: {}_id={}; path=/; expires=\"Mon, 01-Jan-3000 00:00:00 GMT\"; SameSite=Strict",
                    port, idstr
                ))?;
            } else {
                s.write_line(format!("Set-Cookie: {}_id={}; path=/; SameSite=Strict", port, idstr))?;
            }
            let rp = q.get(b"requested_path");
            if crate::cgi::is_safe_local_path(&rp) {
                s.write_line([&b"Location: "[..], &rp].concat())?;
            } else {
                s.write_line([&b"Location: /"[..], &hp, b"/index.html"].concat())?;
            }
            s.write_line("")
        }
        b"logout" => {
            set_jump(jump, "/");
            let cookie = ctx.sv.st().cookie.clone();
            pc.servmgr.cookies().remove(&cookie);
            Ok(())
        }
        b"portcheck4" | b"portcheck6" => {
            let v = if cmd == b"portcheck4" { 4 } else { 6 };
            let mut out = StringStream::new();
            {
                let mut o = super::commands::Out::new(&mut out);
                o.with_log(|o| {
                    pc.servmgr.set_firewall(v, super::servmgr::FW_UNKNOWN);
                    let r = if v == 4 {
                        pc.servmgr.check_firewall(pc)
                    } else {
                        pc.servmgr.check_firewall_ipv6();
                        Ok(())
                    };
                    match r {
                        Ok(()) => o.line(format!(
                            "IPv{} firewall is {}",
                            v,
                            super::servmgr::firewall_state_str(pc.servmgr.get_firewall(v))
                        )),
                        Err(e) => o.line(format!("Error: {}", e.msg)),
                    }
                })?;
            }
            http.send_response(Response::ok(Headers::from(&[("Content-Type", b"text/plain; charset=UTF-8")]), out.into_inner()))
        }
        b"redirect" => {
            let buf = &query[..query.len().min(servhs::MAX_CGI_LEN - 1)];
            match servhs::redirect_url(buf) {
                Some(url) => {
                    let s = &mut *http.stream;
                    s.write_line(HTTP_SC_OK)?;
                    s.write_line(format!("Server: {}", PCX_AGENT))?;
                    s.write_line("Content-Type: text/html")?;
                    s.write_line("")?;
                    s.write(&html::refresh_page(&url))
                }
                None => Err(http_error(HTTP_SC_BADREQUEST, 400)),
            }
        }
        b"refresh_speedtest" => {
            pc.servmgr.uptest.force_update();
            redirect_back(http, b"/speedtest.html")
        }
        b"shutdown" => {
            http.send_response(Response::ok(Headers::new(), b"Server is shutting down...".to_vec()))?;
            pc.servmgr.shutdown_timer.store(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        b"stop" => {
            let id = cgi_id(query);
            if let Some(ch) = pc.chanmgr.find_channel_by_id(&id) {
                ch.thread.shutdown();
                ch.wait_thread();
            }
            jump_back(ctx, http, jump, "relays.html");
            Ok(())
        }
        b"stop_servent" => {
            let idx = q.get(b"servent_id");
            if !servhs::is_decimal(&idx) {
                return http.send_response(Response::bad_request(b"invalid servent_id"));
            }
            let found = String::from_utf8_lossy(&idx).parse::<i32>().ok().and_then(|i| pc.servmgr.find_servent_by_id(i));
            match found {
                Some(s) => {
                    s.abort();
                    jump_back(ctx, http, jump, "connections.html");
                    Ok(())
                }
                None => http.send_response(Response::not_found(b"servent not found")),
            }
        }
        b"update_channel_info" => {
            let id = crate::gnuid::from_str(&q.get(b"id"));
            let ch = pc
                .chanmgr
                .find_channel_by_id(&id)
                .ok_or_else(|| http_error_msg(HTTP_SC_SERVERERROR, 500, "No such channel"))?;
            let mut info = ch.info();
            info.name = utf8_field(&q.get(b"name"));
            info.desc = utf8_field(&q.get(b"desc"));
            info.genre = utf8_field(&q.get(b"genre"));
            info.url = utf8_field(&q.get(b"contactURL"));
            info.comment = utf8_field(&q.get(b"comment"));
            info.track.contact = utf8_field(&q.get(b"track.contactURL"));
            info.track.title = utf8_field(&q.get(b"track.title"));
            info.track.artist = utf8_field(&q.get(b"track.artist"));
            info.track.album = utf8_field(&q.get(b"track.album"));
            info.track.genre = utf8_field(&q.get(b"track.genre"));
            if !ch.update_info(pc, &info) {
                return Err(http_error_msg(HTTP_SC_SERVERERROR, 500, "Failed to update channel info"));
            }
            set_jump(jump, page("relays.html"));
            Ok(())
        }
        b"viewxml" => handshake_xml(ctx, http),
        b"customizeAppearance" => {
            for key in q.keys() {
                match key.as_slice() {
                    b"cmd" => {}
                    b"preferredTheme" => pc.servmgr.settings().preferred_theme = q.get(key),
                    b"accentColor" => pc.servmgr.settings().accent_color = q.get(key),
                    _ => crate::log_warn!("Unexpected key `{}`", b(key)),
                }
            }
            redirect_back(http, b"/")
        }
        b"chooseLanguage" => {
            let mut referer = http.headers.get(b"Referer");
            crate::log_debug!("old referer: {}", b(&referer));
            for key in q.keys() {
                match key.as_slice() {
                    b"cmd" => {}
                    b"htmlPath" => {
                        let new_path = [&b"html/"[..], &q.get(key)].concat();
                        if !servhs::is_valid_html_path(&new_path) {
                            crate::log_warn!("CMD_chooseLanguage: invalid htmlPath");
                            return http.send_response(Response::bad_request(b"Bad request"));
                        }
                        match servhs::rewrite_referer(&referer, &new_path) {
                            Some(r) => {
                                referer = r;
                                crate::log_debug!("new referer: {}", b(&referer));
                            }
                            None => crate::log_warn!("CMD_chooseLanguage: Failed to rewrite referer: {}", b(&referer)),
                        }
                        pc.servmgr.settings().html_path = new_path[..new_path.len().min(127)].to_vec();
                        return http.send_response(Response::redirect_to(if referer.is_empty() { b"/" } else { &referer }));
                    }
                    _ => crate::log_warn!("Unexpected key `{}`", b(key)),
                }
            }
            // htmlPath がない
            http.send_response(Response::bad_request(b"Bad request"))
        }
        _ => Err(http_error(HTTP_SC_BADREQUEST, 400)),
    }
}

/// `nextCGIarg` で読んだ `id` (最後のもの)
fn cgi_id(query: &[u8]) -> [u8; 16] {
    let mut id = [0u8; 16];
    for (k, v) in servhs::cgi_args(query) {
        if k == b"id" {
            id = crate::gnuid::from_str(&v);
        }
    }
    id
}

/// `CMD_apply`: 設定の保存
fn cmd_apply(ctx: &Ctx, http: &mut Http, query: &[u8], jump: &mut Vec<u8>) -> Result<()> {
    use servhs::ApplyKey as K;
    let pc = ctx.pc;
    let sm = &pc.servmgr;
    let mut br_root = false;
    let mut get_upd = false;
    let mut allow_server1 = 0u32;
    let ops = servhs::apply_ops(query);
    sm.channel_directory.clear_feeds();
    let mut new_port = {
        let mut s = sm.settings();
        s.public_directory_enabled = false;
        s.transcoding_enabled = false;
        s.chat = false;
        s.server_host.port as i32
    };
    if let Some(f) = sm.flags.find(b"randomizeBroadcastingChannelID") {
        f.set(false);
    }
    // filters[0..numFilters] と作業用の 1 つ。numFilters = 0 から始める (作業用は前の filters[0])
    let mut filters: Vec<sf::ServFilter> = vec![sm.settings().filters.first().cloned().unwrap_or_default()];
    let mut cur = 0usize;
    for op in &ops {
        let v = op.int;
        let st = || String::from_utf8_lossy(&op.str).into_owned();
        match op.key {
            K::ServerName => sm.settings().server_name = pcs(&op.str),
            K::ServerActive => sm.settings().auto_serve = v != 0,
            K::Port => new_port = v,
            K::IcyMeta => pc.chanmgr.settings().icy_meta_interval = v,
            K::PassNew => sm.settings().password = op.str[..op.str.len().min(63)].to_vec(),
            K::Root => sm.set_root(v != 0),
            K::BrRoot => br_root = v != 0,
            K::GetUpd => get_upd = v != 0,
            K::HuInt => pc.chanmgr.set_update_interval(v as u32),
            K::ForceIp => sm.settings().force_ip = pcs(&op.str),
            K::HtmlPath => {
                if v != 0 {
                    sm.settings().html_path = op.str[..op.str.len().min(127)].to_vec();
                } else {
                    crate::log_warn!("Ignoring invalid htmlPath");
                }
            }
            K::DjMsg => pc.chanmgr.set_broadcast_msg(pc, &pcs(&op.str)),
            K::PcMsg => sm.settings().root_msg = pcs(&op.str),
            K::MaxCin => sm.settings().max_control = v as u32,
            K::MaxSin => sm.settings().max_serv_in = v as u32,
            K::MaxUp => sm.settings().max_bitrate_out = v as u32,
            K::MaxRelays => sm.set_max_relays(v),
            K::MaxDirect => sm.settings().max_direct = v as u32,
            K::MaxRelayPc => pc.chanmgr.settings().max_relays_per_channel = v,
            K::FiltIp => {
                // ip が最初
                let n = filters.len() - 1;
                cur = n;
                let f = &mut filters[n];
                f.init();
                f.set_pattern(&op.str);
                if f.is_set() && n < super::servmgr::MAX_FILTERS - 1 {
                    filters.push(sf::ServFilter::default());
                }
            }
            K::FiltBan => filters[cur].flags |= sf::F_BAN,
            K::FiltPrivate => filters[cur].flags |= sf::F_PRIVATE,
            K::FiltNetwork => filters[cur].flags |= sf::F_NETWORK,
            K::FiltDirect => filters[cur].flags |= sf::F_DIRECT,
            K::ChannelFeedUrl => {
                sm.channel_directory.add_feed(&op.str);
            }
            K::ClientActive => sm.settings().auto_connect = v != 0,
            K::Yp => {
                let mut s = sm.settings();
                if op.str != s.root_host.data {
                    crate::log_info!("Root host changed from '{}' to '{}'", b(&s.root_host.data), st());
                    s.root_host = pcs(&op.str);
                    s.root_msg.clear();
                }
            }
            K::DeadHitAge => pc.chanmgr.settings().dead_hit_age = v as u32,
            K::Refresh => sm.settings().refresh_html = v as u32,
            K::Chat => sm.settings().chat = v != 0,
            K::RandomizeChid => {
                if let Some(f) = sm.flags.find(b"randomizeBroadcastingChannelID") {
                    f.set(v != 0);
                }
            }
            K::PublicDirectory => sm.settings().public_directory_enabled = true,
            K::Auth => sm.settings().auth_type = if v == 1 { AUTH_COOKIE } else { AUTH_HTTPBASIC },
            K::Expire => sm.cookies().never_expire = v != 0,
            K::LogLevel => super::log::set_level(v),
            K::AllowHtml => allow_server1 |= if v != 0 { svt::ALLOW_HTML } else { 0 },
            K::AllowNetwork => allow_server1 |= if v != 0 { svt::ALLOW_NETWORK } else { 0 },
            K::AllowBroadcast => allow_server1 |= if v != 0 { svt::ALLOW_BROADCAST } else { 0 },
            K::AllowDirect => allow_server1 |= if v != 0 { svt::ALLOW_DIRECT } else { 0 },
            K::Transcoding => sm.settings().transcoding_enabled = v != 0,
            K::Preset => sm.settings().preset = op.str.clone(),
            K::AudioCodec => sm.settings().audio_codec = op.str.clone(),
            K::PreferredTheme => sm.settings().preferred_theme = op.str.clone(),
            K::AccentColor => sm.settings().accent_color = op.str.clone(),
        }
    }
    let (port, hp) = {
        let mut s = sm.settings();
        s.filters = filters;
        s.allow_server1 = allow_server1;
        (s.server_host.port, s.html_path.clone())
    };
    if port as i32 != new_port {
        let host_header = http.headers.get(b"Host");
        let ipstr = if !host_header.is_empty() {
            let first = crate::strutil::split(&host_header, b":").into_iter().next().unwrap_or_default();
            [&first[..], format!(":{}", new_port).as_bytes()].concat()
        } else {
            Host::v4(super::host::get_ip(&sys::hostname()), new_port as u16).str().into_bytes()
        };
        set_jump(jump, [&b"http://"[..], &ipstr, b"/", &hp, b"/settings.html"].concat());
        pc.set_server_port(new_port as u16);
        // サーバーが始め直す時間
        sys::sleep(500);
    } else {
        set_jump(jump, [&b"/"[..], &hp, b"/settings.html"].concat());
    }
    pc.save_settings();
    pc.notify_message(super::notif::NT_PEERCAST, "設定を保存しました。".as_bytes());
    if sm.is_root() && br_root {
        sm.broadcast_root_settings(pc, get_upd);
    }
    Ok(())
}

// ---------------------------------------------------------------- dump_hitlists、viewxml

/// `Servent::formatTimeDifference`
fn format_time_difference(t: u32, now: u32) -> String {
    let d = (now as u64).wrapping_sub(t as u64) as i32;
    if d < 0 {
        format!("({}s into the future)", d)
    } else if d > 0 {
        format!("({}s ago)", d)
    } else {
        "just now".to_string()
    }
}

fn indent(text: &[u8], n: i32) -> Vec<u8> {
    crate::strutil::indent_tab(text, n).unwrap_or_default()
}

fn insp(s: &[u8]) -> Vec<u8> {
    crate::inspect::inspect(s)
}

fn bl(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

fn dump_hit(h: &ChanHit) -> Vec<u8> {
    let now = sys::get_time();
    let mut buf = Vec::new();
    let mut l = |s: String| {
        buf.extend_from_slice(s.as_bytes());
        buf.push(b'\n');
    };
    l(format!("host = {}", h.host.str()));
    l(format!("rhost[0] = {}", h.rhost[0].str()));
    l(format!("rhost[1] = {}", h.rhost[1].str()));
    l(format!("numlisteners = {}", h.num_listeners));
    l(format!("numRelays = {}", h.num_relays));
    l(format!("numHops = {}", h.num_hops));
    l(format!("sessionID = {}", ci::id_str(&h.session_id)));
    l(format!("chanID = {}", ci::id_str(&h.chan_id)));
    l(format!("version = {}", h.version));
    l(format!("versionVP = {}", h.version_vp));
    l(format!("versionExPrefix = {}", b(&insp(&h.version_ex_prefix))));
    l(format!("versionExNumber = {}", h.version_ex_number));
    l(format!("oldestPos = {}", h.oldest_pos));
    l(format!("newestPos = {}", h.newest_pos));
    l(format!("firewalled = {}", bl(h.firewalled)));
    l(format!("stable = {}", bl(h.stable)));
    l(format!("tracker = {}", bl(h.tracker)));
    l(format!("recv = {}", bl(h.recv)));
    l(format!("yp = {}", bl(h.yp)));
    l(format!("dead = {}", bl(h.dead)));
    l(format!("direct = {}", bl(h.direct)));
    l(format!("relay = {}", bl(h.relay)));
    l(format!("cin = {}", bl(h.cin)));
    l(format!("uphost = {}", h.uphost.str()));
    l(format!("uphostHops = {}", h.uphost_hops));
    l(format!("time = {} {}", h.time, format_time_difference(h.time, now)));
    l(format!("upTime = {}", h.up_time));
    l(format!(
        "lastContact = {}{}",
        h.last_contact,
        if h.last_contact != 0 { format!(" {}", format_time_difference(h.last_contact, now)) } else { String::new() }
    ));
    [&b"ChanHit\n"[..], &indent(&buf, 1)].concat()
}

fn dump_chan_info(info: &ChanInfo) -> Vec<u8> {
    let now = sys::get_time();
    let mut buf = Vec::new();
    {
        let mut l = |k: &str, v: &[u8]| {
            buf.extend_from_slice(k.as_bytes());
            buf.extend_from_slice(b" = ");
            buf.extend_from_slice(v);
            buf.push(b'\n');
        };
        l("name", &insp(&info.name.data));
        l("id", ci::id_str(&info.id).as_bytes());
        l("bcID", ci::id_str(&info.bc_id).as_bytes());
        l("bitrate", info.bitrate.to_string().as_bytes());
        l("contentType", &info.content_type.data);
        l("MIMEType", &insp(&info.mime_type.data));
        l("streamExt", &insp(&info.stream_ext.data));
        l("srcProtocol", info.src_protocol.to_string().as_bytes());
        l("lastPlayStart", info.last_play_start.to_string().as_bytes());
        l("lastPlayEnd", info.last_play_end.to_string().as_bytes());
        l("numSkips", info.num_skips.to_string().as_bytes());
        l("createdTime", format!("{} {}", info.created_time, format_time_difference(info.created_time, now)).as_bytes());
        l("status", info.status.to_string().as_bytes());
    }
    let mut t = Vec::new();
    for (k, v) in [
        ("contact", &info.track.contact),
        ("title", &info.track.title),
        ("artist", &info.track.artist),
        ("album", &info.track.album),
        ("genre", &info.track.genre),
    ] {
        t.extend_from_slice(format!("{} = ", k).as_bytes());
        t.extend_from_slice(&insp(&v.data));
        t.push(b'\n');
    }
    buf.extend_from_slice(b"TrackInfo\n");
    buf.extend(indent(&t, 1));
    for (k, v) in [("desc", &info.desc), ("genre", &info.genre), ("url", &info.url), ("comment", &info.comment)] {
        buf.extend_from_slice(format!("{} = ", k).as_bytes());
        buf.extend_from_slice(&insp(&v.data));
        buf.push(b'\n');
    }
    [&b"ChanInfo\n"[..], &indent(&buf, 1)].concat()
}

/// `CMD_dump_hitlists`
fn dump_hitlists(pc: &Peercast) -> Vec<u8> {
    pc.chanmgr.with_hitlists(|lists| {
        let n = lists.len();
        let mut buf = format!("{} hit list{} found.\n\n", n, if n != 1 { "s" } else { "" }).into_bytes();
        for (i, l) in lists.iter().enumerate() {
            buf.extend_from_slice(b"ChanHitList\n");
            buf.extend_from_slice(format!("\tused = {}\n", bl(l.used)).as_bytes());
            buf.extend_from_slice(format!("\tlastHitTime = {}\n", l.last_hit_time).as_bytes());
            buf.extend_from_slice(b"\tinfo:\n");
            buf.extend(indent(&dump_chan_info(&l.info), 1));
            buf.extend_from_slice(format!("\thit ({} entries):\n", l.hits.len()).as_bytes());
            for h in &l.hits {
                buf.extend(indent(&dump_hit(h), 2));
            }
            if i + 1 < n {
                buf.push(b'\n');
            }
        }
        buf
    })
}

/// `handshakeXML`
fn handshake_xml(ctx: &Ctx, http: &mut Http) -> Result<()> {
    use super::stats::{per_second, Stat};
    use super::xmlnode::XmlNode;
    let pc = ctx.pc;
    let sm = &pc.servmgr;
    let max_uptime = pc.chanmgr.max_uptime();
    let mut rn = XmlNode::new("peercast");
    rn.add(XmlNode::new(format!("servent uptime=\"{}\"", sm.uptime() as i32)));
    rn.add(XmlNode::new(format!(
        "bandwidth out=\"{}\" in=\"{}\"",
        per_second(Stat::BytesOut).wrapping_sub(per_second(Stat::LocalBytesOut)) as i32,
        per_second(Stat::BytesIn).wrapping_sub(per_second(Stat::LocalBytesIn)) as i32
    )));
    rn.add(XmlNode::new(format!(
        "connections total=\"{}\" relays=\"{}\" direct=\"{}\"",
        sm.num_connected() as i32,
        sm.num_streams_type(pc, svt::T_RELAY, true) as i32,
        sm.num_streams_type(pc, svt::T_DIRECT, true) as i32
    )));
    let mut an = XmlNode::new(format!("channels_relayed total=\"{}\"", pc.chanmgr.num_channels()));
    for c in pc.chanmgr.channels() {
        if c.is_active() {
            let info = c.info();
            let mut n = info.channel_xml(max_uptime);
            n.add(c.relay_xml(pc, true));
            n.add(info.track_xml());
            an.add(n);
        }
    }
    rn.add(an);
    let mut fnode = XmlNode::new(format!("channels_found total=\"{}\"", pc.chanmgr.num_hit_lists()));
    for l in pc.chanmgr.hitlists() {
        let mut n = l.info.channel_xml(max_uptime);
        n.add(l.xml(true));
        n.add(l.info.track_xml());
        fnode.add(n);
    }
    rn.add(fnode);
    let mut hc = XmlNode::new("host_cache");
    for sh in sm.host_cache() {
        if sh.ty != super::servmgr::SH_NONE {
            hc.add(XmlNode::new(format!(
                "host ip=\"{}\" type=\"{}\" time=\"{}\"",
                sh.host.str(),
                super::servmgr::serv_host_type_str(sh.ty),
                sh.time as i32
            )));
        }
    }
    rn.add(hc);
    let doc = rn.write_document()?;
    let s = &mut *http.stream;
    s.write_line(HTTP_SC_OK)?;
    s.write_line(format!("Server: {}", PCX_AGENT))?;
    s.write_line(format!("Content-Type: {}", MIME_XML))?;
    s.write_line("Connection: close")?;
    s.write_line("")?;
    s.write(&doc)
}
