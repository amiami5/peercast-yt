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
            buf.extend_from_slice(v);
            buf.push(b'\n');
        }
        if !self.end_tag.is_empty() {
            buf.extend_from_slice(format!("[{}]\n", self.end_tag).as_bytes());
        }
        buf
    }
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

    /// `readNext`: 次の行 (255 バイトまで)。読めなければ false
    pub fn read_next(&mut self) -> bool {
        match self.stream.eof() {
            Ok(false) => {}
            _ => return false,
        }
        let line = match self.stream.read_line_buf(256) {
            Ok(l) => l,
            Err(_) => return false,
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
        // 最後の行に改行がなければ読めない (C++ 版と同じ)
        assert!(!r.read_next());
    }
}
