//! 掲示板ビューワーの取得 (もとの Python 版の `safe_urlopen`)。
//!
//! 管理画面から任意のホストを指定できるので、内部のアドレス (ループバック、プライベート、
//! リンクローカルなど) には接続しない。名前を引いたアドレスが全部公開アドレスであることを確かめ、
//! 確かめたアドレスに接続する (Python 版は確かめた後にもう一度名前を引いていた)。リダイレクト先も
//! 同じように確かめる。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use super::host::{Host, Ip};
use super::http::{Headers, Http, Request, PCX_AGENT};
use super::socket::ClientSocket;
use super::stream::Stream;
use crate::bbs::{self, Fetched};

/// 本体の大きさの上限 (`MAX_DOWNLOAD_SIZE`)
const MAX_DOWNLOAD_SIZE: usize = 16 * 1024 * 1024;
/// 読み書きの待ち時間 (`DOWNLOAD_TIMEOUT`、ミリ秒)
const TIMEOUT_MS: u32 = 15000;
/// リダイレクトの回数の上限 (urllib の `HTTPRedirectHandler.max_redirections`)
const MAX_REDIRECTIONS: usize = 10;

fn err<T>(m: impl Into<String>) -> bbs::Result<T> {
    Err(bbs::Error(m.into()))
}

/// 公開アドレスか (Python の `ipaddress` の `is_global` より厳しめ)
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => is_public_v4(a),
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let s = a.segments();
            // NAT64 (64:ff9b::/96) は埋め込まれた IPv4 で決める
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let o = a.octets();
                return is_public_v4(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            // 2000::/3 のうち、特別な用途のもの (2001::/23、文書用 2001:db8::/32 と 3fff::/20、6to4 2002::/16) を除く
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] < 0x200)
                && !(s[0] == 0x2001 && s[1] == 0xdb8)
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

fn is_public_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    let in_net = |net: [u8; 4], bits: u32| -> bool {
        let m = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
        (u32::from(a) & m) == (u32::from_be_bytes(net) & m)
    };
    !(o[0] == 0 // 0.0.0.0/8
        || o[0] == 10
        || in_net([100, 64, 0, 0], 10) // CGN
        || o[0] == 127
        || in_net([169, 254, 0, 0], 16)
        || in_net([172, 16, 0, 0], 12)
        || in_net([192, 0, 0, 0], 24)
        || in_net([192, 0, 2, 0], 24)
        || in_net([192, 88, 99, 0], 24)
        || in_net([192, 168, 0, 0], 16)
        || in_net([198, 18, 0, 0], 15)
        || in_net([198, 51, 100, 0], 24)
        || in_net([203, 0, 113, 0], 24)
        || o[0] >= 224) // マルチキャスト、予約、ブロードキャスト
}

/// 分けた URL
#[derive(Debug, PartialEq, Eq)]
struct Target {
    https: bool,
    /// 名前か IP アドレス (IPv6 は [] を除いたもの)
    host: String,
    port: u16,
    /// パスと問い合わせ (`/` から始まる)
    path: String,
}

impl Target {
    /// `Host` ヘッダーの値 (ポートは既定でないときだけ)
    fn host_header(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let default = if self.https { 443 } else { 80 };
        if self.port == default {
            h
        } else {
            format!("{}:{}", h, self.port)
        }
    }

    fn origin(&self) -> String {
        format!("{}://{}", if self.https { "https" } else { "http" }, self.host_header())
    }
}

/// URL を分ける。http と https だけ。ユーザー名があるもの、空白や制御文字、ASCII でない文字を含むものは不可
fn parse_target(url: &str) -> bbs::Result<Target> {
    if !url.bytes().all(|c| (0x21..0x7f).contains(&c)) {
        return err("bad url");
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(|| bbs::Error("bad url".into()))?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "http" => false,
        "https" => true,
        _ => return err("unsupported scheme"),
    };
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.contains('@') {
        return err("bad url");
    }
    let (host, port) = if let Some(r) = authority.strip_prefix('[') {
        let (h, after) = r.split_once(']').ok_or_else(|| bbs::Error("bad url".into()))?;
        if h.parse::<Ipv6Addr>().is_err() {
            return err("bad url");
        }
        (h, after)
    } else {
        match authority.find(':') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => (authority, ""),
        }
    };
    if host.is_empty() || !(host.contains(':') || host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')) {
        return err("bad url");
    }
    let port = match port {
        "" | ":" => {
            if https {
                443
            } else {
                80
            }
        }
        p => match p.strip_prefix(':').and_then(|p| if p.bytes().all(|c| c.is_ascii_digit()) { p.parse::<u16>().ok() } else { None }) {
            Some(p) if p > 0 => p,
            _ => return err("bad url"),
        },
    };
    let path = if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('?') {
        format!("/{}", path)
    } else {
        path.to_string()
    };
    Ok(Target { https, host: host.to_ascii_lowercase(), port, path })
}

/// リダイレクト先 (`Location`) を、今の URL をもとに絶対の URL にする (`urljoin` の簡易版)
fn join(base: &Target, loc: &str) -> String {
    let lower = loc.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || (loc.contains("://") && !loc.starts_with('/')) {
        return loc.to_string();
    }
    if let Some(r) = loc.strip_prefix("//") {
        return format!("{}://{}", if base.https { "https" } else { "http" }, r);
    }
    if loc.starts_with('/') {
        return format!("{}{}", base.origin(), loc);
    }
    let base_path = base.path.split(['?', '#']).next().unwrap_or("/");
    if loc.is_empty() {
        return format!("{}{}", base.origin(), base.path);
    }
    if loc.starts_with('?') {
        return format!("{}{}{}", base.origin(), base_path, loc);
    }
    let dir = &base_path[..base_path.rfind('/').map_or(0, |i| i + 1)];
    format!("{}{}{}", base.origin(), if dir.is_empty() { "/" } else { dir }, loc)
}

/// 名前を引き、全部公開アドレスなら最初のもの
fn resolve_public(t: &Target, allow: fn(IpAddr) -> bool) -> bbs::Result<Ip> {
    let addrs: Vec<IpAddr> = match (t.host.as_str(), t.port).to_socket_addrs() {
        Ok(it) => it.map(|a| a.ip()).collect(),
        Err(_) => return err("cannot resolve host"),
    };
    if addrs.is_empty() {
        return err("cannot resolve host");
    }
    if !addrs.iter().all(|&a| allow(a)) {
        return err("access to non-public address is not allowed");
    }
    Ok(Ip::from_std(addrs[0]))
}

/// 1 回の要求 (リダイレクトはたどらない)。(状態の番号, Location, 本体)
fn request_once(t: &Target, post: Option<(&[u8], &str)>, allow: fn(IpAddr) -> bool) -> bbs::Result<(i32, String, Vec<u8>)> {
    let ip = resolve_public(t, allow)?;
    let mut sock = if t.https { ClientSocket::new_tls(t.host.as_bytes()) } else { ClientSocket::new() };
    sock.set_read_timeout(TIMEOUT_MS);
    sock.set_write_timeout(TIMEOUT_MS);
    sock.connect(Host::new(ip, t.port)).map_err(|e| bbs::Error(e.to_string()))?;
    let host = t.host_header();
    let mut headers: Vec<(&str, &[u8])> = vec![("Host", host.as_bytes()), ("Connection", b"close"), ("User-Agent", PCX_AGENT.as_bytes())];
    let len;
    let (method, body): (&[u8], &[u8]) = match post {
        Some((body, referer)) => {
            len = body.len().to_string();
            headers.push(("Content-Type", b"application/x-www-form-urlencoded"));
            headers.push(("Content-Length", len.as_bytes()));
            if !referer.is_empty() {
                headers.push(("Referer", referer.as_bytes()));
            }
            (b"POST", body)
        }
        None => (b"GET", b""),
    };
    let mut req = Request::new(method, t.path.as_bytes(), b"HTTP/1.1", Headers::from(&headers));
    req.body = body.to_vec();
    let res = Http::new(&mut sock).send_request(&req).map_err(|e| bbs::Error(e.to_string()))?;
    sock.close();
    if res.body.len() > MAX_DOWNLOAD_SIZE {
        return err("response too large");
    }
    let loc = String::from_utf8_lossy(&res.headers.get(b"Location")).into_owned();
    Ok((res.status_code, loc, res.body))
}

/// 要求して、リダイレクトをたどる (urllib と同じく、POST は 301、302、303 なら GET にし、307、308 は誤り)
fn fetch(url: &str, mut post: Option<(&[u8], &str)>, allow: fn(IpAddr) -> bool) -> bbs::Result<Fetched> {
    let mut t = parse_target(url)?;
    for _ in 0..=MAX_REDIRECTIONS {
        let (code, loc, body) = request_once(&t, post, allow)?;
        match code {
            200..=299 => return Ok(Fetched::Ok(body)),
            301 | 302 | 303 | 307 | 308 if !loc.is_empty() => {
                if post.is_some() {
                    if code == 307 || code == 308 {
                        return Ok(Fetched::Status(code as u16));
                    }
                    post = None;
                }
                t = parse_target(&join(&t, &loc))?;
            }
            c if (100..=999).contains(&c) => return Ok(Fetched::Status(c as u16)),
            _ => return err("bad status"),
        }
    }
    err("too many redirections")
}

/// サーバーで使う取得
pub struct Fetcher;

impl bbs::Fetch for Fetcher {
    fn get(&mut self, url: &str) -> bbs::Result<Fetched> {
        fetch(url, None, is_public)
    }

    fn post(&mut self, url: &str, body: &[u8], referer: &str) -> bbs::Result<Fetched> {
        fetch(url, Some((body, referer)), is_public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses() {
        for a in ["8.8.8.8", "1.1.1.1", "133.242.0.1", "2001:4860:4860::8888", "2400:cb00::1", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(is_public(ip(a)), "{}", a);
        }
        for a in [
            "0.0.0.0", "127.0.0.1", "10.1.2.3", "100.64.0.1", "169.254.169.254", "172.16.0.1", "172.31.255.255", "192.168.1.1",
            "192.0.0.8", "192.0.2.1", "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "240.0.0.1", "255.255.255.255",
            "::", "::1", "fe80::1", "fc00::1", "fd12::1", "ff02::1", "::ffff:127.0.0.1", "::ffff:192.168.0.1", "64:ff9b::7f00:1",
            "2001:db8::1", "2001::1", "2002:c0a8:101::1", "3fff::1", "::127.0.0.1",
        ] {
            assert!(!is_public(ip(a)), "{}", a);
        }
    }

    #[test]
    fn parse_urls() {
        let t = parse_target("http://Example.com/test/SETTING.TXT").unwrap();
        assert_eq!(t, Target { https: false, host: "example.com".into(), port: 80, path: "/test/SETTING.TXT".into() });
        assert_eq!(t.host_header(), "example.com");
        let t = parse_target("https://a.example:8443?x=1#f").unwrap();
        assert_eq!((t.https, t.port, t.path.as_str(), t.host_header().as_str()), (true, 8443, "/?x=1", "a.example:8443"));
        let t = parse_target("http://[2001:db8::1]:81/p").unwrap();
        assert_eq!((t.host.as_str(), t.port, t.host_header().as_str()), ("2001:db8::1", 81, "[2001:db8::1]:81"));
        for u in [
            "ftp://a.example/", "file:///etc/passwd", "http://user@a.example/", "http://a.example@127.0.0.1/", "http://a.example:0/",
            "http://a.example:99999/", "http://a.example:x/", "http://a b/", "http://a.example/\r\nX: y", "http:///x", "http://[::1/",
            "http://a_b/", "gopher://a.example/", "http://あ.example/",
        ] {
            assert!(parse_target(u).is_err(), "{}", u);
        }
    }

    #[test]
    fn join_urls() {
        let b = parse_target("https://a.example/x/y.cgi?q=1").unwrap();
        assert_eq!(join(&b, "http://b.example/z"), "http://b.example/z");
        assert_eq!(join(&b, "//c.example/z"), "https://c.example/z");
        assert_eq!(join(&b, "/z"), "https://a.example/z");
        assert_eq!(join(&b, "z?w"), "https://a.example/x/z?w");
        assert_eq!(join(&b, "?w"), "https://a.example/x/y.cgi?w");
    }

    /// 決まった応答を順に返す HTTP サーバー。受けた要求を返す
    fn serve(responses: Vec<Vec<u8>>) -> (u16, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut got = Vec::new();
            for r in responses {
                let (mut s, _) = l.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                // ヘッダーと、Content-Length の分の本体を読む
                loop {
                    let n = s.read(&mut buf).unwrap();
                    req.extend_from_slice(&buf[..n]);
                    if let Some(i) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..i]).to_ascii_lowercase();
                        let cl = head.lines().find_map(|l| l.strip_prefix("content-length: ")).map_or(0, |v| v.trim().parse().unwrap());
                        if req.len() >= i + 4 + cl {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                got.push(req);
                s.write_all(&r).unwrap();
            }
            got
        });
        (port, h)
    }

    fn loopback(a: IpAddr) -> bool {
        a.is_loopback()
    }

    #[test]
    fn get_follows_redirects() {
        let (port, h) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /b?x=1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n".to_vec(),
        ]);
        let r = fetch(&format!("http://127.0.0.1:{}/a", port), None, loopback).unwrap();
        assert!(matches!(r, Fetched::Ok(ref b) if b == b"abcde"));
        let got = h.join().unwrap();
        let first = String::from_utf8_lossy(&got[0]).into_owned();
        assert!(first.starts_with("GET /a HTTP/1.1\r\n"), "{}", first);
        assert!(first.contains(&format!("Host: 127.0.0.1:{}\r\n", port)), "{}", first);
        assert!(String::from_utf8_lossy(&got[1]).starts_with("GET /b?x=1 HTTP/1.1\r\n"));
    }

    #[test]
    fn post_and_errors() {
        let (port, h) = serve(vec![
            b"HTTP/1.1 303 See Other\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
            b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let base = format!("http://127.0.0.1:{}", port);
        let r = fetch(&format!("{}/bbs.cgi", base), Some((b"a=1&b=2", "http://ref.example/")), loopback).unwrap();
        assert!(matches!(r, Fetched::Ok(ref b) if b == b"ok"));
        let r = fetch(&format!("{}/bbs.cgi", base), Some((b"a=1", "")), loopback).unwrap();
        assert!(matches!(r, Fetched::Status(307)));
        let r = fetch(&format!("{}/x", base), None, loopback).unwrap();
        assert!(matches!(r, Fetched::Status(404)));
        // リダイレクト先も確かめる
        assert!(fetch(&format!("{}/x", base), None, loopback).is_err());
        let got = h.join().unwrap();
        let post = String::from_utf8_lossy(&got[0]).into_owned();
        assert!(post.starts_with("POST /bbs.cgi HTTP/1.1\r\n"), "{}", post);
        assert!(post.contains("Referer: http://ref.example/\r\n") && post.contains("Content-Length: 7\r\n") && post.ends_with("\r\n\r\na=1&b=2"), "{}", post);
        assert!(String::from_utf8_lossy(&got[1]).starts_with("GET /done HTTP/1.1\r\n"));
    }

    #[test]
    fn refuses_private_addresses() {
        // 名前を引いた結果がループバックなら接続しない (実際には接続しない)
        let mut f = Fetcher;
        use crate::bbs::Fetch;
        for u in ["http://127.0.0.1:1/", "http://localhost:1/", "http://[::1]:1/", "http://10.0.0.1:1/"] {
            match f.get(u) {
                Err(e) => assert!(e.0.contains("non-public") || e.0.contains("resolve"), "{} {:?}", u, e),
                Ok(_) => panic!("{}", u),
            }
        }
    }
}
