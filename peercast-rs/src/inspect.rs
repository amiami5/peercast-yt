//! デバッグ表示用の文字列化。C++ 版 core/common/str.cpp の `inspect`, `json_inspect` に相当する。
//! 出力は ASCII の範囲では C++ 版と同一。`isprint` は C ロケールの規則 (0x20〜0x7E) で判定する。

use crate::utf8;

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

fn is_print(b: u8) -> bool {
    (0x20..=0x7e).contains(&b)
}

fn push_hex_escape(out: &mut Vec<u8>, prefix: &[u8], b: u8) {
    out.extend_from_slice(prefix);
    out.push(HEX_LOWER[(b >> 4) as usize]);
    out.push(HEX_LOWER[(b & 0xf) as usize]);
}

/// C 言語風のエスケープをした文字列 (二重引用符付き)。UTF-8 として正しければ、非 ASCII はそのまま出す。
pub fn inspect(s: &[u8]) -> Vec<u8> {
    let utf8_ok = utf8::validate(s);
    let mut out = vec![b'"'];
    for &b in s {
        if utf8_ok && b >= 0x80 {
            out.push(b);
        } else if is_print(b) {
            match b {
                b'\'' => out.extend_from_slice(b"\\'"),
                b'"' => out.extend_from_slice(b"\\\""),
                b'?' => out.extend_from_slice(b"\\?"),
                b'\\' => out.extend_from_slice(b"\\\\"),
                _ => out.push(b),
            }
        } else {
            match b {
                0x07 => out.extend_from_slice(b"\\a"),
                0x08 => out.extend_from_slice(b"\\b"),
                0x0c => out.extend_from_slice(b"\\f"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                _ => push_hex_escape(&mut out, b"\\x", b),
            }
        }
    }
    out.push(b'"');
    out
}

/// JSON の文字列リテラル (二重引用符付き)。UTF-8 として正しくなければ `None`。
pub fn json_inspect(s: &[u8]) -> Option<Vec<u8>> {
    if !utf8::validate(s) {
        return None;
    }
    let mut out = vec![b'"'];
    for &b in s {
        if b >= 0x80 {
            out.push(b);
        } else if is_print(b) {
            match b {
                b'"' => out.extend_from_slice(b"\\\""),
                b'\\' => out.extend_from_slice(b"\\\\"),
                _ => out.push(b),
            }
        } else {
            match b {
                0x08 => out.extend_from_slice(b"\\b"),
                0x0c => out.extend_from_slice(b"\\f"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                _ => push_hex_escape(&mut out, b"\\u00", b),
            }
        }
    }
    out.push(b'"');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_basic() {
        assert_eq!(inspect(b""), b"\"\"");
        assert_eq!(inspect(b"abc"), b"\"abc\"");
        assert_eq!(inspect(b"a\"b'c?d\\e"), b"\"a\\\"b\\'c\\?d\\\\e\"");
        assert_eq!(inspect(b"\x07\x08\x0c\n\r\t\x0b\x00\x7f"), b"\"\\a\\b\\f\\n\\r\\t\\x0b\\x00\\x7f\"");
    }

    #[test]
    fn inspect_keeps_valid_utf8_and_escapes_the_rest() {
        assert_eq!(inspect("あ".as_bytes()), "\"あ\"".as_bytes());
        // 不正な UTF-8 が混ざると、非 ASCII は全部 \x に (C++ 版と同じ)
        assert_eq!(inspect(b"a\xffb"), b"\"a\\xffb\"");
        assert_eq!(inspect(&[b'a', 0xe3, 0x81, 0x82, 0xff]), b"\"a\\xe3\\x81\\x82\\xff\"");
    }

    #[test]
    fn json_inspect_basic() {
        assert_eq!(json_inspect(b"").unwrap(), b"\"\"");
        assert_eq!(json_inspect(b"a\"b\\c").unwrap(), b"\"a\\\"b\\\\c\"");
        assert_eq!(json_inspect(b"\x08\x0c\n\r\t").unwrap(), b"\"\\b\\f\\n\\r\\t\"");
        assert_eq!(json_inspect(b"\x00\x07\x1f\x7f").unwrap(), b"\"\\u0000\\u0007\\u001f\\u007f\"");
        assert_eq!(json_inspect("あ💩".as_bytes()).unwrap(), "\"あ💩\"".as_bytes());
        assert_eq!(json_inspect(b"'?").unwrap(), b"\"'?\"");
    }

    #[test]
    fn json_inspect_rejects_invalid_utf8() {
        assert!(json_inspect(b"\xff").is_none());
        assert!(json_inspect(&[0xed, 0xa0, 0x80]).is_none());
    }
}
