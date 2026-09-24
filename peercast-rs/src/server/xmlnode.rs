//! XML の書き出し (core/common/xml.cpp の `XML::Node` を書式付きの文字列から作って `write` する部分)。
//!
//! C++ 版は `XML::Node("tag a=\"%s\"", ...)` のように書式付きの文字列からノードを作り、その文字列を
//! `setAttributes` (段階 3c の `crate::xml::parse_attributes`) で名前と値に分けてから書き出す。
//! 書き出しも、分けた名前と値を使う (値に `"` などがあると元の文字列とは変わる)。

use super::error::{Error, Result};

/// `XML::Node`
#[derive(Clone, Debug, Default)]
pub struct XmlNode {
    /// タグ名と属性 (`XML::Node` のコンストラクターに渡す文字列)
    pub attrs: Vec<u8>,
    pub content: Option<Vec<u8>>,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    /// `XML::Node(fmt, ...)` (書式を展開したあとの文字列。8191 バイトまで)
    pub fn new(attrs: impl AsRef<[u8]>) -> XmlNode {
        let a = attrs.as_ref();
        XmlNode { attrs: a[..a.len().min(8191)].to_vec(), content: None, children: Vec::new() }
    }

    /// `add` (最後の子として加える)
    pub fn add(&mut self, n: XmlNode) {
        self.children.push(n);
    }

    /// `setContent`
    pub fn set_content(&mut self, c: &[u8]) {
        self.content = Some(c[..c.iter().position(|&x| x == 0).unwrap_or(c.len())].to_vec());
    }

    /// `write`: ノードとその子を書き出す
    pub fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        let a = crate::xml::parse_attributes(&self.attrs).map_err(|e| {
            Error::stream(match e {
                crate::xml::AttrError::TooMany => "Too many attributes",
                crate::xml::AttrError::BadValue => "Bad tag value",
            })
        })?;
        let cstr = |pos: usize| {
            let s = &a.data[pos..];
            &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
        };
        let name = cstr(0);
        out.push(b'<');
        out.extend_from_slice(name);
        for &(n, v) in &a.attrs[1..] {
            out.push(b' ');
            out.extend_from_slice(cstr(n));
            out.extend_from_slice(b"=\"");
            out.extend_from_slice(cstr(v));
            out.push(b'"');
        }
        if self.content.is_none() && self.children.is_empty() {
            out.extend_from_slice(b"/>\n");
        } else {
            out.extend_from_slice(b">\n");
            if let Some(c) = &self.content {
                out.extend_from_slice(c);
            }
            for c in &self.children {
                c.write(out)?;
            }
            out.extend_from_slice(b"</");
            out.extend_from_slice(name);
            out.extend_from_slice(b">\n");
        }
        Ok(())
    }

    /// `XML::write`: 宣言と根
    pub fn write_document(&self) -> Result<Vec<u8>> {
        let mut out = b"<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n".to_vec();
        self.write(&mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write() {
        let mut root = XmlNode::new("peercast");
        let mut c = XmlNode::new("channel name=\"a\" id=\"b\"");
        c.add(XmlNode::new("track title=\"t\""));
        root.add(c);
        root.add(XmlNode::new("servent agent=\"PeerCast\" "));
        assert_eq!(
            String::from_utf8(root.write_document().unwrap()).unwrap(),
            "<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n<peercast>\n<channel name=\"a\" id=\"b\">\n<track title=\"t\"/>\n</channel>\n<servent agent=\"PeerCast\"/>\n</peercast>\n"
        );
        assert!(XmlNode::new("a b=c").write(&mut Vec::new()).is_err());
    }
}
