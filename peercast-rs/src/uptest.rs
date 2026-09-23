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
    /// `XML::read` の失敗 (`Callback` にはならない)
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
    attr_error: Option<AttrError>,
}

/// `XML::Node::Node("%s", tag)` の書き込み先の大きさ
const NODE_TMP_LEN: usize = 8192;

impl xml::Builder for Tree {
    fn content(&mut self, _s: &[u8]) -> Result<(), ()> {
        Ok(())
    }

    fn start_tag(&mut self, s: &[u8], single: bool) -> Result<(), ()> {
        let tag = &s[..s.len().min(NODE_TMP_LEN - 1)];
        let attrs = xml::parse_attributes(tag).map_err(|e| self.attr_error = Some(e))?;
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

    fn end_tag(&mut self) -> Result<(), ()> {
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
        return Err(match (e, tree.attr_error) {
            (xml::Error::Callback, Some(a)) => ReadError::Attr(a),
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

/// `UptestInfo::postURL`
pub fn post_url(addr: &[u8], port: &[u8], object: &[u8]) -> Vec<u8> {
    [&b"http://"[..], c_str(addr), b":", c_str(port), c_str(object)].concat()
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
