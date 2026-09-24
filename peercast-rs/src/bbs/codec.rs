//! 掲示板の文字コード: CP932 と EUC-JP の読み、Shift_JIS と EUC-JP の書き。
//!
//! もとの Python 版 (ui/cgi-bin の掲示板ビューワー) は標準ライブラリの `cp932`、`euc_jp`、`shift_jis`
//! の変換を使っていた。同じ結果になるように、変換表は Python 3.14 の変換から書き出したもの
//! (`data/*.bin`、書き出し方は `data/README.md`) を使い、読めないバイトの扱い (何バイトを U+FFFD
//! 1 つにするか) は CPython の `Modules/cjkcodecs/_codecs_jp.c` と同じにしている。

use std::sync::OnceLock;

/// 読めないバイトがあったとき
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errors {
    /// 誤りにする (`errors='strict'`)
    Strict,
    /// U+FFFD にする (`errors='replace'`)
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Cp932,
    EucJp,
    ShiftJis,
}

struct Tables {
    cp932_dec: Vec<(u32, u16)>,
    eucjp_dec: Vec<(u32, u16)>,
    sjis_enc: Vec<(u16, u32)>,
    eucjp_enc: Vec<(u16, u32)>,
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        cp932_dec: include_bytes!("data/cp932_dec.bin").chunks(4).map(|c| (u16le(c) as u32, u16le(&c[2..]))).collect(),
        eucjp_dec: include_bytes!("data/eucjp_dec.bin").chunks(6).map(|c| (u32le(c), u16le(&c[4..]))).collect(),
        sjis_enc: include_bytes!("data/sjis_enc.bin").chunks(4).map(|c| (u16le(c), u16le(&c[2..]) as u32)).collect(),
        eucjp_enc: include_bytes!("data/eucjp_enc.bin").chunks(6).map(|c| (u16le(c), u32le(&c[2..]))).collect(),
    })
}

fn find<K: Ord + Copy, V: Copy>(t: &[(K, V)], k: K) -> Option<V> {
    t.binary_search_by(|e| e.0.cmp(&k)).ok().map(|i| t[i].1)
}

fn push(out: &mut String, u: u16) {
    out.push(char::from_u32(u as u32).unwrap_or('\u{fffd}'));
}

/// 読む。`Strict` で読めないバイトがあれば `None`
pub fn decode(codec: Codec, b: &[u8], errors: Errors) -> Option<String> {
    let t = tables();
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        // 読めたら (文字, バイト数)、読めなければ Err(U+FFFD 1 つにするバイト数)
        let r: Result<(u16, usize), usize> = match codec {
            Codec::Cp932 | Codec::ShiftJis => {
                if let Some(u) = find(&t.cp932_dec, c as u32) {
                    Ok((u, 1))
                } else if i + 1 >= b.len() {
                    // incomplete multibyte sequence (残り全部)
                    Err(b.len() - i)
                } else {
                    match find(&t.cp932_dec, ((c as u32) << 8) | b[i + 1] as u32) {
                        Some(u) => Ok((u, 2)),
                        None => Err(1),
                    }
                }
            }
            Codec::EucJp => {
                if c < 0x80 {
                    Ok((c as u16, 1))
                } else if c == 0x8e {
                    if i + 1 >= b.len() {
                        Err(b.len() - i)
                    } else {
                        match find(&t.eucjp_dec, 0x8e00 | b[i + 1] as u32) {
                            Some(u) => Ok((u, 2)),
                            None => Err(1),
                        }
                    }
                } else if c == 0x8f {
                    if i + 2 >= b.len() {
                        Err(b.len() - i)
                    } else {
                        match find(&t.eucjp_dec, 0x8f0000 | ((b[i + 1] as u32) << 8) | b[i + 2] as u32) {
                            Some(u) => Ok((u, 3)),
                            None => Err(1),
                        }
                    }
                } else if i + 1 >= b.len() {
                    Err(b.len() - i)
                } else {
                    match find(&t.eucjp_dec, ((c as u32) << 8) | b[i + 1] as u32) {
                        Some(u) => Ok((u, 2)),
                        None => Err(1),
                    }
                }
            }
        };
        match r {
            Ok((u, n)) => {
                push(&mut out, u);
                i += n;
            }
            Err(n) => {
                if errors == Errors::Strict {
                    return None;
                }
                out.push('\u{fffd}');
                i += n;
            }
        }
    }
    Some(out)
}

/// 書く。書けない文字は `&#<10 進>;` にする (`errors='xmlcharrefreplace'`)
pub fn encode_xmlcharref(codec: Codec, s: &str) -> Vec<u8> {
    let t = tables();
    let table = match codec {
        Codec::ShiftJis | Codec::Cp932 => &t.sjis_enc,
        Codec::EucJp => &t.eucjp_enc,
    };
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        let v = if (ch as u32) < 0x10000 { find(table, ch as u32 as u16) } else { None };
        match v {
            Some(v) => {
                let bytes = v.to_be_bytes();
                let skip = bytes.iter().position(|&x| x != 0).unwrap_or(3);
                out.extend_from_slice(&bytes[skip..]);
            }
            None => out.extend_from_slice(format!("&#{};", ch as u32).as_bytes()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_basic() {
        assert_eq!(decode(Codec::Cp932, b"a\x82\xa0\x87\x40\xa0\xb1", Errors::Strict).as_deref(), Some("aあ①\u{f8f0}ｱ"));
        assert_eq!(decode(Codec::Cp932, b"\x82", Errors::Strict), None);
        assert_eq!(decode(Codec::Cp932, b"\x82\x20x", Errors::Replace).as_deref(), Some("\u{fffd} x"));
        assert_eq!(decode(Codec::EucJp, b"\xa4\xa2\x8e\xb1", Errors::Strict).as_deref(), Some("あｱ"));
        // 読めないバイトは 1 バイトずつ U+FFFD にする (途中で切れたものは残り全部で 1 つ)
        assert_eq!(decode(Codec::EucJp, b"\xa4\x20x", Errors::Replace).as_deref(), Some("\u{fffd} x"));
        assert_eq!(decode(Codec::EucJp, b"\x8f\xa1\xa1", Errors::Replace).as_deref(), Some("\u{fffd}\u{3000}"));
        assert_eq!(decode(Codec::EucJp, b"x\xa4", Errors::Replace).as_deref(), Some("x\u{fffd}"));
    }

    #[test]
    fn encode_basic() {
        assert_eq!(encode_xmlcharref(Codec::ShiftJis, "aあ〜😀"), b"a\x82\xa0\x81`&#128512;".to_vec());
        assert_eq!(encode_xmlcharref(Codec::EucJp, "あｱ"), b"\xa4\xa2\x8e\xb1".to_vec());
        // Shift_JIS (cp932 でない) では ～ (U+FF5E) と ① は書けない
        assert_eq!(encode_xmlcharref(Codec::ShiftJis, "～①"), b"&#65374;&#9312;".to_vec());
    }
}
