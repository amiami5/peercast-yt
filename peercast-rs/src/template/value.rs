//! テンプレートの値 (C++ の `amf0::Value`)。
//!
//! 比較 (`==`)、真偽 (`isTruish`)、文字列化 (`inspect`) は C++ 版と同じ。C++ との受け渡しには、
//! 下の `encode` / `decode` の形式 (どちらもこの crate と core/common/rusttemplate.h だけが使う)
//! を使う。

use std::collections::BTreeMap;

use super::{Error, Result};

/// 値。`Object` と `Array` (ECMA 配列) は、C++ の `std::map<std::string, Value>` と同じく
/// キーのバイト列の順に並ぶ。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Number(f64),
    String(Vec<u8>),
    Bool(bool),
    Date(f64, u16),
    Object(BTreeMap<Vec<u8>, Value>),
    Array(BTreeMap<Vec<u8>, Value>),
    StrictArray(Vec<Value>),
}

impl Value {
    pub fn str(s: &[u8]) -> Value {
        Value::String(s.to_vec())
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// `number()`
    pub fn number(&self) -> Result<f64> {
        match self {
            Value::Number(n) => Ok(*n),
            _ => Err(Error::runtime("not a number")),
        }
    }

    /// `string()`
    pub fn string(&self) -> Result<&[u8]> {
        match self {
            Value::String(s) => Ok(s),
            _ => Err(Error::runtime("not a string")),
        }
    }

    /// `object()`: オブジェクトか ECMA 配列
    pub fn object(&self) -> Result<&BTreeMap<Vec<u8>, Value>> {
        match self {
            Value::Object(m) | Value::Array(m) => Ok(m),
            _ => Err(Error::runtime("not an object or an array")),
        }
    }

    /// `strictArray()`
    pub fn strict_array(&self) -> Result<&[Value]> {
        match self {
            Value::StrictArray(a) => Ok(a),
            _ => Err(Error::runtime("not a strict array")),
        }
    }

    /// `inspect()`。文字列が UTF-8 として正しくなければ `std::invalid_argument` (C++ 版の
    /// `str::json_inspect` と同じ)。
    pub fn inspect(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.inspect_into(&mut out)?;
        Ok(out)
    }

    fn inspect_into(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Value::Number(n) => out.extend(format_g17(*n).into_bytes()),
            Value::Object(m) | Value::Array(m) => {
                out.push(b'{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.extend(json_inspect(k)?);
                    out.push(b':');
                    v.inspect_into(out)?;
                }
                out.push(b'}');
            }
            Value::String(s) => out.extend(json_inspect(s)?),
            Value::Null => out.extend_from_slice(b"null"),
            Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Value::Date(t, tz) => out.extend(format!("({}, {})", format_f6(*t), tz).into_bytes()),
            Value::StrictArray(a) => {
                out.push(b'[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    v.inspect_into(out)?;
                }
                out.push(b']');
            }
        }
        Ok(())
    }
}

fn json_inspect(s: &[u8]) -> Result<Vec<u8>> {
    crate::inspect::json_inspect(s).ok_or_else(|| Error::InvalidArgument(b"json_inspect: UTF-8 validation failed".to_vec()))
}

/// `isTruish`: null、空文字列、"0"、数の 0、false が偽
pub fn is_truish(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !(s.is_empty() || s == b"0"),
        Value::Number(n) => *n != 0.0,
        Value::Bool(b) => *b,
        _ => true,
    }
}

/// `to_s`: 文字列はそのまま、null は空、それ以外は `inspect`
pub fn to_s(v: &Value) -> Result<Vec<u8>> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(Vec::new()),
        _ => v.inspect(),
    }
}

fn nonfinite(x: f64) -> Option<String> {
    if x.is_nan() {
        Some(if x.is_sign_negative() { "-nan" } else { "nan" }.into())
    } else if x.is_infinite() {
        Some(if x < 0.0 { "-inf" } else { "inf" }.into())
    } else {
        None
    }
}

/// printf の `%.17g` (C++ 版は `std::setprecision(17)` で `ostream` に書いていた)
pub fn format_g17(x: f64) -> String {
    const P: i32 = 17;
    if let Some(s) = nonfinite(x) {
        return s;
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    // 有効数字 P 桁の指数表記 (丸めたあとの指数で書き方を決める)
    let e = format!("{:.*e}", (P - 1) as usize, x);
    let (mantissa, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if exp < -4 || exp >= P {
        let m = strip_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", m, sign, exp.abs())
    } else {
        strip_zeros(&format!("{:.*}", (P - 1 - exp) as usize, x)).to_string()
    }
}

/// 小数点以下の末尾の 0 (と、残らなければ小数点) を取る
fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// `std::to_string(double)` (`%f`)
pub fn format_f6(x: f64) -> String {
    nonfinite(x).unwrap_or_else(|| format!("{:.6}", x))
}

// ---------------------------------------------------------------- C++ との受け渡し

const T_NULL: u8 = 0;
const T_NUMBER: u8 = 1;
const T_STRING: u8 = 2;
const T_BOOL: u8 = 3;
const T_DATE: u8 = 4;
const T_OBJECT: u8 = 5;
const T_ARRAY: u8 = 6;
const T_STRICT_ARRAY: u8 = 7;

/// 型の 1 バイトのあとに中身。整数と浮動小数点数はリトルエンディアン、長さは 4 バイト。
pub fn encode(v: &Value, out: &mut Vec<u8>) {
    fn bytes(b: &[u8], out: &mut Vec<u8>) {
        out.extend((b.len() as u32).to_le_bytes());
        out.extend_from_slice(b);
    }
    match v {
        Value::Null => out.push(T_NULL),
        Value::Number(n) => {
            out.push(T_NUMBER);
            out.extend(n.to_bits().to_le_bytes());
        }
        Value::String(s) => {
            out.push(T_STRING);
            bytes(s, out);
        }
        Value::Bool(b) => {
            out.push(T_BOOL);
            out.push(*b as u8);
        }
        Value::Date(t, tz) => {
            out.push(T_DATE);
            out.extend(t.to_bits().to_le_bytes());
            out.extend(tz.to_le_bytes());
        }
        Value::Object(m) | Value::Array(m) => {
            out.push(if matches!(v, Value::Object(_)) { T_OBJECT } else { T_ARRAY });
            out.extend((m.len() as u32).to_le_bytes());
            for (k, v) in m {
                bytes(k, out);
                encode(v, out);
            }
        }
        Value::StrictArray(a) => {
            out.push(T_STRICT_ARRAY);
            out.extend((a.len() as u32).to_le_bytes());
            for v in a {
                encode(v, out);
            }
        }
    }
}

pub fn encoded(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode(v, &mut out);
    out
}

/// `encode` の逆。形式が壊れていれば `None`。
pub fn decode(b: &[u8]) -> Option<Value> {
    let mut pos = 0;
    let v = decode_at(b, &mut pos, 0)?;
    (pos == b.len()).then_some(v)
}

fn take<'a>(b: &'a [u8], pos: &mut usize, n: usize) -> Option<&'a [u8]> {
    let s = b.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    Some(s)
}

fn take_u32(b: &[u8], pos: &mut usize) -> Option<usize> {
    Some(u32::from_le_bytes(take(b, pos, 4)?.try_into().ok()?) as usize)
}

fn take_f64(b: &[u8], pos: &mut usize) -> Option<f64> {
    Some(f64::from_bits(u64::from_le_bytes(take(b, pos, 8)?.try_into().ok()?)))
}

fn decode_at(b: &[u8], pos: &mut usize, depth: usize) -> Option<Value> {
    if depth > 10000 {
        return None;
    }
    let t = *take(b, pos, 1)?.first()?;
    Some(match t {
        T_NULL => Value::Null,
        T_NUMBER => Value::Number(take_f64(b, pos)?),
        T_STRING => {
            let n = take_u32(b, pos)?;
            Value::String(take(b, pos, n)?.to_vec())
        }
        T_BOOL => Value::Bool(*take(b, pos, 1)?.first()? != 0),
        T_DATE => {
            let t = take_f64(b, pos)?;
            let tz = u16::from_le_bytes(take(b, pos, 2)?.try_into().ok()?);
            Value::Date(t, tz)
        }
        T_OBJECT | T_ARRAY => {
            let n = take_u32(b, pos)?;
            let mut m = BTreeMap::new();
            for _ in 0..n {
                let kn = take_u32(b, pos)?;
                let k = take(b, pos, kn)?.to_vec();
                let v = decode_at(b, pos, depth + 1)?;
                m.insert(k, v);
            }
            if t == T_OBJECT {
                Value::Object(m)
            } else {
                Value::Array(m)
            }
        }
        T_STRICT_ARRAY => {
            let n = take_u32(b, pos)?;
            let mut a = Vec::new();
            for _ in 0..n {
                a.push(decode_at(b, pos, depth + 1)?);
            }
            Value::StrictArray(a)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g17() {
        let cases: [(f64, &str); 14] = [
            (0.0, "0"),
            (-0.0, "-0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (0.1, "0.10000000000000001"),
            (100.0, "100"),
            (1e16, "10000000000000000"),
            (1e17, "1e+17"),
            (1e300, "1.0000000000000001e+300"),
            (0.0001, "0.0001"),
            (0.00001, "1.0000000000000001e-05"),
            (-2.5, "-2.5"),
            (f64::NAN, "nan"),
            (f64::NEG_INFINITY, "-inf"),
        ];
        for (x, s) in cases {
            assert_eq!(format_g17(x), s, "{}", x);
        }
        assert_eq!(format_g17(123456789012.0), "123456789012");
        assert_eq!(format_g17(1.0 / 3.0), "0.33333333333333331");
    }

    #[test]
    fn inspect() {
        let mut m = BTreeMap::new();
        m.insert(b"b".to_vec(), Value::Number(2.0));
        m.insert(b"a".to_vec(), Value::str(b"x\"y"));
        let v = Value::StrictArray(vec![Value::Object(m), Value::Null, Value::Bool(true), Value::Date(1.5, 9)]);
        assert_eq!(v.inspect().unwrap(), br#"[{"a":"x\"y","b":2},null,true,(1.500000, 9)]"#.to_vec());
        assert!(Value::str(b"\xff").inspect().is_err());
    }

    #[test]
    fn roundtrip() {
        let mut m = BTreeMap::new();
        m.insert(b"k".to_vec(), Value::StrictArray(vec![Value::Number(-1.25), Value::Date(3.0, 540)]));
        let v = Value::StrictArray(vec![Value::Array(m.clone()), Value::Object(m), Value::str(b"\0z"), Value::Bool(false), Value::Null]);
        assert_eq!(decode(&encoded(&v)), Some(v));
        assert_eq!(decode(&[T_STRING, 5, 0, 0, 0, b'a']), None);
        assert_eq!(decode(&[T_NULL, 0]), None);
    }

    #[test]
    fn truish() {
        assert!(!is_truish(&Value::str(b"0")));
        assert!(is_truish(&Value::str(b"00")));
        assert!(!is_truish(&Value::Number(0.0)));
        assert!(is_truish(&Value::Number(f64::NAN)));
        assert!(is_truish(&Value::StrictArray(vec![])));
    }
}
