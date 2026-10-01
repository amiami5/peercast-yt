//! XML の書き出し (core/common/xml.cpp の `XML::Node` を作って `write` する部分)。
//!
//! C++ 版は `XML::Node("tag a=\"%s\"", ...)` のように書式付きの文字列からノードを作り、その文字列を
//! 名前と値に分けてから書き出す。値に `"` などが入ると、属性や要素を書き足されたり、分けられずに
//! 書き出せなくなったりする。ここでは名前と生の値の組で持ち、書き出すときに値をエスケープする。

/// `XML::Node`
#[derive(Clone, Debug, Default)]
pub struct XmlNode {
    pub name: &'static str,
    /// 属性の名前と生の値 (エスケープする前のもの)
    pub attrs: Vec<(&'static str, Vec<u8>)>,
    pub children: Vec<XmlNode>,
}

/// 属性の値を書く。`&` `<` `>` `"` `'` を実体参照にし、制御文字 (XML 1.0 に書けない) を除く。
/// 正しい UTF-8 でないところは U+FFFD にする (宣言の encoding="utf-8" と合わせる)。
fn write_escaped(out: &mut Vec<u8>, v: &[u8]) {
    for ch in String::from_utf8_lossy(v).chars() {
        match ch {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '"' => out.extend_from_slice(b"&quot;"),
            '\'' => out.extend_from_slice(b"&#039;"),
            c if c < ' ' || c == '\x7f' => {}
            c => {
                let mut b = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
}

impl XmlNode {
    pub fn new(name: &'static str) -> XmlNode {
        XmlNode { name, attrs: Vec::new(), children: Vec::new() }
    }

    /// 属性を加える
    pub fn attr(mut self, name: &'static str, value: impl AsRef<[u8]>) -> XmlNode {
        self.attrs.push((name, value.as_ref().to_vec()));
        self
    }

    /// `add` (最後の子として加える)
    pub fn add(&mut self, n: XmlNode) {
        self.children.push(n);
    }

    /// `write`: ノードとその子を書き出す
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(b'<');
        out.extend_from_slice(self.name.as_bytes());
        for (n, v) in &self.attrs {
            out.push(b' ');
            out.extend_from_slice(n.as_bytes());
            out.extend_from_slice(b"=\"");
            write_escaped(out, v);
            out.push(b'"');
        }
        if self.children.is_empty() {
            out.extend_from_slice(b"/>\n");
        } else {
            out.extend_from_slice(b">\n");
            for c in &self.children {
                c.write(out);
            }
            out.extend_from_slice(b"</");
            out.extend_from_slice(self.name.as_bytes());
            out.extend_from_slice(b">\n");
        }
    }

    /// `XML::write`: 宣言と根
    pub fn write_document(&self) -> Vec<u8> {
        let mut out = b"<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n".to_vec();
        self.write(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write() {
        let mut root = XmlNode::new("peercast");
        let mut c = XmlNode::new("channel").attr("name", "a").attr("id", "b");
        c.add(XmlNode::new("track").attr("title", "t"));
        root.add(c);
        root.add(XmlNode::new("servent").attr("agent", "PeerCast"));
        assert_eq!(
            String::from_utf8(root.write_document()).unwrap(),
            "<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n<peercast>\n<channel name=\"a\" id=\"b\">\n<track title=\"t\"/>\n</channel>\n<servent agent=\"PeerCast\"/>\n</peercast>\n"
        );
    }

    #[test]
    fn values_are_escaped() {
        let mut out = Vec::new();
        XmlNode::new("channel").attr("type", "FLV\" x=\"<script>").attr("name", "a&'b\r\n\x01日本\x7f").attr("desc", b"\xff").write(&mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "<channel type=\"FLV&quot; x=&quot;&lt;script&gt;\" name=\"a&amp;&#039;b日本\" desc=\"\u{fffd}\"/>\n"
        );
    }
}
