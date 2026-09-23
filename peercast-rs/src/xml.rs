//! 簡易 XML パーサー (core/common/xml.cpp の `XML::read`, `XML::Node::setAttributes`,
//! `XML::Node::getBinaryContent`)。
//!
//! 字句解析と属性の解析は Rust で行い、ノードの木は C++ 側 (`Builder` の通知先) が組み立てる。
//! C++ 版の癖 (コメントは最初の `>` で終わる、閉じタグの名前は見ない、など) はそのまま。

use crate::reader::{Abort, Reader};

/// C++ 版の `BUFFER_LEN` (タグ 1 つ、内容 1 つの長さの上限)
pub const BUFFER_LEN: usize = 100 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// 読み出しが C++ の例外で中断された
    Abort,
    /// 通知先 (C++) で例外が起きた
    Callback,
    /// "Tag too long"
    TagTooLong,
    /// "Content too big"
    ContentTooBig,
    /// "Not XML document"
    NotXml,
    /// "Unexpected end tag"
    UnexpectedEndTag,
}

impl From<Abort> for Error {
    fn from(_: Abort) -> Self {
        Error::Abort
    }
}

/// 読んだ要素の通知先。どれも C++ 側で例外が起きたら `Err(())` を返す。
pub trait Builder {
    /// 今のノードの内容 (最初の NUL の手前まで)
    fn content(&mut self, s: &[u8]) -> Result<(), ()>;
    /// 開きタグ (最初の NUL の手前まで)。`single` は `<x/>` の形。
    fn start_tag(&mut self, s: &[u8], single: bool) -> Result<(), ()>;
    /// 閉じタグ
    fn end_tag(&mut self) -> Result<(), ()>;
}

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

fn cb(r: Result<(), ()>) -> Result<(), Error> {
    r.map_err(|_| Error::Callback)
}

/// `XML::read`
///
/// C++ 版はバッファを `static` で持っていて、別々のスレッドで同時に読むと壊れた。また、
/// 長さが上限ちょうどのとき、終端の NUL をバッファの 1 バイト外に書いていた。Rust 版は
/// 呼び出しごとのバッファを使う。
pub fn read(r: &mut dyn Reader, b: &mut dyn Builder) -> Result<(), Error> {
    let mut buf: Vec<u8> = Vec::new();
    // 開いているノードの数 (C++ 版の currNode が NULL でないかどうかに当たる)
    let mut depth: usize = 0;

    while !r.eof()? {
        let c = r.read_char()?;
        if c == b'<' {
            if !buf.is_empty() && depth > 0 {
                cb(b.content(until_nul(&buf)))?;
            }
            buf.clear();

            // 次の '>' まで読む
            while !r.eof()? {
                let c = r.read_char()?;
                if c == b'>' {
                    break;
                }
                if buf.len() >= BUFFER_LEN {
                    return Err(Error::TagTooLong);
                }
                buf.push(c);
            }

            let first = buf.first().copied().unwrap_or(0);
            if first == b'!' {
                // コメント。何もしない
            } else if first == b'?' {
                // 文書型宣言。"?xml " で始まること (大文字小文字は問わない)
                let rest = until_nul(&buf[1..]);
                if !(rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(b"xml ")) {
                    return Err(Error::NotXml);
                }
            } else if first == b'/' {
                if depth == 0 {
                    return Err(Error::UnexpectedEndTag);
                }
                depth -= 1;
                cb(b.end_tag())?;
            } else {
                let mut single = false;
                if buf.last() == Some(&b'/') {
                    single = true;
                    buf.pop();
                }
                let name = until_nul(&buf);
                if !name.is_empty() {
                    cb(b.start_tag(name, single))?;
                    if !single {
                        depth += 1;
                    }
                }
            }
            buf.clear();
        } else {
            if buf.len() >= BUFFER_LEN {
                return Err(Error::ContentTooBig);
            }
            buf.push(c);
        }
    }
    Ok(())
}

fn is_white_space(c: u8) -> bool {
    matches!(c, b' ' | b'\r' | b'\n' | b'\t')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrError {
    /// "Too many attributes"
    TooMany,
    /// "Bad tag value"
    BadValue,
}

/// `XML::Node::setAttributes` の結果。`data` は C++ 版の `attrData` と同じもの (名前と値の
/// 終わりを NUL に書き換えた、元と同じ長さのバイト列)。`attrs` は (名前の位置, 値の位置) で、
/// 0 番目はタグ名 (0, 0)。
#[derive(Debug, PartialEq, Eq)]
pub struct Attributes {
    pub data: Vec<u8>,
    pub attrs: Vec<(usize, usize)>,
}

/// `XML::Node::setAttributes`: `tag a="1" b="2"` のようなタグの中身を分ける。
pub fn parse_attributes(input: &[u8]) -> Result<Attributes, AttrError> {
    let mut a = until_nul(input).to_vec();
    let at = |a: &Vec<u8>, i: usize| a.get(i).copied().unwrap_or(0);

    // 属性の数の上限 (引用符の外の '=' の数 + タグ名)
    let mut max_attr = 1;
    let mut in_q = false;
    for &c in &a {
        if c == b'"' {
            in_q = !in_q;
        }
        if !in_q && c == b'=' {
            max_attr += 1;
        }
    }

    let mut attrs = vec![(0usize, 0usize)];

    // 最初の空白まで (タグ名)
    let mut i = 0;
    let mut c;
    loop {
        c = at(&a, i);
        i += 1;
        if c == 0 || is_white_space(c) {
            break;
        }
    }
    if c == 0 {
        return Ok(Attributes { data: a, attrs }); // 属性なし
    }
    a[i - 1] = 0;

    while at(&a, i) != 0 {
        if is_white_space(at(&a, i)) {
            i += 1;
            continue;
        }
        if attrs.len() >= max_attr {
            return Err(AttrError::TooMany);
        }

        // 名前は次の空白か '=' で終わる。空白で終わっても '=' まで読み進める。
        let name_pos = i;
        while at(&a, i) != 0 {
            let c = a[i];
            i += 1;
            if c == b'=' || is_white_space(c) {
                a[i - 1] = 0;
                if c == b'=' {
                    break;
                }
            }
        }

        // 空白を飛ばす
        while at(&a, i) != 0 && is_white_space(a[i]) {
            i += 1;
        }

        // 値は '"' で始まること
        if at(&a, i) != b'"' {
            return Err(AttrError::BadValue);
        }
        i += 1;
        let value_pos = i;
        attrs.push((name_pos, value_pos));

        // 値は次の '"' で終わる。閉じる '"' がなければ、C++ 版と同じく値の最後の 1 文字を
        // 捨てる (最後の文字を NUL に書き換える)。
        while at(&a, i) != 0 {
            let c = a[i];
            i += 1;
            if c == b'"' {
                break;
            }
        }
        a[i - 1] = 0;
    }
    Ok(Attributes { data: a, attrs })
}

/// C++ 版の `nibsToByte` の 1 文字分。C++ 版は `char` で計算しており、0x80 以上のバイトの
/// 結果が CPU で違った (`char` の符号)。Rust 版は CPU によらず符号付きとして計算する。
fn nib(c: u8) -> i32 {
    let c = c as i8 as i32;
    if c >= b'A' as i32 {
        c - b'A' as i32 + 10
    } else {
        c - b'0' as i32
    }
}

/// `XML::Node::getBinaryContent`: 16 進の内容をバイト列にする (空白は飛ばす)。1 バイトは
/// 下位 4 ビット、上位 4 ビットの順に書かれている。`size` バイトを超えたら `None`。
///
/// C++ 版は、16 進数字が奇数個だと最後の 1 文字の次に終端の NUL を読み、さらにその先の
/// メモリを読み進めていた。Rust 版は足りない 1 文字を NUL とみなして、そこで止まる。
pub fn binary_content(input: &[u8], size: usize) -> Option<Vec<u8>> {
    let input = until_nul(input);
    let mut out = Vec::new();
    let mut i = 0;
    while i < input.len() {
        if is_white_space(input[i]) {
            i += 1;
            continue;
        }
        if out.len() >= size {
            return None;
        }
        let n1 = nib(input[i]);
        let n2 = nib(input.get(i + 1).copied().unwrap_or(0));
        out.push((((n2 & 0xf) << 4) | (n1 & 0xf)) as u8);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の読み出し口 (eof は位置で判断する)
    struct Src<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl Reader for Src<'_> {
        fn read_char(&mut self) -> Result<u8, Abort> {
            let c = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            Ok(c)
        }
        fn read_exact(&mut self, _: usize) -> Result<Vec<u8>, Abort> { unreachable!() }
        fn read_some(&mut self, _: usize) -> Result<Vec<u8>, Abort> { unreachable!() }
        fn eof(&mut self) -> Result<bool, Abort> { Ok(self.pos >= self.data.len()) }
    }

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Builder for Log {
        fn content(&mut self, s: &[u8]) -> Result<(), ()> {
            self.0.push(format!("c:{}", String::from_utf8_lossy(s)));
            Ok(())
        }
        fn start_tag(&mut self, s: &[u8], single: bool) -> Result<(), ()> {
            self.0.push(format!("{}{}", if single { "s:" } else { "t:" }, String::from_utf8_lossy(s)));
            Ok(())
        }
        fn end_tag(&mut self) -> Result<(), ()> {
            self.0.push("e".into());
            Ok(())
        }
    }

    fn parse(s: &[u8]) -> (Result<(), Error>, Vec<String>) {
        let mut r = Src { data: s, pos: 0 };
        let mut log = Log::default();
        let res = read(&mut r, &mut log);
        (res, log.0)
    }

    #[test]
    fn document() {
        let (res, log) = parse(b"<?xml version=\"1.0\"?><a x=\"1\">hi<b/><!-- c --></a>");
        assert_eq!(res, Ok(()));
        assert_eq!(log, ["t:a x=\"1\"", "c:hi", "s:b", "e"]);
        assert_eq!(parse(b"<?html ?>").0, Err(Error::NotXml));
        assert_eq!(parse(b"</a>").0, Err(Error::UnexpectedEndTag));
        // 空のタグは無視 (C++ 版は buf[-1] を読んでいた箇所)
        assert_eq!(parse(b"<>").1, Vec::<String>::new());
        // 内容はノードの外なら捨てる
        assert_eq!(parse(b"xx<a>").1, ["t:a"]);
    }

    #[test]
    fn limits() {
        let mut long = b"<".to_vec();
        long.extend(std::iter::repeat(b'a').take(BUFFER_LEN + 1));
        assert_eq!(parse(&long).0, Err(Error::TagTooLong));
        let long = vec![b'a'; BUFFER_LEN + 1];
        assert_eq!(parse(&long).0, Err(Error::ContentTooBig));
        // ちょうど上限の長さは受け付ける
        let mut exact = b"<".to_vec();
        exact.extend(std::iter::repeat(b'a').take(BUFFER_LEN));
        exact.push(b'>');
        assert_eq!(parse(&exact).0, Ok(()));
    }

    fn attr_strings(r: &Attributes) -> Vec<(String, String)> {
        let s = |p: usize| String::from_utf8_lossy(until_nul(&r.data[p..])).into_owned();
        r.attrs.iter().map(|&(n, v)| (s(n), s(v))).collect()
    }

    #[test]
    fn attributes() {
        let r = parse_attributes(b"host ip=\"1.2.3.4\" port = \"7144\"").unwrap();
        assert_eq!(
            attr_strings(&r),
            [("host".into(), "host".into()), ("ip".into(), "1.2.3.4".into()), ("port".into(), "7144".into())]
        );
        assert_eq!(attr_strings(&parse_attributes(b"tag").unwrap()), [("tag".into(), "tag".into())]);
        assert_eq!(parse_attributes(b"tag a b"), Err(AttrError::TooMany));
        assert_eq!(parse_attributes(b"tag a=1"), Err(AttrError::BadValue));
        // 閉じる '"' がないと、値の最後の文字が落ちる (C++ 版と同じ)
        assert_eq!(attr_strings(&parse_attributes(b"t a=\"xyz").unwrap())[1].1, "xy");
    }

    #[test]
    fn binary() {
        assert_eq!(binary_content(b"10 32", 10), Some(vec![0x01, 0x23]));
        assert_eq!(binary_content(b"1032", 1), None);
        // 奇数個: 足りない 1 文字は NUL とみなして止まる
        assert_eq!(binary_content(b"1", 10), Some(vec![0x01]));
    }
}
