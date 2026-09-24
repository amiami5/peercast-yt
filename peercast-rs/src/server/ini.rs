//! 設定ファイル (core/common/ini.cpp の `ini::Document` の書き出し、inifile.cpp の `IniFileBase` の読み出し)。

use super::stream::{Stream, StreamExt};

/// `ini::Section`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub keys: Vec<(String, Vec<u8>)>,
    pub end_tag: String,
}

impl Section {
    pub fn new(name: &str, end: bool) -> Section {
        Section { name: name.into(), keys: Vec::new(), end_tag: if end { "End".into() } else { String::new() } }
    }

    pub fn key(mut self, name: &str, v: impl IniValue) -> Section {
        self.keys.push((name.into(), v.ini()));
        self
    }

    pub fn push(&mut self, name: &str, v: impl IniValue) {
        self.keys.push((name.into(), v.ini()));
    }

    /// `Section::dump`
    pub fn dump(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(format!("\n[{}]\n", self.name).as_bytes());
        for (k, v) in &self.keys {
            buf.extend_from_slice(k.as_bytes());
            buf.extend_from_slice(b" = ");
            buf.extend(line_value(v, MAX_LINE.saturating_sub(k.len() + 3)));
            buf.push(b'\n');
        }
        if !self.end_tag.is_empty() {
            buf.extend_from_slice(format!("[{}]\n", self.end_tag).as_bytes());
        }
        buf
    }
}

/// 読み込める 1 行の長さ (C++ 版の `IniFileBase::readNext` の `char buf[256]`)
const MAX_LINE: usize = 255;

/// 1 行に書く値: 制御文字 (改行を含む) を除き、`max` バイトに収まるように UTF-8 の文字の途中で
/// 切れないところで切る。
///
/// C++ 版はそのまま書いていたので、ネットワークから受け取ったチャンネル名などに改行があると
/// (中継チャンネルは名前などを保存する)、次に読むときに別の行として `password` などを書き換え
/// られた。長すぎる値も、残りが次の行として読まれた。
fn line_value(v: &[u8], max: usize) -> Vec<u8> {
    let mut v: Vec<u8> = v.iter().copied().filter(|&c| c >= 0x20 && c != 0x7f).collect();
    if v.len() > max {
        let mut cut = max;
        while cut > 0 && v[cut] & 0xc0 == 0x80 {
            cut -= 1;
        }
        v.truncate(cut);
    }
    v
}

/// `ini::dump`
pub fn dump(doc: &[Section]) -> Vec<u8> {
    doc.iter().flat_map(|s| s.dump()).collect()
}

/// `ini::Value` のコンストラクター (真偽値は "Yes" / "No")
pub trait IniValue {
    fn ini(&self) -> Vec<u8>;
}

impl IniValue for bool {
    fn ini(&self) -> Vec<u8> {
        if *self { b"Yes".to_vec() } else { b"No".to_vec() }
    }
}

impl IniValue for i32 {
    fn ini(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl IniValue for u32 {
    fn ini(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl IniValue for u16 {
    fn ini(&self) -> Vec<u8> {
        (*self as i32).to_string().into_bytes()
    }
}

impl IniValue for &str {
    fn ini(&self) -> Vec<u8> {
        cut_nul(self.as_bytes())
    }
}

impl IniValue for String {
    fn ini(&self) -> Vec<u8> {
        cut_nul(self.as_bytes())
    }
}

impl IniValue for &[u8] {
    fn ini(&self) -> Vec<u8> {
        cut_nul(self)
    }
}

impl IniValue for Vec<u8> {
    fn ini(&self) -> Vec<u8> {
        cut_nul(self)
    }
}

/// C++ 版は `const char*` や `String` から作るので、NUL で切れる
fn cut_nul(s: &[u8]) -> Vec<u8> {
    s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())].to_vec()
}

/// `IniFileBase`: 1 行ずつ `名前 = 値` を読む
pub struct IniReader<'a> {
    stream: &'a mut dyn Stream,
    name: Vec<u8>,
    value: Option<Vec<u8>>,
}

/// `trimstr`: 前後の空白とタブを取る
fn trimstr(s: &[u8]) -> Vec<u8> {
    let start = s.iter().position(|&c| c != b' ' && c != b'\t').unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c != b' ' && c != b'\t').map_or(start, |e| e + 1);
    if end <= start {
        Vec::new()
    } else {
        s[start..end].to_vec()
    }
}

/// `Sys::stricmp` が 0 か (ASCII の大文字小文字を区別しない)
pub fn stricmp_eq(a: &[u8], b: &[u8]) -> bool {
    let a = &a[..a.iter().position(|&c| c == 0).unwrap_or(a.len())];
    let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
    a.eq_ignore_ascii_case(b)
}

impl<'a> IniReader<'a> {
    pub fn new(stream: &'a mut dyn Stream) -> IniReader<'a> {
        IniReader { stream, name: Vec::new(), value: None }
    }

    /// 1 行 (LF まで。CR は捨てる)。255 バイトを超えた分は捨てる (C++ 版は次の行として読んでいた
    /// ので、長い値の後ろに別の設定を書けた)。最後の行に LF がなくてもよい。何も読めなければ `None`
    fn read_line(&mut self) -> Option<Vec<u8>> {
        let mut line = Vec::new();
        let mut any = false;
        loop {
            let c = match self.stream.read_char() {
                Ok(c) => c,
                Err(_) => return if any { Some(line) } else { None },
            };
            any = true;
            match c {
                b'\n' => return Some(line),
                b'\r' => {}
                _ if line.len() < MAX_LINE => line.push(c),
                _ => {}
            }
        }
    }

    /// `readNext`: 次の行 (255 バイトまで)。読めなければ false
    pub fn read_next(&mut self) -> bool {
        match self.stream.eof() {
            Ok(false) => {}
            _ => return false,
        }
        let line = match self.read_line() {
            Some(l) => l,
            None => return false,
        };
        // C++ 版は NUL を文字列の終わりとみなす
        let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        match line.iter().position(|&c| c == b'=') {
            Some(i) => {
                self.value = Some(trimstr(&line[i + 1..]));
                self.name = trimstr(&line[..i]);
            }
            None => {
                self.value = None;
                self.name = trimstr(line);
            }
        }
        true
    }

    /// `isName`
    pub fn is_name(&self, s: &str) -> bool {
        stricmp_eq(&self.name, s.as_bytes())
    }

    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// `getIntValue` (`atoi`。Rust 版は `int` に収まらなければ端の値)
    pub fn int_value(&self) -> i32 {
        self.value.as_deref().map_or(0, crate::http::atoi)
    }

    /// `getStrValue`
    pub fn str_value(&self) -> &[u8] {
        self.value.as_deref().unwrap_or(b"")
    }

    /// `getBoolValue`: yes / y / 1 (大文字小文字を区別しない)
    pub fn bool_value(&self) -> bool {
        match &self.value {
            Some(v) => stricmp_eq(v, b"yes") || stricmp_eq(v, b"y") || stricmp_eq(v, b"1"),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::stream::StringStream;
    use super::*;

    #[test]
    fn dump_sections() {
        let doc = vec![
            Section::new("Server", false).key("serverName", "").key("serverPort", 7144u16).key("autoServe", true),
            Section::new("Filter", true).key("ip", "255.255.255.255").key("private", false),
        ];
        assert_eq!(
            String::from_utf8(dump(&doc)).unwrap(),
            "\n[Server]\nserverName = \nserverPort = 7144\nautoServe = Yes\n\n[Filter]\nip = 255.255.255.255\nprivate = No\n[End]\n"
        );
    }

    #[test]
    fn read() {
        let mut s = StringStream::from(b"\n[Server]\r\n Name = a = b \nflag=YES\n  [End]\nlast = x".to_vec());
        let mut r = IniReader::new(&mut s);
        assert!(r.read_next());
        assert_eq!(r.name(), b"");
        assert!(r.read_next());
        assert!(r.is_name("[server]"));
        assert!(r.read_next());
        assert!(r.is_name("name"));
        assert_eq!(r.str_value(), b"a = b");
        assert!(r.read_next());
        assert!(r.bool_value());
        assert!(r.read_next());
        assert!(r.is_name("[End]"));
        // 最後の行に改行がなくても読む (C++ 版は読めなかった)
        assert!(r.read_next());
        assert!(r.is_name("last"));
        assert_eq!(r.str_value(), b"x");
        assert!(!r.read_next());
    }

    /// 全部の行を (名前, 値) にする
    fn read_all(data: Vec<u8>) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut s = StringStream::from(data);
        let mut r = IniReader::new(&mut s);
        let mut v = Vec::new();
        while r.read_next() {
            v.push((r.name().to_vec(), r.str_value().to_vec()));
        }
        v
    }

    #[test]
    fn values_cannot_inject_lines() {
        // ネットワークから受け取ったチャンネル名などに改行があっても、別の行にならない
        let evil: &[u8] = b"ch\n[End]\npassword = x\r\n[Server]\x0b\x7f";
        let doc = vec![Section::new("RelayChannel", true).key("name", evil).key("genre", "g")];
        let out = dump(&doc);
        assert_eq!(out, b"\n[RelayChannel]\nname = ch[End]password = x[Server]\ngenre = g\n[End]\n");
        let lines = read_all(out);
        assert_eq!(lines.iter().filter(|(n, _)| n.eq_ignore_ascii_case(b"password")).count(), 0);

        // 長い値は 1 行 (255 バイト) に収まるように切る。残りが次の行にならない
        let long = [vec![b'a'; 250], b"password=x".to_vec()].concat();
        let out = dump(&[Section::new("RelayChannel", true).key("comment", &long[..])]);
        assert!(out.split(|&c| c == b'\n').all(|l| l.len() <= MAX_LINE));
        let lines = read_all(out);
        assert_eq!(lines.len(), 4, "{:?}", lines);
        assert!(lines.iter().all(|(n, _)| !n.eq_ignore_ascii_case(b"password")));

        // UTF-8 の文字の途中では切らない
        let jp = "あ".repeat(100); // 300 バイト
        let out = dump(&[Section::new("S", false).key("name", jp.as_str())]);
        let line = out.split(|&c| c == b'\n').find(|l| l.starts_with(b"name")).unwrap().to_vec();
        assert!(line.len() <= MAX_LINE);
        let v = std::str::from_utf8(&line[b"name = ".len()..]).unwrap();
        assert_eq!(v, "あ".repeat((MAX_LINE - 7) / 3));
    }

    #[test]
    fn long_lines_are_cut_when_read() {
        // 古い版が書いた長すぎる行 (C++ 版は 255 バイトから後ろを次の行として読んでいた)
        let data = [b"comment = ".to_vec(), vec![b'a'; 245], b"password = x\nnext = 1\n".to_vec()].concat();
        let lines = read_all(data);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, b"comment");
        assert_eq!(lines[0].1.len(), MAX_LINE - 10);
        assert_eq!(lines[1], (b"next".to_vec(), b"1".to_vec()));
    }
}
