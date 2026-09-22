//! URL の判定。C++ 版 core/common/str.cpp の `is_http_url` に相当する。

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
}
