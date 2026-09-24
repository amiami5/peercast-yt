//! HTTP のやりとり (core/common/http.h / http.cpp の `HTTP`、`HTTPHeaders`、`HTTPRequest`、
//! `HTTPResponse`、`http::get`)。行とヘッダーの解析そのものは段階 3a の `crate::http`。

use std::collections::BTreeMap;

use super::error::{Error, Kind, Result};
use super::host::Host;
use super::socket::ClientSocket;
use super::stream::{Stream, StreamExt};
use super::sys;
use crate::strutil;

pub use crate::version::{PCX_AGENT, PCX_VERSTRING};

pub const HTTP_SC_OK: &str = "HTTP/1.0 200 OK";
pub const HTTP_SC_NOTFOUND: &str = "HTTP/1.0 404 Not Found";
pub const HTTP_SC_UNAVAILABLE: &str = "HTTP/1.0 503 Service Unavailable";
pub const HTTP_SC_UNAUTHORIZED: &str = "HTTP/1.0 401 Unauthorized";
pub const HTTP_SC_FOUND: &str = "HTTP/1.0 302 Found";
pub const HTTP_SC_BADREQUEST: &str = "HTTP/1.0 400 Bad Request";
pub const HTTP_SC_FORBIDDEN: &str = "HTTP/1.0 403 Forbidden";
pub const HTTP_SC_SWITCH: &str = "HTTP/1.0 101 Switch protocols";
pub const HTTP_SC_BADGATEWAY: &str = "HTTP/1.0 502 Bad Gateway";
pub const HTTP_SC_SERVERERROR: &str = "HTTP/1.0 500 Internal Server Error";
pub const HTTP_SC_URITOOLONG: &str = "HTTP/1.0 414 URI Too Long";
pub const HTTP_SC_TOOMANYREQUESTS: &str = "HTTP/1.0 429 Too Many Requests";

pub const MIME_MP3: &str = "audio/mpeg";
pub const MIME_XMP3: &str = "audio/x-mpeg";
pub const MIME_OGG: &str = "application/ogg";
pub const MIME_XOGG: &str = "application/x-ogg";
pub const MIME_MOV: &str = "video/quicktime";
pub const MIME_MPG: &str = "video/mpeg";
pub const MIME_FLV: &str = "video/x-flv";
pub const MIME_MKV: &str = "video/x-matroska";
pub const MIME_WEBM: &str = "video/webm";
pub const MIME_MP4: &str = "video/mp4";
pub const MIME_HTML: &str = "text/html";
pub const MIME_XML: &str = "text/xml";
pub const MIME_CSS: &str = "text/css";
pub const MIME_TEXT: &str = "text/plain";
pub const MIME_PLS: &str = "audio/mpegurl";
pub const MIME_XPLS: &str = "audio/x-mpegurl";
pub const MIME_XSCPLS: &str = "audio/x-scpls";
pub const MIME_M3U: &str = "audio/m3u";
pub const MIME_XPCP: &str = "application/x-peercast-pcp";
pub const MIME_RAW: &str = "application/binary";
pub const MIME_JS: &str = "application/javascript; charset=utf-8";

/// `HTTPException`: 返す状態の行と番号
pub fn http_error(status_line: &str, code: i32) -> Error {
    Error::new(Kind::Http, status_line).with_err(code)
}

/// `HTTPException(m, c, message)`
pub fn http_error_msg(status_line: &str, code: i32, message: &str) -> Error {
    let mut e = http_error(status_line, code);
    e.detail = message.to_string();
    e
}

/// `HTTPHeaders`: 名前は大文字にして持つ
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub BTreeMap<Vec<u8>, Vec<u8>>);

impl Headers {
    pub fn new() -> Headers {
        Headers::default()
    }

    pub fn from(pairs: &[(&str, &[u8])]) -> Headers {
        let mut h = Headers::new();
        for (k, v) in pairs {
            h.set(k.as_bytes(), v);
        }
        h
    }

    pub fn set(&mut self, name: &[u8], value: &[u8]) {
        self.0.insert(strutil::upcase(name), value.to_vec());
    }

    pub fn get(&self, name: &[u8]) -> Vec<u8> {
        self.0.get(&strutil::upcase(name)).cloned().unwrap_or_default()
    }

    /// `hasKeyWithValue`: `,` で区切った値のどれかが (大文字小文字を区別せず) 一致するか
    pub fn has_key_with_value(&self, key: &[u8], value: &[u8]) -> bool {
        match self.0.get(&strutil::upcase(key)) {
            None => false,
            Some(v) => {
                let target = strutil::upcase(&strutil::strip(value));
                strutil::split(v, b",").iter().any(|w| strutil::upcase(&strutil::strip(w)) == target)
            }
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Vec<u8>)> {
        self.0.iter()
    }
}

/// `HTTPRequest`
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: Vec<u8>,
    pub url: Vec<u8>,
    pub path: Vec<u8>,
    pub query_string: Vec<u8>,
    pub protocol_version: Vec<u8>,
    pub body: Vec<u8>,
    pub headers: Headers,
}

impl Request {
    pub fn new(method: &[u8], url: &[u8], protocol_version: &[u8], headers: Headers) -> Request {
        let (path, query_string) = crate::http::split_request_url(url);
        Request {
            method: method.to_vec(),
            url: url.to_vec(),
            path,
            query_string,
            protocol_version: protocol_version.to_vec(),
            body: Vec::new(),
            headers,
        }
    }
}

/// `HTTPResponse`
pub struct Response {
    pub status_code: i32,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// 本体を読み出すもの (あれば `body` の代わりに、読めなくなるまで写す)
    pub stream: Option<Box<dyn Stream>>,
}

impl Response {
    pub fn new(status_code: i32, headers: Headers) -> Response {
        Response { status_code, headers, body: Vec::new(), stream: None }
    }

    pub fn ok(headers: Headers, body: Vec<u8>) -> Response {
        let mut r = Response::new(200, headers);
        r.body = body;
        r
    }

    pub fn not_found(message: &[u8]) -> Response {
        let mut r = Response::new(404, Headers::from(&[("Content-Type", b"text/html")]));
        r.body = message.to_vec();
        r
    }

    /// `serverError` (C++ 版のヘッダーの名前の綴りの誤り "Conetnt-Type" もそのまま)
    pub fn server_error(message: &[u8]) -> Response {
        let mut r = Response::new(500, Headers::from(&[("Conetnt-Type", b"text/html")]));
        r.body = if message.is_empty() { b"Internal server error".to_vec() } else { message.to_vec() };
        r
    }

    pub fn redirect_to(url: &[u8]) -> Response {
        Response::new(302, Headers::from(&[("Location", url)]))
    }

    pub fn bad_request(message: &[u8]) -> Response {
        let mut r = Response::new(400, Headers::from(&[("Content-Type", b"text/html")]));
        r.body = message.to_vec();
        r
    }

    pub fn not_modified(headers: Headers) -> Response {
        Response::new(304, headers)
    }
}

/// `statusMessage`
pub fn status_message(code: i32) -> &'static str {
    match code {
        101 => "Switch protocols",
        200 => "OK",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `cgi::rfc1123Time`
pub fn rfc1123_time(t: i64) -> String {
    let tm = super::os::gmtime(t);
    let s = format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[tm.wday as usize],
        tm.mday,
        MONTHS[tm.mon as usize],
        tm.year as i64 + 1900,
        tm.hour,
        tm.min,
        tm.sec
    );
    // C++ 版は 30 バイトのバッファ
    s.chars().take(29).collect()
}

/// `str::capitalize` を名前に
fn cap(name: &[u8]) -> Vec<u8> {
    strutil::capitalize(name)
}

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `HTTP`: 要求や応答の行とヘッダーを 1 つずつ読む
pub struct Http<'a> {
    pub stream: &'a mut dyn Stream,
    /// 最後に読んだ行 (`cmdLine`)
    pub cmd_line: Vec<u8>,
    /// ヘッダーの値 (`arg`)
    pub arg: Option<Vec<u8>>,
    headers_read: bool,
    header_count: usize,
    body: Option<Vec<u8>>,
    pub method: Vec<u8>,
    pub request_url: Vec<u8>,
    pub protocol_version: Vec<u8>,
    pub headers: Headers,
}

/// 1 つの要求や応答で受け付けるヘッダーの行の数
pub const MAX_HEADERS: usize = 128;
/// `getRequest` で読む POST の本体の上限
pub const MAX_REQUEST_BODY: i32 = 1024 * 1024;
/// `getResponse` で読む応答の本体の上限
pub const MAX_RESPONSE_BODY: usize = 32 * 1024 * 1024;
/// `cmdLine` の大きさ
const CMDLINE_LEN: usize = 8192;

impl<'a> Http<'a> {
    pub fn new(stream: &'a mut dyn Stream) -> Http<'a> {
        Http {
            stream,
            cmd_line: Vec::new(),
            arg: None,
            headers_read: false,
            header_count: 0,
            body: None,
            method: Vec::new(),
            request_url: Vec::new(),
            protocol_version: Vec::new(),
            headers: Headers::new(),
        }
    }

    fn read_cmd_line(&mut self) -> Result<()> {
        let l = self.stream.read_line_buf(CMDLINE_LEN)?;
        self.cmd_line = until_nul(&l).to_vec();
        Ok(())
    }

    /// `readRequest`
    pub fn read_request(&mut self) -> Result<()> {
        self.read_cmd_line()?;
        self.parse_request_line();
        Ok(())
    }

    /// `initRequest`
    pub fn init_request(&mut self, r: &[u8]) {
        let r = until_nul(r);
        self.cmd_line = r[..r.len().min(CMDLINE_LEN - 1)].to_vec();
        self.parse_request_line();
    }

    /// `parseRequestLine`
    pub fn parse_request_line(&mut self) {
        let v = strutil::split(&self.cmd_line, b" ");
        if !v.is_empty() {
            self.method = v[0].clone();
        }
        if v.len() > 1 {
            self.request_url = v[1].clone();
        }
        if v.len() > 2 {
            self.protocol_version = v[2].clone();
        }
    }

    /// `readResponse`: 状態の番号 (行は番号の後ろで切る)
    pub fn read_response(&mut self) -> Result<i32> {
        self.read_cmd_line()?;
        let (status, cut) = crate::http::parse_status_line(&self.cmd_line);
        self.cmd_line.truncate(cut);
        Ok(status)
    }

    /// `checkResponse`
    pub fn check_response(&mut self, r: i32) -> Result<()> {
        if self.read_response()? != r {
            crate::log_error!("Unexpected HTTP: {}", String::from_utf8_lossy(&self.cmd_line));
            return Err(Error::stream("Unexpected HTTP response"));
        }
        Ok(())
    }

    /// `nextHeader`: 次のヘッダーの行。空行なら false
    pub fn next_header(&mut self) -> Result<bool> {
        if self.headers_read {
            return Ok(false);
        }
        self.read_cmd_line()?;
        if !self.cmd_line.is_empty() {
            self.header_count += 1;
            if self.header_count > MAX_HEADERS {
                return Err(Error::stream("Too many headers"));
            }
            match crate::http::parse_header_line(&self.cmd_line) {
                Some(h) => {
                    self.arg = Some(self.cmd_line[h.arg_offset..].to_vec());
                    self.headers.set(&h.name, &h.value);
                }
                None => self.arg = None,
            }
            Ok(true)
        } else {
            self.arg = None;
            self.headers_read = true;
            Ok(false)
        }
    }

    /// `readHeaders`
    pub fn read_headers(&mut self) -> Result<()> {
        while self.next_header()? {}
        Ok(())
    }

    /// `isHeader`: 行のどこかに (大文字小文字を区別せず) 含まれるか
    pub fn is_header(&self, hs: &str) -> bool {
        crate::http::stristr(&self.cmd_line, hs.as_bytes()).is_some()
    }

    /// `isRequest`: 行がこれで始まるか
    pub fn is_request(&self, rq: &str) -> bool {
        self.cmd_line.starts_with(rq.as_bytes())
    }

    /// `getArgStr`
    pub fn arg_str(&self) -> Option<&[u8]> {
        self.arg.as_deref()
    }

    /// `getArgInt`
    pub fn arg_int(&self) -> i32 {
        self.arg.as_deref().map_or(0, crate::http::atoi)
    }

    /// `getAuthUserPass`: Basic 認証 (ユーザー名とパスワードは 63 バイトまで)
    pub fn auth_user_pass(&self) -> (Vec<u8>, Vec<u8>) {
        parse_authorization_header(self.arg.as_deref().unwrap_or(b""))
    }

    fn write_line_f(&mut self, parts: &[&[u8]]) -> Result<()> {
        let mut l = Vec::new();
        for p in parts {
            l.extend_from_slice(until_nul(p));
        }
        self.stream.write_line(&l)
    }

    /// `writeResponseHeaders`
    pub fn write_response_headers(&mut self, additional: &Headers) -> Result<()> {
        let mut headers: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        headers.insert(b"Server".to_vec(), PCX_AGENT.as_bytes().to_vec());
        headers.insert(b"Connection".to_vec(), b"close".to_vec());
        headers.insert(b"Date".to_vec(), rfc1123_time(sys::get_time() as i64).into_bytes());
        for (k, v) in additional.iter() {
            headers.insert(cap(k), v.clone());
        }
        for (k, v) in &headers {
            self.write_line_f(&[k, b": ", v])?;
        }
        self.stream.write_line(b"")
    }

    /// `writeResponseStatus`
    pub fn write_response_status(&mut self, protocol_version: &str, code: i32) -> Result<()> {
        if protocol_version != "HTTP/1.0" && protocol_version != "HTTP/1.1" {
            return Err(Error::argument(format!("Unknown protocol version string \"{}\"", protocol_version)));
        }
        self.stream.write_line(format!("{} {} {}", protocol_version, code, status_message(code)))
    }

    /// `getRequest`
    pub fn get_request(&mut self) -> Result<Request> {
        if self.method.is_empty() || self.request_url.is_empty() || self.protocol_version.is_empty() || !self.cmd_line.is_empty() {
            return Err(Error::general("Request not ready"));
        }
        let mut req = Request::new(&self.method, &self.request_url, &self.protocol_version, self.headers.clone());
        if self.method == b"POST" {
            let cl = self.headers.get(b"Content-Length");
            if cl.is_empty() {
                return Err(Error::general("POST without Content-Length"));
            }
            let size = crate::http::atoi(&cl);
            if size < 0 {
                return Err(http_error(HTTP_SC_BADREQUEST, 400));
            }
            if size > MAX_REQUEST_BODY {
                return Err(http_error("HTTP/1.0 413 Request Entity Too Large", 413));
            }
            if self.body.is_none() {
                self.body = Some(self.stream.read_n(size as usize)?);
            }
            req.body = self.body.clone().unwrap_or_default();
        }
        Ok(req)
    }

    /// `send(const HTTPResponse&)`: 応答を書く (行の終わりは常に CRLF)
    pub fn send_response(&mut self, mut response: Response) -> Result<()> {
        let mut out = Vec::new();
        out.extend(format!("HTTP/1.0 {} {}\r\n", response.status_code, status_message(response.status_code)).into_bytes());
        let mut headers: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        headers.insert(b"Server".to_vec(), PCX_AGENT.as_bytes().to_vec());
        headers.insert(b"Connection".to_vec(), b"close".to_vec());
        headers.insert(b"Date".to_vec(), rfc1123_time(sys::get_time() as i64).into_bytes());
        let body_is_stream = response.stream.is_some();
        if response.headers.get(b"Content-Length").is_empty() && !body_is_stream {
            headers.insert(b"Content-Length".to_vec(), response.body.len().to_string().into_bytes());
        }
        for (k, v) in response.headers.iter() {
            headers.insert(cap(k), v.clone());
        }
        for (k, v) in &headers {
            out.extend_from_slice(until_nul(k));
            out.extend_from_slice(b": ");
            out.extend_from_slice(until_nul(v));
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        self.stream.write(&out)?;
        match response.stream.as_mut() {
            Some(s) => {
                let mut buf = [0u8; 4096];
                loop {
                    match s.read(&mut buf) {
                        Ok(r) => self.stream.write(&buf[..r])?,
                        Err(e) if e.is_stream() => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            None => self.stream.write(&response.body)?,
        }
        Ok(())
    }

    /// `send(const HTTPRequest&)`: 要求を書いて応答を読む
    pub fn send_request(&mut self, req: &Request) -> Result<Response> {
        let path_query =
            if req.query_string.is_empty() { req.path.clone() } else { [&req.path[..], b"?", &req.query_string].concat() };
        self.write_line_f(&[&req.method, b" ", &path_query, b" ", &req.protocol_version])?;
        for (k, v) in req.headers.iter() {
            let k = cap(k);
            self.write_line_f(&[&k, b": ", v])?;
        }
        self.stream.write_line(b"")?;
        if req.method == b"POST" || req.method == b"PUT" {
            let cl = req.headers.get(b"Content-Length");
            if !cl.is_empty() && crate::http::atoi(&cl) as i64 != req.body.len() as i64 {
                return Err(Error::stream("body size mismatch"));
            }
            self.stream.write(&req.body)?;
        }
        self.get_response()
    }

    /// `getResponse`: 状態の行、ヘッダー、本体を読む。
    ///
    /// C++ 版は Content-Length の有無の条件が逆で、Content-Length があるときに接続が閉じるまで
    /// 読み、ないときに 0 バイト読んでいた (docs/cpp-known-issues.md)。Rust 版は、Content-Length が
    /// あればその長さを読み、なければ閉じるまで読む。
    pub fn get_response(&mut self) -> Result<Response> {
        let status = self.read_response()?;
        self.read_headers()?;
        let mut response = Response::new(status, self.headers.clone());
        let mut too_large = false;
        if self.headers.get(b"Transfer-Encoding") == b"chunked" {
            let mut r = super::stream::StreamReader::new(&mut *self.stream);
            loop {
                let (chunk, err) = crate::dechunk::next_chunk(&mut r, MAX_RESPONSE_BODY + 1);
                response.body.extend_from_slice(&chunk);
                if response.body.len() > MAX_RESPONSE_BODY {
                    too_large = true;
                    break;
                }
                if err.is_some() || chunk.is_empty() {
                    break;
                }
            }
        } else {
            let cl = self.headers.get(b"Content-Length");
            if !cl.is_empty() {
                let length = crate::http::atoi(&cl);
                if length < 0 {
                    return Err(Error::stream("invalid Content-Length value"));
                }
                if length as usize > MAX_RESPONSE_BODY {
                    return Err(Error::stream("response too large"));
                }
                response.body = self.stream.read_n(length as usize)?;
            } else {
                let mut buf = [0u8; 4096];
                loop {
                    match self.stream.read_upto(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            response.body.extend_from_slice(&buf[..n]);
                            if response.body.len() > MAX_RESPONSE_BODY {
                                too_large = true;
                                break;
                            }
                        }
                        Err(e) if e.is_stream() => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        if too_large {
            return Err(Error::stream("response too large"));
        }
        Ok(response)
    }

    /// `reset`
    pub fn reset(&mut self) {
        self.cmd_line.clear();
        self.arg = None;
        self.method.clear();
        self.request_url.clear();
        self.protocol_version.clear();
        self.headers.clear();
        self.headers_read = false;
        self.header_count = 0;
    }
}

/// `HTTP::parseAuthorizationHeader`: Basic 認証のユーザー名とパスワード (63 バイトまで)
pub fn parse_authorization_header(arg: &[u8]) -> (Vec<u8>, Vec<u8>) {
    match crate::http::parse_basic_auth(arg) {
        Some((u, p)) => (u[..u.len().min(63)].to_vec(), p[..p.len().min(63)].to_vec()),
        None => (Vec::new(), Vec::new()),
    }
}

/// `http::get`: URL の中身を取る (リダイレクトは 1 回まで)
pub fn get(url: &[u8]) -> Result<Vec<u8>> {
    let mut url = url.to_vec();
    let mut retry = 0;
    loop {
        if retry > 1 {
            return Err(Error::stream("Too many redirections. Giving up ..."));
        }
        let u = String::from_utf8_lossy(&url).into_owned();
        let feed = crate::url::parse_url(&url).map_err(|_| Error::argument(format!("invalid URL ({})", u)))?;
        if feed.scheme != b"http" && feed.scheme != b"https" {
            return Err(Error::argument(format!("unsupported protocol ({})", u)));
        }
        let port = uri_port(&feed);
        let host = Host::from_str_name(&feed.host, port);
        if !host.ip.is_set() {
            return Err(Error::stream(format!("Could not resolve {}", String::from_utf8_lossy(&feed.host))));
        }
        let mut sock = if feed.scheme == b"https" { ClientSocket::new_tls(&feed.host) } else { ClientSocket::new() };
        crate::log_trace!("Connecting to {} ({}) port {} ...", String::from_utf8_lossy(&feed.host), host.ip.str(), port);
        sock.connect(host)?;
        crate::log_trace!("Connected to {}", host.str());
        let mut path = b"/".to_vec();
        path.extend_from_slice(&feed.path);
        let req_path = if feed.query.is_empty() { path } else { [&path[..], b"?", &feed.query].concat() };
        crate::log_trace!("GET {} HTTP/1.1", String::from_utf8_lossy(&req_path));
        let req = Request::new(
            b"GET",
            &req_path,
            b"HTTP/1.1",
            Headers::from(&[("Host", &feed.host), ("Connection", b"close"), ("User-Agent", PCX_AGENT.as_bytes())]),
        );
        let res = Http::new(&mut sock).send_request(&req)?;
        match res.status_code {
            301 | 302 | 307 | 308 => {
                let loc = res.headers.get(b"Location");
                if !loc.is_empty() {
                    crate::log_trace!("Status code {}. Redirecting to {} ...", res.status_code, String::from_utf8_lossy(&loc));
                    url = loc;
                    retry += 1;
                    continue;
                }
                crate::log_error!("Status code {}. No Location header. Giving up ...", res.status_code);
                return Err(Error::stream("No Location header"));
            }
            200 => return Ok(res.body),
            c => {
                crate::log_error!("{}: status code {}", String::from_utf8_lossy(&feed.host), c);
                return Err(Error::stream(format!("status code {}", c)));
            }
        }
    }
}

/// `URI::port`: URL のポート (なければ http は 80、https は 443)
pub fn uri_port(u: &crate::url::Url) -> u16 {
    let p = crate::http::atoi(&u.port);
    if p > 0 && p <= 65535 {
        p as u16
    } else if u.scheme == b"http" {
        80
    } else if u.scheme == b"https" {
        443
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::super::stream::StringStream;
    use super::*;

    #[test]
    fn request_and_headers() {
        let mut s = StringStream::from(b"GET /html/index.html?x=1 HTTP/1.1\r\nHost: localhost:7144\r\nCookie: a=b\r\n\r\n".to_vec());
        let mut h = Http::new(&mut s);
        h.read_request().unwrap();
        h.read_headers().unwrap();
        let r = h.get_request().unwrap();
        assert_eq!(r.path, b"/html/index.html");
        assert_eq!(r.query_string, b"x=1");
        assert_eq!(r.headers.get(b"host"), b"localhost:7144");
        assert!(r.headers.has_key_with_value(b"Cookie", b" A=B "));
    }

    #[test]
    fn response() {
        let mut s = StringStream::new();
        {
            let mut h = Http::new(&mut s);
            h.send_response(Response::ok(Headers::from(&[("content-type", b"text/plain")]), b"hi".to_vec())).unwrap();
        }
        let out = String::from_utf8(s.into_inner()).unwrap();
        assert!(out.starts_with("HTTP/1.0 200 OK\r\n"));
        assert!(out.contains("Content-Length: 2\r\n"));
        assert!(out.contains("Content-Type: text/plain\r\n"));
        assert!(out.contains("Server: PeerCast/0.1218 (YT50-rs2)\r\n"));
        assert!(out.ends_with("\r\n\r\nhi"));

        // Content-Length のある応答
        let mut s = StringStream::from(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef".to_vec());
        let r = Http::new(&mut s).get_response().unwrap();
        assert_eq!(r.body, b"abc");
        // chunked
        let mut s = StringStream::from(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n".to_vec());
        let r = Http::new(&mut s).get_response().unwrap();
        assert_eq!(r.body, b"abcde");
        // 長さなし
        let mut s = StringStream::from(b"HTTP/1.0 404 Not Found\r\n\r\nnope".to_vec());
        let r = Http::new(&mut s).get_response().unwrap();
        assert_eq!((r.status_code, r.body), (404, b"nope".to_vec()));
    }

    #[test]
    fn date() {
        assert_eq!(rfc1123_time(784111777), "Sun, 06 Nov 1994 08:49:37 GMT");
    }
}
