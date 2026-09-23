//! AMF0 のデシリアライザ (core/common/amf0.cpp の `amf0::Deserializer`)。
//!
//! 読んだ値は `Builder` に順に通知する (値を組み立てるのは C++ 側。信頼できない入力の
//! 解釈は Rust 側だけで行い、C++ 側は Rust が確かめた構造をなぞるだけになる)。
//! 読み出しの順序、エラーの種類と起きる順番は C++ 版と同じ。

use crate::reader::{Abort, Reader};

pub const AMF_NUMBER: u8 = 0x00;
pub const AMF_BOOL: u8 = 0x01;
pub const AMF_STRING: u8 = 0x02;
pub const AMF_OBJECT: u8 = 0x03;
pub const AMF_NULL: u8 = 0x05;
pub const AMF_ARRAY: u8 = 0x08;
pub const AMF_STRICTARRAY: u8 = 0x0a;
pub const AMF_DATE: u8 = 0x0b;

/// `Deserializer::MAX_DEPTH`
pub const MAX_DEPTH: i32 = 32;
/// `Deserializer::MAX_VALUES`
pub const MAX_VALUES: i32 = 100000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// 読み出しが C++ の例外で中断された
    Abort,
    /// "AMF0: nesting too deep"
    TooDeep,
    /// "AMF0: too many values"
    TooMany,
    /// "unknown AMF value type N"。N は符号付きの `char` としての値。
    UnknownType(i8),
}

impl From<Abort> for Error {
    fn from(_: Abort) -> Self {
        Error::Abort
    }
}

/// オブジェクトの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Object,
    /// ECMA 配列
    Array,
}

/// 読んだ値の通知先。オブジェクトは `begin_object`, (`key`, 値)*, `end_object` の順、
/// 厳密配列は `begin_strict_array`, 値*, `end_strict_array` の順に通知する。
pub trait Builder {
    fn number(&mut self, v: f64);
    fn string(&mut self, s: &[u8]);
    fn boolean(&mut self, b: bool);
    fn null(&mut self);
    fn date(&mut self, unix_time: f64, timezone: u16);
    fn begin_object(&mut self, kind: ObjectKind);
    fn key(&mut self, k: &[u8]);
    fn end_object(&mut self);
    fn begin_strict_array(&mut self);
    fn end_strict_array(&mut self);
}

/// `readBool`
pub fn read_bool(r: &mut dyn Reader) -> Result<bool, Abort> {
    Ok(r.read_char()? != 0)
}

/// `readInt32` (ビッグエンディアン)
pub fn read_int32(r: &mut dyn Reader) -> Result<i32, Abort> {
    let mut b = [0u8; 4];
    for x in b.iter_mut() {
        *x = r.read_char()?;
    }
    Ok(i32::from_be_bytes(b))
}

/// `readInt16` (ビッグエンディアン)
pub fn read_int16(r: &mut dyn Reader) -> Result<i16, Abort> {
    let b0 = r.read_char()?;
    let b1 = r.read_char()?;
    Ok(i16::from_be_bytes([b0, b1]))
}

/// `readString`: 2 バイトの長さ (0〜65535) と、その長さの内容。
pub fn read_string(r: &mut dyn Reader) -> Result<Vec<u8>, Abort> {
    let b0 = r.read_char()?;
    let b1 = r.read_char()?;
    let len = u16::from_be_bytes([b0, b1]) as usize;
    r.read_exact(len)
}

/// `readDouble` (ビッグエンディアンの IEEE 754 倍精度)。
///
/// C++ 版はバイトを逆順にメモリへ書いて `double` とみなしており、リトルエンディアンの
/// CPU でしか正しく動かなかった。Rust 版はどの CPU でも同じ値になる。
pub fn read_double(r: &mut dyn Reader) -> Result<f64, Abort> {
    let mut b = [0u8; 8];
    for x in b.iter_mut() {
        *x = r.read_char()?;
    }
    Ok(f64::from_be_bytes(b))
}

/// 1 回の読み出しで共有する状態 (C++ 版の `budget`)。
struct State<'a> {
    r: &'a mut dyn Reader,
    b: &'a mut dyn Builder,
    budget: i32,
}

impl State<'_> {
    fn read_object(&mut self, depth: i32, kind: ObjectKind) -> Result<(), Error> {
        self.b.begin_object(kind);
        loop {
            let key = read_string(self.r)?;
            if key.is_empty() {
                break;
            }
            self.b.key(&key);
            self.read_value(depth + 1)?;
        }
        self.r.read_char()?; // OBJECT_END
        self.b.end_object();
        Ok(())
    }

    fn read_value(&mut self, depth: i32) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        self.budget -= 1;
        if self.budget < 0 {
            return Err(Error::TooMany);
        }

        let ty = self.r.read_char()?;
        match ty {
            AMF_NUMBER => {
                let v = read_double(self.r)?;
                self.b.number(v);
            }
            AMF_STRING => {
                let s = read_string(self.r)?;
                self.b.string(&s);
            }
            AMF_OBJECT => self.read_object(depth, ObjectKind::Object)?,
            AMF_BOOL => {
                let v = read_bool(self.r)?;
                self.b.boolean(v);
            }
            AMF_ARRAY => {
                read_int32(self.r)?; // length (使わない)
                self.read_object(depth, ObjectKind::Array)?;
            }
            AMF_STRICTARRAY => {
                let len = read_int32(self.r)?;
                self.b.begin_strict_array();
                for _ in 0..len.max(0) {
                    self.read_value(depth + 1)?;
                }
                self.b.end_strict_array();
            }
            AMF_NULL => self.b.null(),
            AMF_DATE => {
                let t = read_double(self.r)?;
                let tz = read_int16(self.r)? as u16;
                self.b.date(t, tz);
            }
            // C++ 版は型を char で表示しており、0x80 以上の値の表示が CPU で違っていた
            // (x86 では負の数、ARM の Linux では正の数)。Rust 版は常に符号付き。
            _ => return Err(Error::UnknownType(ty as i8)),
        }
        Ok(())
    }
}

/// `Deserializer::readValue`
pub fn read_value(r: &mut dyn Reader, b: &mut dyn Builder) -> Result<(), Error> {
    State { r, b, budget: MAX_VALUES }.read_value(0)
}

/// `Deserializer::readObject` (型のバイトのないオブジェクト)
pub fn read_object(r: &mut dyn Reader, b: &mut dyn Builder) -> Result<(), Error> {
    State { r, b, budget: MAX_VALUES }.read_object(0, ObjectKind::Object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::SliceReader;

    /// 通知を文字列にして並べる
    #[derive(Default)]
    struct Log(Vec<String>);

    impl Builder for Log {
        fn number(&mut self, v: f64) { self.0.push(format!("n{}", v)); }
        fn string(&mut self, s: &[u8]) { self.0.push(format!("s{}", String::from_utf8_lossy(s))); }
        fn boolean(&mut self, b: bool) { self.0.push(format!("b{}", b)); }
        fn null(&mut self) { self.0.push("null".into()); }
        fn date(&mut self, t: f64, tz: u16) { self.0.push(format!("d{},{}", t, tz)); }
        fn begin_object(&mut self, k: ObjectKind) { self.0.push(format!("{{{:?}", k)); }
        fn key(&mut self, k: &[u8]) { self.0.push(format!("k{}", String::from_utf8_lossy(k))); }
        fn end_object(&mut self) { self.0.push("}".into()); }
        fn begin_strict_array(&mut self) { self.0.push("[".into()); }
        fn end_strict_array(&mut self) { self.0.push("]".into()); }
    }

    fn parse(data: &[u8]) -> (Result<(), Error>, Vec<String>, usize) {
        let mut r = SliceReader { data, pos: 0 };
        let mut log = Log::default();
        let res = read_value(&mut r, &mut log);
        (res, log.0, r.pos)
    }

    #[test]
    fn scalars() {
        assert_eq!(parse(b"\x05").1, ["null"]);
        assert_eq!(parse(b"\x01\x01").1, ["btrue"]);
        assert_eq!(parse(b"\x00\x3f\xf0\0\0\0\0\0\0").1, ["n1"]);
        assert_eq!(parse(b"\x02\x00\x03abc").1, ["sabc"]);
        assert_eq!(parse(b"\x0b\x3f\xf0\0\0\0\0\0\0\x00\x09").1, ["d1,9"]);
    }

    #[test]
    fn object_and_arrays() {
        let (res, log, pos) = parse(b"\x03\x00\x01a\x05\x00\x00\x09");
        assert_eq!(res, Ok(()));
        assert_eq!(log, ["{Object", "ka", "null", "}"]);
        assert_eq!(pos, 8);
        let (_, log, _) = parse(b"\x08\x00\x00\x00\x01\x00\x01a\x05\x00\x00\x09");
        assert_eq!(log, ["{Array", "ka", "null", "}"]);
        let (_, log, _) = parse(b"\x0a\x00\x00\x00\x02\x05\x05");
        assert_eq!(log, ["[", "null", "null", "]"]);
        // 負の長さの厳密配列は空
        let (_, log, _) = parse(b"\x0a\xff\xff\xff\xff");
        assert_eq!(log, ["[", "]"]);
    }

    #[test]
    fn errors() {
        assert_eq!(parse(b"\x0c").0, Err(Error::UnknownType(0x0c)));
        assert_eq!(parse(b"\xff").0, Err(Error::UnknownType(-1)));
        // 文字列が足りない (read_exact が中断)
        assert_eq!(parse(b"\x02\x00\x05ab").0, Err(Error::Abort));
        // 深すぎるネスト
        let mut deep = Vec::new();
        for _ in 0..40 {
            deep.extend_from_slice(b"\x0a\x00\x00\x00\x01");
        }
        deep.push(0x05);
        assert_eq!(parse(&deep).0, Err(Error::TooDeep));
        // 値が多すぎる
        let mut many = b"\x0a\x7f\xff\xff\xff".to_vec();
        many.extend(std::iter::repeat(0x05).take(200000));
        assert_eq!(parse(&many).0, Err(Error::TooMany));
    }

    #[test]
    fn truncated_input_reads_zeros_like_memory_stream() {
        // MemoryStream はデータが尽きると 0 を返すので、足りない数値は 0 になる
        let expected = f64::from_be_bytes([0x3f, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(parse(b"\x00\x3f").1, [format!("n{}", expected)]);
        assert_eq!(parse(b"").1, ["n0"]);
    }
}
