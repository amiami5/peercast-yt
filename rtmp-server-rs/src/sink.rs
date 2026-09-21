//! 出力先。C++ 版 rtmp-server.cpp の openUri() 相当。
//!
//! - `http://host:port/path?query` : 接続して `POST path?query HTTP/1.0` を送り、以降は FLV をそのまま流す
//! - `file://...` または URL でない文字列 : ファイルに書き出す (既存なら置き換え)

use crate::{Error, Result};
use std::fs::File;
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// 複数の出力先に同じデータを書く。どれかが失敗したら全体が失敗する (C++ 版の StreamSplitter と同じ)。
pub struct Splitter(pub Vec<Box<dyn Write>>);

impl Write for Splitter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for s in &mut self.0 {
            s.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        for s in &mut self.0 {
            s.flush()?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub struct HttpTarget {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: String,
}

/// `scheme://` を持つなら (scheme, 残り) を返す。
fn split_scheme(spec: &str) -> Option<(&str, &str)> {
    let i = spec.find("://")?;
    let scheme = &spec[..i];
    if scheme.is_empty()
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return None;
    }
    Some((scheme, &spec[i + 3..]))
}

pub fn parse_http(rest: &str) -> Result<HttpTarget> {
    let end = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let (authority, after) = rest.split_at(end);
    let after = after.split('#').next().unwrap_or("");
    let (path, query) = match after.find('?') {
        Some(i) => (&after[..i], &after[i + 1..]),
        None => (after, ""),
    };
    let path = if path.is_empty() { "/" } else { path };

    let hostport = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
        // [::1]:7144
        let close = stripped.find(']').ok_or_else(|| Error::protocol("invalid URL"))?;
        let host = &stripped[..close];
        let tail = &stripped[close + 1..];
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| Error::protocol("invalid port"))?,
            None if tail.is_empty() => 80,
            None => return Err(Error::protocol("invalid URL")),
        };
        (host, port)
    } else {
        match hostport.rfind(':') {
            Some(i) => (
                &hostport[..i],
                hostport[i + 1..].parse::<u16>().map_err(|_| Error::protocol("invalid port"))?,
            ),
            None => (hostport, 80),
        }
    };
    if host.is_empty() {
        return Err(Error::protocol("invalid URL: no host"));
    }

    // リクエスト行に埋め込むので、空白や制御文字 (CR/LF を含む) は許さない。
    for s in [path, query] {
        if s.bytes().any(|b| b <= 0x20 || b == 0x7f) {
            return Err(Error::protocol("invalid characters in URL"));
        }
    }

    Ok(HttpTarget { host: host.to_string(), port, path: path.to_string(), query: query.to_string() })
}

fn connect(t: &HttpTarget) -> Result<TcpStream> {
    let mut last_err: Option<io::Error> = None;
    for addr in (t.host.as_str(), t.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
    }
    Err(match last_err {
        Some(e) => Error::Io(e),
        None => Error::protocol("host not found"),
    })
}

pub fn open_sink(spec: &str) -> Result<Box<dyn Write>> {
    match split_scheme(spec) {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => {
            let target = parse_http(rest)?;
            let mut sock = connect(&target)?;
            sock.set_write_timeout(Some(WRITE_TIMEOUT))?;
            sock.set_nodelay(true)?;
            // C++ 版と同じリクエスト。ヘッダーなしの HTTP/1.0。
            write!(sock, "POST {}?{} HTTP/1.0\r\n\r\n", target.path, target.query)?;
            Ok(Box::new(sock))
        }
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => {
            // file:///tmp/x → /tmp/x  (file://host/tmp/x → /tmp/x)
            let path = if rest.starts_with('/') {
                rest
            } else {
                rest.find('/').map(|i| &rest[i..]).unwrap_or("")
            };
            Ok(Box::new(File::create(path)?))
        }
        Some((scheme, _)) => Err(Error::protocol(format!("Unsupported protocol {}", scheme))),
        None => Ok(Box::new(File::create(spec)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_peercast_endpoint() {
        let t = parse_http("localhost:7144/?name=x&type=FLV&ipv=4").unwrap();
        assert_eq!(
            t,
            HttpTarget {
                host: "localhost".into(),
                port: 7144,
                path: "/".into(),
                query: "name=x&type=FLV&ipv=4".into()
            }
        );
    }

    #[test]
    fn parses_ipv6_and_defaults() {
        let t = parse_http("[::1]:8080/a/b?q").unwrap();
        assert_eq!((t.host.as_str(), t.port, t.path.as_str(), t.query.as_str()), ("::1", 8080, "/a/b", "q"));
        let t = parse_http("example.com").unwrap();
        assert_eq!((t.port, t.path.as_str(), t.query.as_str()), (80, "/", ""));
    }

    #[test]
    fn rejects_header_injection() {
        assert!(parse_http("localhost:1/?a=b\r\nX: y").is_err());
        assert!(parse_http("localhost:1/?a=b c").is_err());
    }

    #[test]
    fn unsupported_scheme() {
        assert!(open_sink("https://example.com/").is_err());
    }
}
