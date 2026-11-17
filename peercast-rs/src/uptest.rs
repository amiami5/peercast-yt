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
    } else if scheme != b"http" && scheme != b"https" {
        Err(b"unsupported protocol")
    } else if existing.contains(&url) {
        Err(b"URL already exists")
    } else {
        Ok(())
    }
}

fn find_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

fn rfind_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).rposition(|w| w.eq_ignore_ascii_case(needle))
}

fn strip_html_tags(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut in_tag = false;
    for &b in s {
        if b == b'<' {
            in_tag = true;
        } else if b == b'>' {
            in_tag = false;
        } else if !in_tag {
            out.push(b);
            if out.len() >= 256 {
                break;
            }
        }
    }
    out
}

fn extract_line_after(section: &[u8], label: &[u8]) -> Option<Vec<u8>> {
    let pos = find_ascii_ci(section, label)?;
    let mut rest = &section[pos + label.len()..];
    rest = &rest[..rest.len().min(512)];
    if let Some(colon) = rest.iter().take(16).position(|&c| c == b':') {
        rest = &rest[colon + 1..];
    }
    let end = [b"<br".as_slice(), b"</li>", b"</div>"]
        .iter()
        .filter_map(|tag| find_ascii_ci(rest, tag))
        .min()
        .unwrap_or(rest.len());
    Some(strip_html_tags(&rest[..end]))
}

fn has_ascii_word(haystack: &[u8], word: &[u8]) -> bool {
    let mut i = 0;
    while i + word.len() <= haystack.len() {
        if let Some(rel) = find_ascii_ci(&haystack[i..], word) {
            let start = i + rel;
            let end = start + word.len();
            let left_ok = start == 0 || !haystack[start - 1].is_ascii_alphanumeric();
            let right_ok = end == haystack.len() || !haystack[end].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
            i = start + 1;
        } else {
            break;
        }
    }
    false
}

fn parse_bitrate_text(text: &[u8]) -> (Vec<u8>, bool) {
    let over = has_ascii_word(text, b"over");
    let Some(kbps_pos) = find_ascii_ci(text, b"kbps") else {
        return (b"0".to_vec(), false);
    };
    let mut end = kbps_pos;
    while end > 0 && text[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && text[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let digits = &text[start..end];
    if digits.is_empty() || digits.len() > 10 {
        (b"0".to_vec(), false)
    } else {
        (digits.to_vec(), over)
    }
}

/// SP 現行 HTML (`index.php` の `<div class="setting">` や `result.php` の `Uptest result`) から
/// 帯域情報を読む。チャンネル一覧の文字列による偽装を防ぐため、最後の設定欄/結果欄だけを見る。
/// 接続先の `addr`・`port`・`object`・`post_size` は HTML からは読まず、呼び出し元が信頼された
/// URL から決めた値を使う。
pub fn read_sp_html(body: &[u8], addr: &[u8], port: u16, object: &[u8]) -> Result<Info, ReadError> {
    let (start, end_tag) = if let Some(p) = rfind_ascii_ci(body, b"<div class=\"setting\"") {
        (p, b"</div>".as_slice())
    } else if let Some(p) = rfind_ascii_ci(body, b"<h2>uptest result</h2>") {
        (p, b"<div class=\"powered\"".as_slice())
    } else {
        return Err(ReadError::Null);
    };
    let tail = &body[start..body.len().min(start + 4096)];
    let section_end = find_ascii_ci(tail, end_tag).unwrap_or(tail.len());
    let section = &tail[..section_end];

    let bitrate_text = extract_line_after(section, b"Max playable bitrate").ok_or(ReadError::Null)?;
    let (speed, over) = parse_bitrate_text(&bitrate_text);
    let port_open = extract_line_after(section, b"Port check")
        .is_some_and(|t| has_ascii_word(&t, b"pass"));

    let mut info: Info = Default::default();
    info[0] = if addr.eq_ignore_ascii_case(b"bayonet.ddo.jp") { b"SP".to_vec() } else { addr.to_vec() };
    info[2] = if port_open { b"1".to_vec() } else { b"0".to_vec() };
    info[3] = speed;
    info[4] = if over { b"1".to_vec() } else { b"0".to_vec() };
    info[5] = b"1".to_vec(); // checkable
    info[6] = b"0".to_vec(); // remain
    info[7] = addr.to_vec();
    info[8] = port.to_string().into_bytes();
    info[9] = object.to_vec();
    info[10] = b"250".to_vec(); // post_size (250,000 bytes)
    info[11] = b"2000".to_vec(); // limit
    info[12] = b"15".to_vec(); // interval
    info[13] = b"1".to_vec(); // enabled
    Ok(info)
}

/// SP HTML 方式で POST する固定パスを、登録された URL のパスだけから導出する
/// (HTML 内の `<form action>` は使わない)。
pub fn sp_post_object(path: &[u8]) -> Vec<u8> {
    let mut segs: Vec<&[u8]> = path
        .split(|&c| c == b'/')
        .filter(|s| !s.is_empty() && *s != b"." && *s != b"..")
        .collect();
    if segs.last().is_some_and(|s| s.contains(&b'.')) {
        segs.pop();
    }
    if segs.last().is_some_and(|s| s.eq_ignore_ascii_case(b"uptest")) {
        segs.pop();
    }
    let mut out = Vec::new();
    for s in segs {
        out.push(b'/');
        out.extend_from_slice(s);
    }
    out.extend_from_slice(b"/uptest.cgi");
    out
}

/// 旧既定 URL (`http(s)://bayonet.ddo.jp/sp/yp4g.xml`) かどうか
pub fn is_legacy_sp_yp4g_url(host: &[u8], path: &[u8]) -> bool {
    host.eq_ignore_ascii_case(b"bayonet.ddo.jp") && path.eq_ignore_ascii_case(b"sp/yp4g.xml")
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
        assert_eq!(check_add_url(true, b"ftp", b"x", &ex), Err(&b"unsupported protocol"[..]));
        assert_eq!(check_add_url(true, b"https", b"https://b/", &ex), Ok(()));
        assert_eq!(check_add_url(true, b"http", b"http://a/", &ex), Err(&b"URL already exists"[..]));
        assert_eq!(check_add_url(true, b"http", b"http://b/", &ex), Ok(()));
    }

    #[test]
    fn sp_html_and_paths() {
        assert_eq!(sp_post_object(b""), b"/uptest.cgi");
        assert_eq!(sp_post_object(b"index.php"), b"/uptest.cgi");
        assert_eq!(sp_post_object(b"sp"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/index.php"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/result.php"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/uptest/"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/uptest/uptest_js.php"), b"/sp/uptest.cgi");
        assert_eq!(sp_post_object(b"sp/../admin/index.php"), b"/sp/admin/uptest.cgi");

        assert!(is_legacy_sp_yp4g_url(b"bayonet.ddo.jp", b"sp/yp4g.xml"));
        assert!(is_legacy_sp_yp4g_url(b"Bayonet.DDO.jp", b"sp/yp4g.xml"));
        assert!(!is_legacy_sp_yp4g_url(b"bayonet.ddo.jp", b"sp/index.php"));

        let html = br#"<html><body>
            <div>Channel comment: <div class="setting">Max playable bitrate : 99999 kbps</div></div>
            <table class="chlist"><tr><td>Max playable bitrate : 88888 kbps</td></tr></table>
            <div class="setting"><a href="?mode=setting">Settings</a><br />Use Port : 7144<br />
            Port check : <span class="pass">Pass</span><br />
            Max playable bitrate : <span class="fast">2000 kbps over</span><br />
            Generate time : 0 msec</div></body>"#;
        let info = read_sp_html(html, b"bayonet.ddo.jp", 443, b"/sp/uptest.cgi").unwrap();
        assert_eq!(info[0], b"SP");
        assert_eq!(info[2], b"1");
        assert_eq!(info[3], b"2000");
        assert_eq!(info[4], b"1");
        assert_eq!(info[5], b"1");
        assert_eq!(info[7], b"bayonet.ddo.jp");
        assert_eq!(info[8], b"443");
        assert_eq!(info[9], b"/sp/uptest.cgi");
        assert_eq!(info[10], b"250");

        let slow = br#"<div class="setting">Port check : <span class="fail">Fail</span><br />
            Max playable bitrate : <span class="slow">1234 kbps</span></div>"#;
        let info2 = read_sp_html(slow, b"127.0.0.1", 8080, b"/uptest.cgi").unwrap();
        assert_eq!(info2[0], b"127.0.0.1");
        assert_eq!(info2[2], b"0");
        assert_eq!(info2[3], b"1234");
        assert_eq!(info2[4], b"0");

        let untested = br#"<div class="setting">Max playable bitrate : <span>Untested</span></div>"#;
        let info3 = read_sp_html(untested, b"bayonet.ddo.jp", 443, b"/sp/uptest.cgi").unwrap();
        assert_eq!(info3[3], b"0");
        assert_eq!(info3[4], b"0");

        let result_page = br#"<div class="header"><h2>Uptest result</h2></div>
            <div><ul><li>Max playable bitrate : <span class="fast">2000 kbps over</span></li></ul></div>
            <div class="powered">Powered by YP4G</div>"#;
        let info4 = read_sp_html(result_page, b"bayonet.ddo.jp", 443, b"/sp/uptest.cgi").unwrap();
        assert_eq!(info4[3], b"2000");
        assert_eq!(info4[4], b"1");

        assert_eq!(read_sp_html(b"<html><body>Not found</body></html>", b"a", 80, b"/uptest.cgi"), Err(ReadError::Null));
    }
}
