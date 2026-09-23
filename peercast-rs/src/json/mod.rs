//! JSON (段階 8a)。C++ 版が使っている nlohmann::json 3.7.3 (`core/common/json.hpp`) と同じふるまいにする。
//!
//! * `parse`: 受け付ける文字列、数の種類 (符号なし・符号付き・浮動小数点数) の決め方、例外の文言
//!   (`what()`) を nlohmann の字句解析器・構文解析器と同じにする。
//! * `dump`: `dump()` (字下げなし、`ensure_ascii` なし、不正な UTF-8 は例外) と同じ出力。
//!   浮動小数点数は nlohmann の Grisu2 をそのまま移したもので書く。
//! * `Value` の `as_*` は、`get<T>()` などの型の変換と、失敗したときの `type_error` の文言。
//!
//! オブジェクトは nlohmann と同じく `std::map` (キーのバイト列の順) なので、`BTreeMap` にする。
//! 文字列は UTF-8 とは限らないバイト列。

mod dump;
mod parse;

pub use dump::dump;
pub use parse::{parse, ParseError};

use std::collections::BTreeMap;

pub type Object = BTreeMap<Vec<u8>, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// number_integer (int64_t)
    Int(i64),
    /// number_unsigned (uint64_t)
    UInt(u64),
    /// number_float (double)
    Float(f64),
    Str(Vec<u8>),
    Array(Vec<Value>),
    Object(Object),
}

/// nlohmann の `type_error` の `what()`。
pub fn type_error(id: u32, what: &str) -> Vec<u8> {
    format!("[json.exception.type_error.{}] {}", id, what).into_bytes()
}

impl Value {
    pub fn str(s: &[u8]) -> Value {
        Value::Str(s.to_vec())
    }

    /// `{ {"key", value}, ... }` で作るオブジェクト。同じキーは後のものが残る。
    pub fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
        let mut m = Object::new();
        for (k, v) in pairs {
            m.insert(k.as_bytes().to_vec(), v);
        }
        Value::Object(m)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// `type_name()`
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Object(_) => "object",
            Value::Array(_) => "array",
            Value::Str(_) => "string",
            Value::Bool(_) => "boolean",
            _ => "number",
        }
    }

    /// `size()`: null は 0、配列とオブジェクトは要素の数、それ以外は 1。
    pub fn size(&self) -> usize {
        match self {
            Value::Null => 0,
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            _ => 1,
        }
    }

    /// `j == "文字列"` (文字列どうしのときだけ等しくなりうる)。
    pub fn is_str(&self, s: &[u8]) -> bool {
        matches!(self, Value::Str(v) if v == s)
    }

    fn must_be(&self, what: &str) -> Vec<u8> {
        type_error(302, &format!("type must be {}, but is {}", what, self.type_name()))
    }

    /// `get<std::string>()`
    pub fn as_string(&self) -> Result<&[u8], Vec<u8>> {
        match self {
            Value::Str(s) => Ok(s),
            _ => Err(self.must_be("string")),
        }
    }

    /// `get<int>()`。nlohmann の算術型の変換 (真偽値も受け付ける) と `static_cast<int>`。
    /// 範囲外の浮動小数点数は C++ では未定義の動作なので、x86-64 と同じく `INT_MIN` にする。
    pub fn as_int(&self) -> Result<i32, Vec<u8>> {
        match *self {
            Value::UInt(u) => Ok(u as i32),
            Value::Int(i) => Ok(i as i32),
            Value::Float(f) => Ok(f64_to_i32(f)),
            Value::Bool(b) => Ok(b as i32),
            _ => Err(self.must_be("number")),
        }
    }

    /// `get<size_t>()`。64 ビットの CPU では `size_t` が nlohmann の `number_unsigned_t` と同じ型なので、
    /// 真偽値は受け付けない (32 ビットの CPU の C++ 版は受け付けていた)。
    pub fn as_size(&self) -> Result<u64, Vec<u8>> {
        match *self {
            Value::UInt(u) => Ok(u),
            Value::Int(i) => Ok(i as u64),
            Value::Float(f) => Ok(f64_to_u64(f)),
            _ => Err(self.must_be("number")),
        }
    }

    /// `get<json::object_t>()`
    pub fn as_object(&self) -> Result<&Object, Vec<u8>> {
        match self {
            Value::Object(o) => Ok(o),
            _ => Err(self.must_be("object")),
        }
    }

    /// `get<std::vector<std::string>>()`
    pub fn as_strings(&self) -> Result<Vec<Vec<u8>>, Vec<u8>> {
        match self {
            Value::Array(a) => a.iter().map(|v| v.as_string().map(|s| s.to_vec())).collect(),
            _ => Err(self.must_be("array")),
        }
    }

    /// const でない `operator[](const char*)`。null はオブジェクトになり、ないキーは null で作られる。
    pub fn index_mut(&mut self, key: &[u8]) -> Result<&mut Value, Vec<u8>> {
        if self.is_null() {
            *self = Value::Object(Object::new());
        }
        let name = self.type_name();
        match self {
            Value::Object(o) => Ok(o.entry(key.to_vec()).or_insert(Value::Null)),
            _ => Err(type_error(305, &format!("cannot use operator[] with a string argument with {}", name))),
        }
    }
}

/// x86-64 の `static_cast<int>(double)` (cvttsd2si)。NaN と範囲外は `INT_MIN`。
pub fn f64_to_i32(f: f64) -> i32 {
    let t = f.trunc();
    if t >= -2147483648.0 && t <= 2147483647.0 {
        t as i32
    } else {
        i32::MIN
    }
}

/// x86-64 の GCC の `static_cast<uint64_t>(double)`。2^63 未満は符号付きの変換 (範囲外は
/// 0x8000000000000000)、2^63 以上は 2^63 を引いて変換してから最上位ビットを反転する。
pub fn f64_to_u64(f: f64) -> u64 {
    const TWO63: f64 = 9223372036854775808.0;
    fn cvt(f: f64) -> u64 {
        let t = f.trunc();
        if t >= -TWO63 && t < TWO63 {
            t as i64 as u64
        } else {
            0x8000_0000_0000_0000
        }
    }
    if f >= TWO63 {
        cvt(f - TWO63) ^ 0x8000_0000_0000_0000
    } else {
        cvt(f)
    }
}

#[cfg(test)]
mod tests;
