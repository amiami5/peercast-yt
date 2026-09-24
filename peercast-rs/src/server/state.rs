//! `getState` (C++ の `VariableWriter::getState`) の値を組み立てるための小さな関数。
//! 値はテンプレートの `Value` (C++ の `amf0::Value`)。

use std::collections::BTreeMap;

pub use crate::template::value::Value;

/// 文字列
pub fn s(v: impl AsRef<[u8]>) -> Value {
    Value::String(v.as_ref().to_vec())
}

/// 数 (C++ の `amf0::Value(int)` など)
pub fn n(v: impl Into<f64>) -> Value {
    Value::Number(v.into())
}

pub fn b(v: bool) -> Value {
    Value::Bool(v)
}

/// オブジェクト (キーの順に並ぶ)
pub fn obj<K: AsRef<[u8]>>(entries: Vec<(K, Value)>) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in entries {
        m.insert(k.as_ref().to_vec(), v);
    }
    Value::Object(m)
}

/// 配列 (`std::vector<amf0::Value>`)
pub fn arr(v: Vec<Value>) -> Value {
    Value::StrictArray(v)
}

/// `std::to_string(bool)` などの "1" / "0"
pub fn flag(v: bool) -> Value {
    s(if v { "1" } else { "0" })
}

/// `VariableWriter::getVariable` / `writeVariable` の、`.` で区切った名前で値を探す部分
/// (`getProperty`)。見つからなければ `None`。
pub fn property(v: &Value, path: &[u8]) -> Option<Value> {
    let m = match v {
        Value::Object(m) => m,
        _ => return None,
    };
    if let Some(x) = m.get(path) {
        return Some(x.clone());
    }
    let parts = crate::strutil::split(path, b".");
    if parts.len() == 1 {
        return None;
    }
    let head = &parts[0];
    let next = m.get(head.as_slice()).cloned().unwrap_or(Value::Null);
    property(&next, &path[head.len() + 1..])
}
