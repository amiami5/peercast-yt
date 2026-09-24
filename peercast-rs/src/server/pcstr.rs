//! C++ の `String` クラス (core/common/_string.h): 255 バイトまでの文字列と、その文字コードの種類。
//! 変換そのものは段階 2 の `crate::pcstring`。

use crate::pcstring;

/// `String::TYPE`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrType {
    #[default]
    Unknown,
    Ascii,
    Esc,
    EscSafe,
    Meta,
    MetaSafe,
    Base64,
    Unicode,
    UnicodeSafe,
}

/// `String`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PcString {
    /// 中身 (NUL を含まない、255 バイトまで)
    pub data: Vec<u8>,
    pub ty: StrType,
}

/// NUL の手前まで、255 バイトまで (`strncpy(data, p, MAX_LEN - 1)`)
pub fn cut(s: &[u8]) -> Vec<u8> {
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    s[..s.len().min(pcstring::MAX_LEN - 1)].to_vec()
}

impl PcString {
    /// `String(p, T_ASCII)` / `set(p, t)`
    pub fn new(s: &[u8]) -> PcString {
        PcString { data: cut(s), ty: StrType::Ascii }
    }

    pub fn with_type(s: &[u8], ty: StrType) -> PcString {
        PcString { data: cut(s), ty }
    }

    /// `set`
    pub fn set(&mut self, s: &[u8], ty: StrType) {
        self.data = cut(s);
        self.ty = ty;
    }

    /// `operator=(const char*)` / `operator=(std::string)` (種類は T_ASCII)
    pub fn assign(&mut self, s: &[u8]) {
        self.set(s, StrType::Ascii);
    }

    /// `clear` (種類は T_UNKNOWN)
    pub fn clear(&mut self) {
        self.data.clear();
        self.ty = StrType::Unknown;
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// `isSame`
    pub fn is_same(&self, s: &[u8]) -> bool {
        self.data == cut(s)
    }

    /// `contains`: 大文字小文字を区別せずに含むか (`stristr`)
    pub fn contains(&self, s: &[u8]) -> bool {
        crate::http::stristr(&self.data, &cut(s)).is_some()
    }

    /// `append`: 収まらなければ何も足さない
    pub fn append(&mut self, s: &[u8]) {
        let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
        if s.len() + self.data.len() < pcstring::MAX_LEN - 1 {
            self.data.extend_from_slice(s);
        }
    }

    /// `setFromString`
    pub fn set_from_string(&mut self, s: &[u8], ty: StrType) {
        self.data = pcstring::from_string(&cut_nul(s));
        self.ty = ty;
    }

    /// `setUnquote`
    pub fn set_unquote(&mut self, s: &[u8], ty: StrType) {
        self.data = pcstring::unquote(&cut_nul(s));
        self.ty = ty;
    }

    /// `convertTo`
    pub fn convert_to(&mut self, t: StrType) -> &mut PcString {
        if t != self.ty {
            // まず ASCII に
            let tmp = match self.ty {
                StrType::Esc | StrType::EscSafe => pcstring::esc_to_ascii(&self.data),
                StrType::Base64 => pcstring::base64_to_ascii(&self.data),
                _ => self.data.clone(),
            };
            let tmp = cut(&tmp);
            match t {
                StrType::Unknown | StrType::Ascii => self.data = tmp,
                StrType::Unicode => self.data = pcstring::unknown_to_unicode(&tmp, false),
                StrType::UnicodeSafe => self.data = pcstring::unknown_to_unicode(&tmp, true),
                StrType::Esc => self.data = pcstring::ascii_to_esc(&tmp, false),
                StrType::EscSafe => self.data = pcstring::ascii_to_esc(&tmp, true),
                StrType::Meta => self.data = pcstring::ascii_to_meta(&tmp, false),
                StrType::MetaSafe => self.data = pcstring::ascii_to_meta(&tmp, true),
                StrType::Base64 => {}
            }
            self.ty = t;
        }
        self
    }

    /// 変換したものを返す (`String(x).convertTo(t)`)
    pub fn converted(&self, t: StrType) -> Vec<u8> {
        let mut c = self.clone();
        c.convert_to(t);
        c.data
    }
}

fn cut_nul(s: &[u8]) -> Vec<u8> {
    s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert() {
        let mut s = PcString::new(b"a b&c");
        assert_eq!(s.converted(StrType::Esc), b"a%20b%26c");
        s.convert_to(StrType::Esc);
        assert_eq!(s.converted(StrType::Ascii), b"a b&c");
        let long = [b'x'; 300];
        assert_eq!(PcString::new(&long).data.len(), 255);
        let mut a = PcString::new(&[b'y'; 250]);
        a.append(b"12345");
        assert_eq!(a.data.len(), 250);
        a.append(b"1234");
        assert_eq!(a.data.len(), 254);
        let sjis = PcString::new(b"\x82\xa0");
        assert_eq!(sjis.converted(StrType::Unicode), "あ".as_bytes());
    }
}
