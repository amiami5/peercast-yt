//! UTF-8 の検査と変換。C++ 版 core/common/str.cpp の
//! `validate_utf8`, `truncate_utf8`, `valid_utf8`, `codepoint_to_utf8` に相当する。
//!
//! C++ 版との違い:
//!  - 検査は RFC 3629 に沿って厳密に行う。C++ 版は先頭バイトと続きバイトの形しか見ておらず、
//!    過長表現 (`C0 80` など)、サロゲート (`ED A0 80`)、U+10FFFF を超えるもの (`F4 90 80 80`,
//!    `F5`〜`F7` 始まり) も「正しい」としていた。
//!  - `codepoint_to_utf8` は 2 バイトになる範囲 (U+0080〜U+07FF) を正しく符号化する。
//!    C++ 版は先頭バイトを `0xB0 | (cp >> 6)` としており (正しくは `0xC0`)、この範囲は
//!    全て不正な UTF-8 になっていた。サロゲートは符号化できない値として `None` を返す。

/// 厳密な UTF-8 かどうか。
pub fn validate(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

/// 先頭バイトの形だけから、その文字が何バイトかを決める。形が不正なら `None`。
fn sequence_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc0..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf7 => Some(4),
        _ => None, // 続きバイト (80〜BF) が先頭に来た、または F8 以上
    }
}

/// `bytes` の先頭から、合計 `limit` バイト以内に収まる完全な文字だけを取り出す。
/// 文字の途中で切ることはない。切り出す範囲に不正な UTF-8 があれば `None`。
///
/// C++ 版と同じく、上限に達したあとの部分は検査しない。
pub fn truncate(bytes: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut dest = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let n = sequence_len(bytes[i])?;
        if dest.len() + n > limit {
            break;
        }
        let seq = bytes.get(i..i + n)?;
        if n > 1 && std::str::from_utf8(seq).is_err() {
            return None;
        }
        dest.extend_from_slice(seq);
        i += n;
    }
    Some(dest)
}

/// UTF-8 として正しければそのまま返す。そうでなければ、ASCII 以外のバイトを `[XX]` に置き換える。
pub fn valid(bytes: &[u8]) -> Vec<u8> {
    if validate(bytes) {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b >= 0x80 {
            out.extend_from_slice(format!("[{:02X}]", b).as_bytes());
        } else {
            out.push(b);
        }
    }
    out
}

/// コードポイントを UTF-8 にする。Unicode のスカラー値でなければ (`>= 0x110000` かサロゲート) `None`。
pub fn codepoint_to_utf8(codepoint: u32) -> Option<Vec<u8>> {
    let c = char::from_u32(codepoint)?;
    let mut buf = [0u8; 4];
    Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- C++ 版の単体テスト (tests/str_unittest.cpp) を移したもの ----

    #[test]
    fn validate_utf8_cases_from_cpp_tests() {
        assert!(validate(b""));
        assert!(validate(b"\0"));
        assert!(validate(&[0xef, 0xbb, 0xbf])); // BOM
        assert!(validate(b"a"));
        assert!(validate("あ".as_bytes()));
        assert!(validate("💩".as_bytes()));
        assert!(validate("aあ💩".as_bytes()));
        assert!(!validate(b"\xff"));
        // Shift_JIS のバイト列
        assert!(!validate(b"\x95\\"));
        assert!(!validate(b"\x8A\xBF"));
        assert!(!validate(b"\x83A"));
        assert!(!validate(b"\xB1"));
    }

    #[test]
    fn truncate_utf8_cases_from_cpp_tests() {
        assert_eq!(truncate(b"", 1000).unwrap(), b"");
        assert_eq!(truncate(b"a", 0).unwrap(), b"");
        assert_eq!(truncate(b"a", 1).unwrap(), b"a");
        assert_eq!(truncate(b"a", 2).unwrap(), b"a");
        for (s, sz) in [("あ", 3usize), ("💩", 4)] {
            for limit in 0..sz {
                assert_eq!(truncate(s.as_bytes(), limit).unwrap(), b"", "{} {}", s, limit);
            }
            assert_eq!(truncate(s.as_bytes(), sz).unwrap(), s.as_bytes());
            assert_eq!(truncate(s.as_bytes(), sz + 1).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn codepoint_to_utf8_cases_from_cpp_tests() {
        assert_eq!(codepoint_to_utf8(0x20).unwrap(), b" ");
        assert_eq!(codepoint_to_utf8(12354).unwrap(), "あ".as_bytes());
        assert_eq!(codepoint_to_utf8(0x1f4a9).unwrap(), "💩".as_bytes());
    }

    // ---- C++ 版から意図的に変えたところ ----

    #[test]
    fn strict_validation_rejects_what_cpp_accepted() {
        assert!(!validate(&[0xc0, 0x80])); // 過長表現の NUL
        assert!(!validate(&[0xc0, 0xae])); // 過長表現の '.'
        assert!(!validate(&[0xe0, 0x80, 0xaf])); // 過長表現の '/'
        assert!(!validate(&[0xed, 0xa0, 0x80])); // U+D800 (サロゲート)
        assert!(!validate(&[0xf4, 0x90, 0x80, 0x80])); // U+110000
        assert!(!validate(&[0xf5, 0x80, 0x80, 0x80]));
    }

    #[test]
    fn two_byte_codepoints_are_encoded_correctly() {
        // C++ 版は U+00A9 を B2 A9 (不正) にしていた。
        assert_eq!(codepoint_to_utf8(0xa9).unwrap(), &[0xc2, 0xa9]);
        assert_eq!(codepoint_to_utf8(0x410).unwrap(), "А".as_bytes()); // キリル文字
        assert_eq!(codepoint_to_utf8(0x7ff).unwrap(), &[0xdf, 0xbf]);
        assert_eq!(codepoint_to_utf8(0x80).unwrap(), &[0xc2, 0x80]);
    }

    #[test]
    fn every_scalar_value_roundtrips() {
        for cp in (0..0x110000u32).step_by(7).chain([0xd7ff, 0xe000, 0xffff, 0x10ffff]) {
            match char::from_u32(cp) {
                Some(c) => assert_eq!(codepoint_to_utf8(cp).unwrap(), c.to_string().as_bytes()),
                None => assert!(codepoint_to_utf8(cp).is_none()),
            }
        }
        assert!(codepoint_to_utf8(0xd800).is_none());
        assert!(codepoint_to_utf8(0x110000).is_none());
        assert!(codepoint_to_utf8(u32::MAX).is_none());
    }

    #[test]
    fn truncate_never_splits_a_character_and_output_is_valid() {
        let s = "aあ💩bい".as_bytes();
        for limit in 0..=s.len() + 2 {
            let t = truncate(s, limit).unwrap();
            assert!(t.len() <= limit);
            assert!(validate(&t));
            assert!(s.starts_with(&t));
        }
    }

    #[test]
    fn truncate_rejects_invalid_sequences_in_range() {
        assert!(truncate(&[0x80], 10).is_none()); // 続きバイトから始まる
        assert!(truncate(&[0xff], 10).is_none());
        assert!(truncate(&[0xe3, 0x81], 10).is_none()); // 途中で終わる
        assert!(truncate(&[0xc0, 0x80], 10).is_none()); // 過長表現
        // 上限で打ち切った場所より後ろは検査しない (C++ 版と同じ)
        assert_eq!(truncate(b"ab\xf0\x9f\x92\xa9\xff", 3).unwrap(), b"ab");
        // ただし先頭バイトが不正な形なら、上限の手前でも見つかり次第エラー (C++ 版と同じ)
        assert!(truncate(b"ab\xff", 2).is_none());
        // 先頭バイトの形で長さを決めてから上限と比べる (C++ 版と同じ)
        assert_eq!(truncate(&[0xe3], 1).unwrap(), b"");
    }

    #[test]
    fn valid_escapes_non_ascii_bytes() {
        assert_eq!(valid(b"abc"), b"abc");
        assert_eq!(valid("あ".as_bytes()), "あ".as_bytes());
        assert_eq!(valid(b"a\x95\\b"), b"a[95]\\b");
        assert_eq!(valid(&[0xff, 0x00, 0xfe]), b"[FF]\0[FE]");
    }
}
