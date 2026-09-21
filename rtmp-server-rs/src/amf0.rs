//! AMF0 の読み書き。C++ 版 (core/common/amf0.{h,cpp}) と同じ範囲だけを扱う。
//!
//! 対応する型: number, boolean, string, object, ECMA array, strict array,
//! null, date。undefined / reference / long string / movieclip は C++ 版
//! と同じく「未知の型」として拒否する。

use crate::{Error, Result};
use std::collections::BTreeMap;

/// オブジェクト・配列のネストの深さの上限。
pub const MAX_DEPTH: usize = 32;
/// 1 回の read_value() で読める値の総数の上限。
pub const MAX_VALUES: i64 = 100_000;

const AMF_NUMBER: u8 = 0x00;
const AMF_BOOL: u8 = 0x01;
const AMF_STRING: u8 = 0x02;
const AMF_OBJECT: u8 = 0x03;
const AMF_NULL: u8 = 0x05;
const AMF_ARRAY: u8 = 0x08;
const AMF_STRICTARRAY: u8 = 0x0a;
const AMF_DATE: u8 = 0x0b;
const AMF_OBJECT_END: u8 = 0x09;

/// AMF の文字列は任意のバイト列 (UTF-8 とは限らない)。そのまま往復できるよう Vec<u8> で持つ。
pub type Bytes = Vec<u8>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    Bool(bool),
    String(Bytes),
    /// キー順は C++ 版の std::map と同じ (バイト列の辞書順)。
    Object(BTreeMap<Bytes, Value>),
    Array(BTreeMap<Bytes, Value>),
    StrictArray(Vec<Value>),
    Null,
    Date { unix_time: f64, timezone: u16 },
}

impl Value {
    pub fn string(s: &str) -> Value {
        Value::String(s.as_bytes().to_vec())
    }

    pub fn object(pairs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.as_bytes().to_vec(), v.clone());
        }
        Value::Object(m)
    }

    pub fn as_string(&self) -> Result<&[u8]> {
        match self {
            Value::String(s) => Ok(s),
            _ => Err(Error::protocol("not a string")),
        }
    }

    /// オブジェクト/配列のキー参照。
    pub fn get(&self, key: &str) -> Result<Option<&Value>> {
        match self {
            Value::Object(m) | Value::Array(m) => Ok(m.get(key.as_bytes())),
            _ => Err(Error::protocol("not an object or an array")),
        }
    }

    /// C++ 版 Value::serialize() と同じ範囲 (number, object, string, null, date) だけを書ける。
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut b = Vec::new();
        match self {
            Value::Number(n) => {
                b.push(AMF_NUMBER);
                b.extend_from_slice(&n.to_be_bytes());
            }
            Value::Object(m) => {
                b.push(AMF_OBJECT);
                for (k, v) in m {
                    write_key(&mut b, k)?;
                    b.extend_from_slice(&v.serialize()?);
                }
                b.extend_from_slice(&[0, 0, AMF_OBJECT_END]);
            }
            Value::String(s) => {
                b.push(AMF_STRING);
                write_key(&mut b, s)?;
            }
            Value::Null => b.push(AMF_NULL),
            Value::Date { unix_time, timezone } => {
                b.push(AMF_DATE);
                b.extend_from_slice(&unix_time.to_be_bytes());
                b.extend_from_slice(&timezone.to_be_bytes());
            }
            _ => return Err(Error::protocol("serialize: unknown type")),
        }
        Ok(b)
    }

    /// ログ用の JSON 風表記。制御文字は必ずエスケープする (攻撃者が送る文字列をそのまま端末に出さない)。
    pub fn inspect(&self) -> String {
        match self {
            Value::Number(n) => format!("{}", n),
            Value::Bool(b) => b.to_string(),
            Value::String(s) => inspect_bytes(s),
            Value::Null => "null".to_string(),
            Value::Object(m) | Value::Array(m) => {
                let items: Vec<String> = m
                    .iter()
                    .map(|(k, v)| format!("{}:{}", inspect_bytes(k), v.inspect()))
                    .collect();
                format!("{{{}}}", items.join(","))
            }
            Value::StrictArray(a) => {
                let items: Vec<String> = a.iter().map(|v| v.inspect()).collect();
                format!("[{}]", items.join(","))
            }
            Value::Date { unix_time, timezone } => format!("({}, {})", unix_time, timezone),
        }
    }
}

fn inspect_bytes(s: &[u8]) -> String {
    let mut out = String::from("\"");
    for &c in s {
        match c {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(c as char),
            _ => out.push_str(&format!("\\x{:02x}", c)),
        }
    }
    out.push('"');
    out
}

fn write_key(b: &mut Vec<u8>, s: &[u8]) -> Result<()> {
    if s.len() >= 65536 {
        return Err(Error::protocol("string too long"));
    }
    b.extend_from_slice(&(s.len() as u16).to_be_bytes());
    b.extend_from_slice(s);
    Ok(())
}

/// バイト列上の読み取りカーソル。範囲外を読もうとすると Error::Eof を返す (panic しない)。
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Eof)?;
        if end > self.buf.len() {
            return Err(Error::Eof);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f64(&mut self) -> Result<f64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_be_bytes(a))
    }

    fn string(&mut self) -> Result<Bytes> {
        let len = self.u16()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    pub fn read_value(&mut self) -> Result<Value> {
        let mut budget = MAX_VALUES;
        self.value(0, &mut budget)
    }

    fn object(&mut self, depth: usize, budget: &mut i64) -> Result<BTreeMap<Bytes, Value>> {
        let mut m = BTreeMap::new();
        loop {
            let key = self.string()?;
            if key.is_empty() {
                break;
            }
            let v = self.value(depth + 1, budget)?;
            m.insert(key, v);
        }
        self.u8()?; // OBJECT_END
        Ok(m)
    }

    fn value(&mut self, depth: usize, budget: &mut i64) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(Error::protocol("AMF0: nesting too deep"));
        }
        *budget -= 1;
        if *budget < 0 {
            return Err(Error::protocol("AMF0: too many values"));
        }

        match self.u8()? {
            AMF_NUMBER => Ok(Value::Number(self.f64()?)),
            AMF_STRING => Ok(Value::String(self.string()?)),
            AMF_OBJECT => Ok(Value::Object(self.object(depth, budget)?)),
            AMF_BOOL => Ok(Value::Bool(self.u8()? != 0)),
            AMF_ARRAY => {
                self.i32()?; // length (無視)
                Ok(Value::Array(self.object(depth, budget)?))
            }
            AMF_STRICTARRAY => {
                let len = self.i32()?;
                let mut list = Vec::new();
                // 負の長さは C++ 版と同じく空配列として扱う。要素ごとに budget を消費するので
                // 巨大な長さを名乗っても無制限にはならない。
                for _ in 0..len.max(0) {
                    list.push(self.value(depth + 1, budget)?);
                }
                Ok(Value::StrictArray(list))
            }
            AMF_NULL => Ok(Value::Null),
            AMF_DATE => {
                let unix_time = self.f64()?;
                let timezone = self.u16()?;
                Ok(Value::Date { unix_time, timezone })
            }
            t => Err(Error::protocol(format!("unknown AMF value type {}", t as i8))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Value) -> Value {
        let bytes = v.serialize().unwrap();
        let mut r = Reader::new(&bytes);
        let out = r.read_value().unwrap();
        assert!(r.eof());
        out
    }

    #[test]
    fn roundtrip_basic() {
        assert_eq!(roundtrip(&Value::Number(1.5)), Value::Number(1.5));
        assert_eq!(roundtrip(&Value::string("abc")), Value::string("abc"));
        assert_eq!(roundtrip(&Value::Null), Value::Null);
        let o = Value::object(&[("b", Value::Number(2.0)), ("a", Value::string("x"))]);
        assert_eq!(roundtrip(&o), o);
    }

    #[test]
    fn object_keys_serialize_in_sorted_order() {
        // C++ 版 (std::map) と同じバイト列になること。
        let o = Value::object(&[("z", Value::Null), ("a", Value::Null)]);
        let b = o.serialize().unwrap();
        assert_eq!(b, [3, 0, 1, b'a', 5, 0, 1, b'z', 5, 0, 0, 9]);
    }

    #[test]
    fn non_utf8_string_roundtrips() {
        let v = Value::String(vec![0xff, 0xfe, 0x00, 0x41]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn truncated_input_is_eof_not_panic() {
        let full = Value::object(&[("k", Value::string("value"))]).serialize().unwrap();
        for n in 0..full.len() {
            let mut r = Reader::new(&full[..n]);
            assert!(r.read_value().is_err(), "prefix of {} bytes", n);
        }
    }

    #[test]
    fn deep_nesting_is_rejected() {
        // 0x0a (strict array) len=1 を 100 段重ねる。
        let mut b = Vec::new();
        for _ in 0..100 {
            b.extend_from_slice(&[AMF_STRICTARRAY, 0, 0, 0, 1]);
        }
        b.push(AMF_NULL);
        assert!(Reader::new(&b).read_value().is_err());
    }

    #[test]
    fn value_budget_is_enforced() {
        // 1 バイトで 1 値を作れる。宣言長 2^31-1 でも上限で止まること。
        let mut b = vec![AMF_STRICTARRAY, 0x7f, 0xff, 0xff, 0xff];
        b.extend(std::iter::repeat(AMF_NULL).take(200_000));
        match Reader::new(&b).read_value() {
            Err(Error::Protocol(m)) => assert!(m.contains("too many")),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn negative_strict_array_length_is_empty() {
        let b = [AMF_STRICTARRAY, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(Reader::new(&b).read_value().unwrap(), Value::StrictArray(vec![]));
    }

    #[test]
    fn unknown_type_rejected() {
        assert!(Reader::new(&[0x06]).read_value().is_err()); // undefined
        assert!(Reader::new(&[0x0c, 0, 0, 0, 0]).read_value().is_err()); // long string
    }
}
