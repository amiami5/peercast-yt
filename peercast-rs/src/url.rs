//! URL の判定と解析。
//!
//! * `is_http_url`: core/common/str.cpp の `str::is_http_url`
//! * `parse_url`, `port_number`: core/common/LUrlParser.cpp (`URI` クラスの中身)
//! * `source_protocol`: core/common/url.cpp の `URLSource::getSourceProtocol`
//!
//! 入力は C++ の `c_str()` で渡されていたので、どれも最初の NUL の手前までを見る。

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `LUrlParser::LUrlParserError` のうち、解析で起きるもの
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    NoUrlCharacter = 2,
    InvalidSchemeName = 3,
    NoDoubleSlash = 4,
    NoAtSign = 5,
}

/// `LUrlParser::clParseURL` の各部分
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Url {
    pub scheme: Vec<u8>,
    pub host: Vec<u8>,
    pub port: Vec<u8>,
    pub path: Vec<u8>,
    pub query: Vec<u8>,
    pub fragment: Vec<u8>,
    pub user_name: Vec<u8>,
    pub password: Vec<u8>,
}

/// `LUrlParser::clParseURL::ParseURL` (RFC 1738, RFC 3986 に基づく簡易な解析)。
/// `scheme://user:password@host:port/path?query#fragment` を分ける。パスの先頭の '/' は含まない。
pub fn parse_url(input: &[u8]) -> Result<Url, UrlError> {
    let s = until_nul(input);
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut r = Url::default();

    // <scheme>:
    let colon = s.iter().position(|&c| c == b':').ok_or(UrlError::NoUrlCharacter)?;
    let scheme = &s[..colon];
    if !scheme.iter().all(|&c| c.is_ascii_alphabetic() || c == b'+' || c == b'-' || c == b'.') {
        return Err(UrlError::InvalidSchemeName);
    }
    r.scheme = scheme.to_ascii_lowercase();
    let mut cur = colon + 1;

    // "//"
    if at(cur) != b'/' || at(cur + 1) != b'/' {
        return Err(UrlError::NoDoubleSlash);
    }
    cur += 2;

    // '/' より前に '@' があれば、ユーザー名とパスワードがある
    let has_user = s[cur..].iter().find(|&&c| c == b'@' || c == b'/') == Some(&b'@');
    if has_user {
        let mut p = cur;
        while at(p) != 0 && at(p) != b':' && at(p) != b'@' {
            p += 1;
        }
        r.user_name = s[cur..p].to_vec();
        cur = p;
        if at(cur) == b':' {
            cur += 1;
            let mut p = cur;
            while at(p) != 0 && at(p) != b'@' {
                p += 1;
            }
            r.password = s[cur..p].to_vec();
            cur = p;
        }
        if at(cur) != b'@' {
            return Err(UrlError::NoAtSign);
        }
        cur += 1;
    }

    // ホスト名 ('[' で始まれば ']' まで、そうでなければ ':' か '/' まで)
    let bracket = at(cur) == b'[';
    let mut p = cur;
    while at(p) != 0 {
        if bracket && at(p) == b']' {
            p += 1;
            break;
        } else if !bracket && (at(p) == b':' || at(p) == b'/') {
            break;
        }
        p += 1;
    }
    r.host = s[cur..p].to_vec();
    cur = p;

    // :port
    if at(cur) == b':' {
        cur += 1;
        let mut p = cur;
        while at(p) != 0 && at(p) != b'/' {
            p += 1;
        }
        r.port = s[cur..p].to_vec();
        cur = p;
    }

    if at(cur) == b'/' {
        cur += 1;
    }

    // path
    let mut p = cur;
    while at(p) != 0 && at(p) != b'#' && at(p) != b'?' {
        p += 1;
    }
    r.path = s[cur..p].to_vec();
    cur = p;

    // ?query
    if at(cur) == b'?' {
        cur += 1;
        let mut p = cur;
        while at(p) != 0 && at(p) != b'#' {
            p += 1;
        }
        r.query = s[cur..p].to_vec();
        cur = p;
    }

    // #fragment
    if at(cur) == b'#' {
        r.fragment = s[cur + 1..].to_vec();
    }

    Ok(r)
}

/// `clParseURL::GetPort`: ポート番号の文字列を数にする。1〜65535 でなければ `None`。
///
/// C++ 版は `atoi` を使っており、2^32 を超える数が一周して小さな値になっていた
/// (例えば "4294967376" が 80 になる。`long` の幅で CPU によっても違う)。Rust 版は、
/// 範囲を超える数をすべて無効とする。
pub fn port_number(port: &[u8]) -> Option<u16> {
    let p = crate::http::atoi(until_nul(port));
    if (1..=65535).contains(&p) {
        Some(p as u16)
    } else {
        None
    }
}

/// `ChanInfo::PROTOCOL` のうち、`getSourceProtocol` が返すもの (値は C++ の列挙と同じ)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceProtocol {
    Http = 1,
    File = 2,
    Pcp = 3,
    Rtmp = 4,
    Pipe = 5,
}

/// `URLSource::getSourceProtocol`: 入力元の URL の先頭 (大文字小文字は問わない) から種類を決め、
/// その後ろ (読み飛ばす長さ) を返す。どれでもなければファイルとみなす (読み飛ばさない)。
pub fn source_protocol(url: &[u8]) -> (SourceProtocol, usize) {
    let url = until_nul(url);
    let prefixes: [(&[u8], SourceProtocol); 5] = [
        (b"http://", SourceProtocol::Http),
        (b"pcp://", SourceProtocol::Pcp),
        (b"file://", SourceProtocol::File),
        (b"rtmp://", SourceProtocol::Rtmp),
        (b"pipe:", SourceProtocol::Pipe),
    ];
    for (prefix, proto) in prefixes {
        if url.len() >= prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return (proto, prefix.len());
        }
    }
    (SourceProtocol::File, 0)
}

/// ネットワークから受け取った入力元の URL (中継元のリダイレクト先や、HTTP で取ったプレイリストの
/// 中身) として使ってよいか。`http://`、`pcp://`、`rtmp://` で始まるものだけ。`pipe:` (外部の
/// プログラムを起こす) と、ファイル (`file://` やスキームのないもの) は、管理者が入力した URL に限る。
pub fn is_remote_safe_source(url: &[u8]) -> bool {
    let url = until_nul(url);
    let (proto, n) = source_protocol(url);
    n > 0 && matches!(proto, SourceProtocol::Http | SourceProtocol::Pcp | SourceProtocol::Rtmp)
}

/// `http://` または `https://` で始まるか (スキームの大文字小文字は区別しない)。
pub fn is_http_url(url: &[u8]) -> bool {
    let starts_with_ignore_case = |prefix: &[u8]| {
        url.len() >= prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(prefix)
    };
    starts_with_ignore_case(b"http://") || starts_with_ignore_case(b"https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_case_insensitively() {
        assert!(is_http_url(b"http://example.com/"));
        assert!(is_http_url(b"https://example.com/"));
        assert!(is_http_url(b"HTTP://EXAMPLE.COM"));
        assert!(is_http_url(b"HtTpS://x"));
        assert!(is_http_url(b"http://"));
    }

    #[test]
    fn rejects_everything_else() {
        assert!(!is_http_url(b""));
        assert!(!is_http_url(b"http:/"));
        assert!(!is_http_url(b"ftp://example.com/"));
        assert!(!is_http_url(b"file:///etc/passwd"));
        assert!(!is_http_url(b"javascript:alert(1)"));
        assert!(!is_http_url(b" http://example.com/"));
        assert!(!is_http_url(b"httpx://example.com/"));
        assert!(!is_http_url(b"https//example.com/"));
    }

    fn s(v: &[u8]) -> &str {
        std::str::from_utf8(v).unwrap()
    }

    #[test]
    fn parses_urls() {
        let u = parse_url(b"HTTP://user:pw@example.com:8080/a/b?x=1#frag").unwrap();
        assert_eq!(
            (s(&u.scheme), s(&u.user_name), s(&u.password), s(&u.host), s(&u.port), s(&u.path), s(&u.query), s(&u.fragment)),
            ("http", "user", "pw", "example.com", "8080", "a/b", "x=1", "frag")
        );
        let u = parse_url(b"http://[::1]:7144/").unwrap();
        assert_eq!((s(&u.host), s(&u.port), s(&u.path)), ("[::1]", "7144", ""));
        let u = parse_url(b"http://host").unwrap();
        assert_eq!((s(&u.host), s(&u.path)), ("host", ""));
        // '/' より後ろの '@' はユーザー名の区切りではない
        let u = parse_url(b"http://host/a@b").unwrap();
        assert_eq!((s(&u.host), s(&u.user_name), s(&u.path)), ("host", "", "a@b"));
    }

    #[test]
    fn url_errors() {
        assert_eq!(parse_url(b"example.com"), Err(UrlError::NoUrlCharacter));
        assert_eq!(parse_url(b"ht1p://x"), Err(UrlError::InvalidSchemeName));
        assert_eq!(parse_url(b"http:/x"), Err(UrlError::NoDoubleSlash));
        assert_eq!(parse_url(b"http:"), Err(UrlError::NoDoubleSlash));
        // パスワードは最初の '@' まで (':' を含んでよい)。'@' があることは先に確かめているので、
        // NoAtSign は実際には起きない (C++ 版も同じ)。
        let u = parse_url(b"http://a:b:c@h").unwrap();
        assert_eq!((s(&u.user_name), s(&u.password), s(&u.host)), ("a", "b:c", "h"));
        // NUL の後ろは見ない
        assert_eq!(parse_url(b"http\0://x"), Err(UrlError::NoUrlCharacter));
    }

    #[test]
    fn ports() {
        assert_eq!(port_number(b"80"), Some(80));
        assert_eq!(port_number(b"65535"), Some(65535));
        assert_eq!(port_number(b"0"), None);
        assert_eq!(port_number(b"65536"), None);
        assert_eq!(port_number(b"4294967376"), None); // C++ 版は 80 になっていた
        assert_eq!(port_number(b""), None);
    }

    #[test]
    fn source_protocols() {
        assert_eq!(source_protocol(b"HTTP://host/x"), (SourceProtocol::Http, 7));
        assert_eq!(source_protocol(b"pipe:ffmpeg -i x"), (SourceProtocol::Pipe, 5));
        assert_eq!(source_protocol(b"/tmp/a.flv"), (SourceProtocol::File, 0));
        assert_eq!(source_protocol(b"file:///tmp/a"), (SourceProtocol::File, 7));
        // mms:// はサポートをやめたので、ファイル名として扱う (読み飛ばさない)
        assert_eq!(source_protocol(b"mms://host/x"), (SourceProtocol::File, 0));
    }

    #[test]
    fn remote_safe_sources() {
        for u in [&b"http://host/x"[..], b"HTTP://host", b"pcp://host:7144/id", b"rtmp://host/app"] {
            assert!(is_remote_safe_source(u), "{}", String::from_utf8_lossy(u));
        }
        for u in [
            &b"pipe:sh -c id"[..],
            b"PIPE:id",
            b"file:///etc/passwd",
            b"/etc/passwd",
            b"relative/path",
            b"",
            b"https://host/x", // 入力元としては未対応 (ファイル扱いになる)
            b"mms://host/x",
            b"\0http://host/",
        ] {
            assert!(!is_remote_safe_source(u), "{}", String::from_utf8_lossy(u));
        }
    }
}
