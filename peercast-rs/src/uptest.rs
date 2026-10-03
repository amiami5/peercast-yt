//! 帯域測定 (core/common/uptest.cpp) のうち、通信しない部分。
//!
//! yp4g.xml の読み取り (`UptestEndpoint::readInfo`)、`UptestInfo::postURL`、`isReady`、
//! 状態の文字列、URL を加えてよいかの判断。ダウンロードと POST、ロックは C++ 側。

use crate::reader::SliceReader;
use crate::xml::{self, AttrError, Attributes};

/// `UptestInfo` の欄の数と並び
pub const FIELDS: [&str; 14] = [
    "name", "ip", "port_open", "speed", "over", "checkable", "remain", "addr", "port", "object", "post_size",
    "limit", "interval", "enabled",
];

/// `readInfo` がどのノードのどの属性を読むか (`FIELDS` の順)
const STEPS: [(&[u8], &[&[u8]]); 4] = [
    (b"yp", &[b"name"]),
    (b"host", &[b"ip", b"port_open", b"speed", b"over"]),
    (b"uptest", &[b"checkable", b"remain"]),
    (b"uptest_srv", &[b"addr", b"port", b"object", b"post_size", b"limit", b"interval", b"enabled"]),
];

/// `UptestInfo` (`FIELDS` の順)
pub type Info = [Vec<u8>; 14];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// `XML::read` の失敗 (`Attr` にはならない)
    Xml(xml::Error),
    /// タグの属性が読めない ("Too many attributes" か "Bad tag value")
    Attr(AttrError),
    /// ノードか属性がない (C++ 版の `nonNull` の失敗)
    Null,
}

fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `XML::Node` のうち、ここで使うもの
struct Node {
    attrs: Attributes,
    children: Vec<usize>,
    parent: Option<usize>,
}

impl Node {
    fn at(&self, pos: usize) -> &[u8] {
        c_str(self.attrs.data.get(pos..).unwrap_or(&[]))
    }

    /// `getName`
    fn name(&self) -> &[u8] {
        self.at(self.attrs.attrs[0].0)
    }

    /// `findAttr`: 名前の先頭が `name` と一致する (大文字小文字は問わない) 最初の属性の値
    fn find_attr(&self, name: &[u8]) -> Option<&[u8]> {
        self.attrs.attrs[1..].iter().find_map(|&(n, v)| {
            let an = self.at(n);
            (an.len() >= name.len() && an[..name.len()].eq_ignore_ascii_case(name)).then(|| self.at(v))
        })
    }
}

/// `XML` の木 (rustbridge.h の `XmlReader` が組み立てるものと同じ形)
#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    root: Option<usize>,
    curr: Option<usize>,
}

/// `XML::Node::Node("%s", tag)` の書き込み先の大きさ
const NODE_TMP_LEN: usize = 8192;

impl xml::Builder for Tree {
    fn content(&mut self, _s: &[u8]) -> Result<(), AttrError> {
        Ok(())
    }

    fn start_tag(&mut self, s: &[u8], single: bool) -> Result<(), AttrError> {
        let tag = &s[..s.len().min(NODE_TMP_LEN - 1)];
        let attrs = xml::parse_attributes(tag)?;
        let i = self.nodes.len();
        self.nodes.push(Node { attrs, children: Vec::new(), parent: self.curr });
        match self.curr {
            Some(p) => self.nodes[p].children.push(i),
            None => self.root = Some(i),
        }
        if !single {
            self.curr = Some(i);
        }
        Ok(())
    }

    fn end_tag(&mut self) -> Result<(), AttrError> {
        // Rust の xml::read は開いているノードがあるときだけ呼ぶ
        self.curr = self.curr.and_then(|c| self.nodes[c].parent);
        Ok(())
    }
}

impl Tree {
    /// `XML::findNode`: 木を先に親、次に子の順でたどり、名前が一致する (大文字小文字は
    /// 問わない) 最初のノード
    fn find_node(&self, name: &[u8]) -> Option<&Node> {
        let mut stack: Vec<usize> = self.root.into_iter().collect();
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            if n.name().eq_ignore_ascii_case(name) {
                return Some(n);
            }
            stack.extend(n.children.iter().rev());
        }
        None
    }
}

/// `UptestEndpoint::readInfo`
pub fn read_info(body: &[u8]) -> Result<Info, ReadError> {
    let mut tree = Tree::default();
    let mut r = SliceReader { data: body, pos: 0 };
    if let Err(e) = xml::read(&mut r, &mut tree) {
        return Err(match e {
            xml::Error::Attr(a) => ReadError::Attr(a),
            _ => ReadError::Xml(e),
        });
    }

    let mut info: Info = Default::default();
    let mut k = 0;
    for (node, attrs) in STEPS {
        let n = tree.find_node(node).ok_or(ReadError::Null)?;
        for a in attrs {
            info[k] = n.find_attr(a).ok_or(ReadError::Null)?.to_vec();
            k += 1;
        }
    }
    Ok(info)
}

/// `UptestInfo::postURL`。IPv6 アドレスは `[]` で囲む
pub fn post_url(addr: &[u8], port: &[u8], object: &[u8]) -> Vec<u8> {
    [&b"http://"[..], &url_host(c_str(addr)), b":", c_str(port), c_str(object)].concat()
}

/// `[]` で囲んだものも含めて、IPv6 アドレスなら囲まずに返す
pub fn ipv6_literal(addr: &[u8]) -> Option<&[u8]> {
    let inner = match addr {
        [b'[', inner @ .., b']'] => inner,
        _ => addr,
    };
    let ok = std::str::from_utf8(inner).ok()?.parse::<std::net::Ipv6Addr>().is_ok();
    ok.then_some(inner)
}

/// URL や Host ヘッダーに書くときのホスト (IPv6 アドレスは `[]` で囲む)
pub fn url_host(addr: &[u8]) -> Vec<u8> {
    match ipv6_literal(addr) {
        Some(v6) => [&b"["[..], v6, b"]"].concat(),
        None => addr.to_vec(),
    }
}

/// `uptest_srv` の `addr`・`port`・`object` を、POST の宛先とパスに使ってよいか確かめ、ポートを返す。
/// yp4g.xml は平文の HTTP で取るので書き換えられうる。改行などで要求を書き足されないよう、
/// `addr` はホスト名か IPv4 アドレスに使う文字だけか、IPv6 アドレス (`[]` で囲んでも、囲まなくても
/// よい。スコープの `%` は付けられない)、`object` は `/` で始まる、空白・制御文字・`"` などの
/// ない URL のパスだけにする (security-review #29)
pub fn check_srv(addr: &[u8], port: &[u8], object: &[u8]) -> Result<u16, &'static str> {
    let name_ok = !addr.is_empty() && addr.len() <= 253 && addr.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-');
    if !name_ok && ipv6_literal(addr).is_none() {
        return Err("invalid addr");
    }
    let port = match port {
        p if !p.is_empty() && p.len() <= 5 && p.iter().all(u8::is_ascii_digit) => {
            p.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32)
        }
        _ => return Err("invalid port"),
    };
    if !(1..=65535).contains(&port) {
        return Err("invalid port");
    }
    let path_ok = |c: u8| (0x21..=0x7e).contains(&c) && !b"\"<>\\^`{|}#".contains(&c);
    if object.first() != Some(&b'/') || object.len() > 1024 || !object.iter().all(|&c| path_ok(c)) {
        return Err("invalid object");
    }
    Ok(port as u16)
}

/// `post_size` (KB) の上限。正当な値は 250 ほど
pub const MAX_POST_SIZE_KB: i32 = 10_000;

/// `post_size` の値から送るバイト数。上限を超えるものは誤り (C++ 版は確かめずに確保する)
pub fn post_size(v: &[u8]) -> Result<usize, &'static str> {
    match crate::http::atoi(c_str(v)) {
        n if n > MAX_POST_SIZE_KB => Err("post_size too large"),
        n => Ok(n.max(0) as usize * 1000),
    }
}

/// `UptestEndpoint::Status`
pub const UNTRIED: i32 = 0;
pub const SUCCESS: i32 = 1;
pub const ERROR: i32 = 2;

/// `UptestEndpoint::kXmlTryInterval`
pub const XML_TRY_INTERVAL: u32 = 60;

/// `UptestEndpoint::isReady`
pub fn is_ready(status: i32, last_tried_at: u32, now: u32) -> bool {
    status == UNTRIED || now.wrapping_sub(last_tried_at) > XML_TRY_INTERVAL
}

/// `textStatus` (C++ 版は知らない値で abort する)
pub fn text_status(status: i32) -> Option<&'static [u8]> {
    match status {
        UNTRIED => Some(b"Untried"),
        SUCCESS => Some(b"Success"),
        ERROR => Some(b"Error"),
        _ => None,
    }
}

/// `UptestServiceRegistry::addURL` の判断。`valid` と `scheme` は `URI` の結果。
pub fn check_add_url(valid: bool, scheme: &[u8], url: &[u8], existing: &[&[u8]]) -> Result<(), &'static [u8]> {
    if !valid {
        Err(b"invalid URL")
    } else if scheme != b"http" {
        Err(b"unsupported protocol")
    } else if existing.contains(&url) {
        Err(b"URL already exists")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &[u8] = b"<yp4g><yp name=\"YP\"/>\
        <host ip=\"192.168.0.1\" port_open=\"1\" speed=\"999\" over=\"0\"/>\
        <uptest checkable=\"1\" remain=\"0\"/>\
        <uptest_srv addr=\"example.com\" port=\"443\" object=\"/yp/uptest.cgi\" post_size=\"250\" limit=\"3000\" interval=\"15\" enabled=\"1\"/>\
        </yp4g>";

    #[test]
    fn read_info_ok() {
        let info = read_info(DOC).unwrap();
        let want: [&[u8]; 14] = [
            b"YP", b"192.168.0.1", b"1", b"999", b"0", b"1", b"0", b"example.com", b"443", b"/yp/uptest.cgi",
            b"250", b"3000", b"15", b"1",
        ];
        for (a, b) in info.iter().zip(want) {
            assert_eq!(a.as_slice(), b);
        }
        assert_eq!(post_url(&info[7], &info[8], &info[9]), b"http://example.com:443/yp/uptest.cgi");
    }

    #[test]
    fn read_info_quirks() {
        // findAttr は名前の先頭が一致すればよく、findNode は大文字小文字を問わない
        let doc = b"<YP nameX=\"a\"/><x><HOST ipv6=\"i\" port_openz=\"p\" speed=\"s\" over=\"o\"/></x>\
            <uptest checkable=\"c\" remain=\"r\"/><uptest_srv addr=\"a\" port_x=\"1\" object=\"\" post_size=\"2\" \
            limit=\"3\" interval=\"4\" enabled=\"5\"/>";
        // 根は最後の最上位のノード (uptest_srv) だけなので、yp は見つからない
        assert_eq!(read_info(doc), Err(ReadError::Null));
        let doc2 = [&b"<r>"[..], doc, b"</r>"].concat();
        let info = read_info(&doc2).unwrap();
        assert_eq!(info[0], b"a");
        assert_eq!(info[1], b"i");
        assert_eq!(info[8], b"1");
        assert_eq!(info[9], b"");
    }

    #[test]
    fn read_info_errors() {
        assert_eq!(read_info(b""), Err(ReadError::Null));
        assert_eq!(read_info(b"</a>"), Err(ReadError::Xml(xml::Error::UnexpectedEndTag)));
        assert_eq!(read_info(b"<?foo?>"), Err(ReadError::Xml(xml::Error::NotXml)));
        assert!(matches!(read_info(b"<a b=c>"), Err(ReadError::Attr(_))));
    }

    /// 変異させた文書でパニックしないこと
    #[test]
    fn fuzz_no_panic() {
        let mut rng = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..20000 {
            let mut d = DOC.to_vec();
            for _ in 0..(next() % 8) {
                let p = (next() as usize) % d.len();
                match next() % 3 {
                    0 => d[p] = next() as u8,
                    1 => d.truncate(p),
                    _ => d.insert(p, [b'<', b'>', b'/', b'=', b'"', 0][(next() % 6) as usize]),
                }
                if d.is_empty() {
                    break;
                }
            }
            let _ = read_info(&d);
        }
        let _ = read_info(&vec![b'a'; 9000]);
        let _ = read_info(&[&b"<"[..], &vec![b'a'; 9000], b" x=\"1\"/>"].concat());
    }

    #[test]
    fn srv_checks() {
        assert_eq!(check_srv(b"example.com", b"443", b"/yp/uptest.cgi?a=1&b=%20"), Ok(443));
        assert_eq!(check_srv(b"192.168.0.1", b"1", b"/"), Ok(1));
        assert_eq!(check_srv(b"a", b"65535", b"/x"), Ok(65535));
        // IPv6 アドレスは [] で囲んでも囲まなくてもよい
        for addr in [&b"::1"[..], b"[::1]", b"2001:db8::1", b"[2001:db8::1]", b"::ffff:127.0.0.1"] {
            assert_eq!(check_srv(addr, b"80", b"/"), Ok(80), "{:?}", addr);
        }
        assert_eq!(url_host(b"::1"), b"[::1]");
        assert_eq!(url_host(b"[::1]"), b"[::1]");
        assert_eq!(url_host(b"example.com"), b"example.com");
        assert_eq!(post_url(b"2001:db8::1", b"80", b"/x"), b"http://[2001:db8::1]:80/x");
        for addr in [
            &b""[..], b"a b", b"a\r\nb", b"a/b", b"a:1", b"a@b", b"a\0", b"a\"b", b"[a]", b"[::1", b"::1]", b"[::1]:80",
            b"fe80::1%eth0", b"[fe80::1%25eth0]", b"::1\r\nX: y", b"[[::1]]", b":::1",
        ] {
            assert_eq!(check_srv(addr, b"80", b"/"), Err("invalid addr"), "{:?}", addr);
        }
        assert_eq!(check_srv(&[b'a'; 254], b"80", b"/"), Err("invalid addr"));
        for port in [&b""[..], b"0", b"65536", b"99999", b"123456", b"-1", b"+80", b" 80", b"80\r\n", b"0x50"] {
            assert_eq!(check_srv(b"a", port, b"/"), Err("invalid port"), "{:?}", port);
        }
        for object in [&b""[..], b"x", b"/a b", b"/a\r\nHost: x", b"/a\n", b"/a\0", b"/\"", b"/<", b"/#", b"/\x7f", b"/\x80"] {
            assert_eq!(check_srv(b"a", b"80", object), Err("invalid object"), "{:?}", object);
        }
        assert_eq!(check_srv(b"a", b"80", &[&b"/"[..], &[b'a'; 1024]].concat()), Err("invalid object"));
    }

    #[test]
    fn post_sizes() {
        assert_eq!(post_size(b"250"), Ok(250_000));
        assert_eq!(post_size(b"10000"), Ok(10_000_000));
        assert_eq!(post_size(b"10001"), Err("post_size too large"));
        assert_eq!(post_size(b"99999999999"), Err("post_size too large"));
        assert_eq!(post_size(b"-5"), Ok(0));
        assert_eq!(post_size(b""), Ok(0));
    }

    #[test]
    fn misc() {
        assert!(is_ready(UNTRIED, 0, 0));
        assert!(!is_ready(ERROR, 0, 0));
        assert!(!is_ready(SUCCESS, 100, 160));
        assert!(is_ready(SUCCESS, 100, 161));
        assert!(is_ready(ERROR, 100, 50)); // 符号なしの引き算
        assert_eq!(text_status(2), Some(&b"Error"[..]));
        assert_eq!(text_status(3), None);
        let ex: [&[u8]; 1] = [b"http://a/"];
        assert_eq!(check_add_url(false, b"http", b"x", &ex), Err(&b"invalid URL"[..]));
        assert_eq!(check_add_url(true, b"https", b"x", &ex), Err(&b"unsupported protocol"[..]));
        assert_eq!(check_add_url(true, b"http", b"http://a/", &ex), Err(&b"URL already exists"[..]));
        assert_eq!(check_add_url(true, b"http", b"http://b/", &ex), Ok(()));
    }
}
