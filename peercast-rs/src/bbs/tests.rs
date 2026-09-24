//! 掲示板ビューワーのテスト。`fixtures.rs` は、もとの Python 版を同じ入力で動かした結果。

use super::*;

#[path = "fixtures.rs"]
mod fixtures;

/// 決まった中身を返す取得。POST されたものを覚える
struct Mock {
    pages: &'static [(&'static str, fixtures::Page)],
    posted: Vec<(String, Vec<u8>, String)>,
}

impl Mock {
    fn page(&self, url: &str) -> Result<Fetched> {
        match self.pages.iter().find(|(u, _)| *u == url) {
            Some((_, fixtures::Page::Body(b))) => Ok(Fetched::Ok(b.to_vec())),
            Some((_, fixtures::Page::Status(c))) => Ok(Fetched::Status(*c)),
            None => err(format!("no page {}", url)),
        }
    }
}

impl Fetch for Mock {
    fn get(&mut self, url: &str) -> Result<Fetched> {
        self.page(url)
    }

    fn post(&mut self, url: &str, body: &[u8], referer: &str) -> Result<Fetched> {
        self.posted.push((url.to_string(), body.to_vec(), referer.to_string()));
        self.page(url)
    }
}

/// CGI の出力 (Python 版の print したもの) にする
fn cgi_output(r: &Reply) -> String {
    let body = String::from_utf8(r.body.clone()).unwrap();
    if r.status == 200 {
        format!("Content-Type: {}\n\n{}", r.content_type, body)
    } else {
        format!("Status: {} Bad Request\nContent-Type: {}\n\n{}", r.status, r.content_type, body)
    }
}

#[test]
fn same_as_python() {
    for c in fixtures::CASES {
        let mut m = Mock { pages: c.pages, posted: Vec::new() };
        let r = handle(c.script, c.query.as_bytes(), &mut m);
        match r {
            Ok(reply) => {
                assert_eq!(c.code, 0, "{}: Python 版は落ちた", c.name);
                assert_eq!(cgi_output(&reply), c.out, "{}", c.name);
            }
            Err(e) => assert_eq!(c.code, 500, "{}: {:?}", c.name, e),
        }
        let posted: Vec<(String, Vec<u8>, String)> = c.posted.iter().map(|(u, b, r)| (u.to_string(), b.to_vec(), r.to_string())).collect();
        assert_eq!(m.posted, posted, "{}", c.name);
    }
}

// もとの tests/bbs_reader_test.py (引数の検査)

#[test]
fn check_params_valid() {
    assert_eq!(check_params("5ch.net", "news4vip", "", None), None);
    assert_eq!(check_params("jbbs.shitaraba.net", "game", "12345", Some("1234567890")), None);
    assert_eq!(check_params("example.com:8080", "a_b-c.d", "", None), None);
}

#[test]
fn check_params_bad() {
    for fqdn in [
        "",
        "127.0.0.1:7144/admin?cmd=shutdown#",
        "user@evil.example",
        "evil.example/",
        "evil.example?x",
        "evil.example#",
        "a b",
        "[::1]",
        "-a.example",
        "a.example-",
        "a.example:99999999",
        "a.example\r\nX: y",
    ] {
        assert_eq!(check_params(fqdn, "news", "", None), Some("bad fqdn"), "{:?}", fqdn);
    }
    let long = "x".repeat(65);
    for category in ["", ".", "..", "a/b", "a?b", "a#b", "a b", "../../x", long.as_str()] {
        assert_eq!(check_params("5ch.net", category, "", None), Some("bad category"), "{:?}", category);
    }
    assert_eq!(check_params("5ch.net", "x", "1/2", None), Some("bad board_num"));
    assert_eq!(check_params("5ch.net", "x", "abc", None), Some("bad board_num"));
    assert_eq!(check_params("5ch.net", "x", "", Some("12/34")), Some("bad id"));
    assert_eq!(check_params("5ch.net", "x", "", Some("")), Some("bad id"));
    assert!(Board::new("127.0.0.1:7144/admin?cmd=shutdown#", "x", "").is_err());
}

#[test]
fn settings_parser() {
    let s = parse_settings("A=1\r\nb : two\n# c\n\n; d\nC=x=y\n  cont\n").unwrap();
    assert_eq!(s, vec![("a".to_string(), "1".to_string()), ("b".to_string(), "two".to_string()), ("c".to_string(), "x=y\ncont".to_string())]);
    assert!(parse_settings("no delimiter\n").is_err());
    assert!(parse_settings("A=1\na=2\n").is_err());
}

#[test]
fn form() {
    let f = Form::parse(b"a=1&b=&c&a=2&d=%E3%81%82+%zz&%61=x");
    assert_eq!(f.get("a"), Some("1"));
    assert_eq!(f.get("b"), None);
    assert_eq!(f.get("c"), None);
    assert_eq!(f.get("d"), Some("あ %zz"));
}

/// `python_cases.json` (もとの Python の変換の結果、`data/README.md`) があれば、全部と比べる
#[test]
fn python_cases_if_available() {
    let path = match std::env::var("PEERCAST_PY_CASES") {
        Ok(p) => p,
        Err(_) => return,
    };
    let data = std::fs::read(&path).unwrap();
    let cases = match crate::json::parse(&data).unwrap() {
        crate::json::Value::Array(a) => a,
        _ => panic!("not an array"),
    };
    let st = |v: &crate::json::Value, k: &str| -> Option<String> {
        match v.get(k.as_bytes()) {
            Some(crate::json::Value::Str(s)) => Some(String::from_utf8(s.clone()).unwrap()),
            _ => None,
        }
    };
    let hex = |s: &str| -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() };
    let codec_of = |s: &str| match s {
        "cp932" => codec::Codec::Cp932,
        "euc_jp" => codec::Codec::EucJp,
        _ => codec::Codec::ShiftJis,
    };
    let mut n = 0;
    for c in &cases {
        let op = st(c, "op").unwrap();
        let input = st(c, "in").unwrap();
        match op.as_str() {
            "dec" => {
                let codec = codec_of(&st(c, "codec").unwrap());
                let b = hex(&input);
                assert_eq!(codec::decode(codec, &b, codec::Errors::Strict), st(c, "strict"), "{} {}", input, st(c, "codec").unwrap());
                assert_eq!(codec::decode(codec, &b, codec::Errors::Replace), st(c, "replace"), "{}", input);
            }
            "enc" => {
                let codec = codec_of(&st(c, "codec").unwrap());
                assert_eq!(codec::encode_xmlcharref(codec, &input), hex(&st(c, "out").unwrap()), "{:?}", input);
            }
            "unescape" => assert_eq!(py::html_unescape(&input), st(c, "out").unwrap(), "{:?}", input),
            "splitlines" => {
                let want: Vec<String> = match c.get(b"out") {
                    Some(crate::json::Value::Array(a)) => a.iter().map(|x| String::from_utf8(x.as_string().unwrap().to_vec()).unwrap()).collect(),
                    _ => panic!(),
                };
                assert_eq!(py::splitlines(&input), want, "{:?}", input);
            }
            "int" => {
                let want = c.get(b"out").and_then(|x| x.as_int().ok()).map(|x| x as i64);
                assert_eq!(py::int(&input), want, "{:?}", input);
            }
            _ => panic!("unknown op {}", op),
        }
        n += 1;
    }
    eprintln!("python cases: {}", n);
}
