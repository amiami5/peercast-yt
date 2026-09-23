//! C++ 版 `String` クラス (core/common/_string.cpp) の変換関数。
//!
//! C++ の `String` は 256 バイト固定長の `char data[MAX_LEN]` を持つ。ここの関数は、その
//! `data` に書き込む内容 (終端の NUL は含まない) をバイト列で返す。長さの上限は C++ 版と
//! 同じにしてあるので、返り値は必ず `MAX_LEN - 1` バイト以下になる。
//!
//! 入力は C++ 側の NUL 終端文字列の中身 (NUL の手前まで)。C++ 版は、入力の末尾で
//! 途中までしかない `%XX` や UTF-8 の文字に出会うと、終端の NUL を読み飛ばしてその先の
//! メモリまで読み進めていた。Rust 版は「入力の後ろには NUL が続いている」とみなして
//! 計算し、入力の終わりで止まる (詳しくは README の「C++ 版との違い」)。
//! 結果は、C++ 版で入力の後ろのメモリが 0 で埋まっていた場合と同じになる。

use crate::utf8;

/// `String::MAX_LEN`
pub const MAX_LEN: usize = 256;

/// `ASCII2ESC` などが書き込みをやめる位置 (`data + MAX_LEN - 10`)
const OUT_LIMIT: usize = MAX_LEN - 10;

fn at(input: &[u8], i: usize) -> u8 {
    input.get(i).copied().unwrap_or(0)
}

fn is_plain_ascii(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// `String::ASCII2ESC`: 英数字以外を `%XX` (`safe` なら `%%XX`) にする。
pub fn ascii_to_esc(input: &[u8], safe: bool) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::new();
    for &c in input {
        if is_plain_ascii(c) {
            out.push(c);
        } else {
            out.push(b'%');
            if safe {
                out.push(b'%');
            }
            out.push(HEX[(c >> 4) as usize]);
            out.push(HEX[(c & 0xf) as usize]);
        }
        if out.len() >= OUT_LIMIT {
            break;
        }
    }
    out
}

/// C++ の `TONIBBLE(TOUPPER(c))`。16 進数字でなくても、そのまま引き算した値を返す
/// (C++ 版と同じ)。
///
/// C++ 版は `char` で計算しており、0x80 以上のバイトの結果が CPU によって違った
/// (`char` は x86 では符号付き、ARM の Linux では符号なし)。Rust 版は CPU によらず
/// 符号付きとして計算する (x86 の C++ 版と同じ結果)。
fn nibble(c: u8) -> i32 {
    let c = c.to_ascii_uppercase() as i8 as i32;
    if (b'A' as i32..=b'F' as i32).contains(&c) {
        c - b'A' as i32 + 10
    } else {
        c - b'0' as i32
    }
}

/// `String::ESC2ASCII`: `+` を空白に、`%XX` (`%%XX` も) をそのバイトに戻す。
///
/// C++ 版と同じく、`%` の後ろが 16 進数字でなくても 2 文字を読んで計算する
/// (`%zz` は意味のないバイトになる)。
pub fn esc_to_ascii(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < input.len() {
        let mut c = input[p];
        p += 1;
        if c == b'+' {
            c = b' ';
        } else if c == b'%' {
            if at(input, p) == b'%' {
                p += 1;
            }
            let hi = nibble(at(input, p));
            let lo = nibble(at(input, p + 1));
            c = ((hi << 4) | lo) as u8;
            p += 2;
        }
        out.push(c);
        if out.len() >= OUT_LIMIT {
            break;
        }
    }
    out
}

/// `String::ASCII2META`: `;` を `:` に、`safe` なら `%` を `%%` にする。
pub fn ascii_to_meta(input: &[u8], safe: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for &c in input {
        let c = match c {
            b'%' => {
                if safe {
                    out.push(b'%');
                }
                c
            }
            b';' => b':',
            _ => c,
        };
        out.push(c);
        if out.len() >= OUT_LIMIT {
            break;
        }
    }
    out
}

/// C++ の `base64chartoval`。`=` は -1、それ以外の範囲外は -2。
fn base64_value(c: u8) -> i32 {
    match c {
        b'A'..=b'Z' => (c - b'A') as i32,
        b'a'..=b'z' => (c - b'a') as i32 + 26,
        b'0'..=b'9' => (c - b'0') as i32 + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => -1,
        _ => -2,
    }
}

/// `String::base64WordToChars`: base64 の 4 文字を 3 バイトにする。不正な文字が
/// あれば `None`。
///
/// C++ 版と同じく、`=` (パディング) の位置には 0 を出力し、常に 3 バイト返す。
/// 3 文字目が `=` で 4 文字目がそうでない場合も、C++ 版と同じ値 (`-1 & 3` を使う) を返す。
pub fn base64_word_to_chars(word: &[u8; 4]) -> Option<[u8; 3]> {
    let v = word.map(base64_value);
    if v[0] < 0 || v[1] < 0 || v[2] < -1 || v[3] < -1 {
        return None;
    }
    let b0 = (v[0] << 2) | (v[1] >> 4);
    let b1 = if v[2] >= 0 { ((v[1] & 0x0f) << 4) | (v[2] >> 2) } else { 0 };
    let b2 = if v[3] >= 0 { ((v[2] & 0x03) << 6) | v[3] } else { 0 };
    Some([b0 as u8, b1 as u8, b2 as u8])
}

/// `String::BASE642ASCII`: 4 文字ずつ復号する。不正な 4 文字は読み飛ばし、
/// 端数 (4 文字に満たない末尾) は捨てる。出力には 0 のバイトが含まれうる。
pub fn base64_to_ascii(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for word in input.chunks_exact(4) {
        if let Some(bytes) = base64_word_to_chars(&[word[0], word[1], word[2], word[3]]) {
            out.extend_from_slice(&bytes);
        }
    }
    // C++ 版は data (MAX_LEN バイト) に上限なしで書き込んでいた。入力が MAX_LEN - 1
    // バイト以下なら出力は 189 バイト以下で収まるが、念のためここで切り詰める。
    out.truncate(MAX_LEN - 1);
    out
}

fn is_sjis(a: u8, b: u8) -> bool {
    ((0x81..=0x9f).contains(&a) || (0xe0..=0xfc).contains(&a))
        && ((0x40..=0x7e).contains(&b) || (0x80..=0xfc).contains(&b))
}

fn is_euc(a: u8) -> bool {
    (0xa1..=0xfe).contains(&a)
}

fn is_utf8_lead(a: u8, b: u8) -> bool {
    (a & 0xc0) == 0xc0 && (b & 0x80) == 0x80
}

fn push_codepoint(buf: &mut Vec<u8>, cp: u32) {
    // jis の変換表と 1 バイトの値 (U+0000〜U+00FF) からしか呼ばれないので、
    // サロゲートなどの符号化できない値は来ない。来たら U+FFFD にする (C++ 版は例外)。
    buf.extend(utf8::codepoint_to_utf8(cp).unwrap_or_else(|| "\u{fffd}".as_bytes().to_vec()));
}

/// `String::UNKNOWN2UNICODE`: 文字コードの分からない文字列を、1 文字ずつ UTF-8,
/// Shift_JIS, EUC-JP, それ以外 (Latin-1 とみなす) のどれかと推測して UTF-8 にする。
/// `safe` なら HTML の特殊文字を実体参照にする。
pub fn unknown_to_unicode(input: &[u8], safe: bool) -> Vec<u8> {
    let mut utf8_out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        i += 1;
        let d = at(input, i);
        let mut buf = Vec::new();

        if is_utf8_lead(c, d) {
            let num_chars = (0..6).take_while(|&k| c & (0x80 >> k) != 0).count();
            buf.push(c);
            for _ in 0..num_chars - 1 {
                // C++ 版は入力の終わりを越えて読んでいた。ここでは 0 とみなす。
                buf.push(at(input, i));
                i += 1;
            }
        } else if is_sjis(c, d) {
            push_codepoint(&mut buf, crate::jis::sjis_to_unicode(((c as u16) << 8) | d as u16) as u32);
            i += 1;
        } else if is_euc(c) && is_euc(d) {
            push_codepoint(&mut buf, crate::jis::euc_to_unicode(((c as u16) << 8) | d as u16) as u32);
            i += 1;
        } else if is_plain_ascii(c) {
            buf.push(c);
        } else if safe && matches!(c, b'&' | b'"' | b'\'' | b'<' | b'>') {
            buf.extend_from_slice(match c {
                b'&' => b"&amp;".as_slice(),
                b'"' => b"&quot;",
                b'\'' => b"&#039;",
                b'<' => b"&lt;",
                _ => b"&gt;",
            });
        } else {
            push_codepoint(&mut buf, c as u32);
        }

        if utf8_out.len() + buf.len() < MAX_LEN {
            utf8_out.extend_from_slice(&buf);
        } else {
            break;
        }
    }

    // C++ 版は結果を strncpy で data に写すので、途中の NUL で終わる。
    if let Some(nul) = utf8_out.iter().position(|&b| b == 0) {
        utf8_out.truncate(nul);
    }
    utf8_out
}

/// `String::setFromString`: 最初の語 (空白区切り) を取り出す。`"` で囲まれた部分は
/// 空白を含めて 1 語とする。先頭の空白は読み飛ばす。
pub fn from_string(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut quote = false;
    for &c in input {
        let add = match c {
            b'"' => {
                if quote {
                    break;
                }
                quote = true;
                false
            }
            b' ' if !quote => {
                if !out.is_empty() {
                    break;
                }
                false
            }
            _ => true,
        };
        if add {
            out.push(c);
            if out.len() >= MAX_LEN - 1 {
                break;
            }
        }
    }
    out
}

/// `String::setUnquote`: 最初と最後の 1 文字を取り除く。2 文字以下なら空。
pub fn unquote(input: &[u8]) -> Vec<u8> {
    let slen = input.len().min(MAX_LEN);
    if slen > 2 {
        input[1..slen - 1].to_vec()
    } else {
        Vec::new()
    }
}

/// `String::setFromStopwatch`: 経過秒数を "1 day, 2 hour" のような文字列にする。
pub fn from_stopwatch(t: u32) -> Vec<u8> {
    let sec = t % 60;
    let min = (t / 60) % 60;
    let hour = (t / 3600) % 24;
    let day = t / 86400;

    let s = if day != 0 {
        format!("{} day, {} hour", day, hour)
    } else if hour != 0 {
        format!("{} hour, {} min", hour, min)
    } else if min != 0 {
        format!("{} min, {} sec", min, sec)
    } else if sec != 0 {
        format!("{} sec", sec)
    } else {
        "-".to_string()
    };
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_roundtrip() {
        assert_eq!(ascii_to_esc(b"a b/c", false), b"a%20b%2Fc");
        assert_eq!(ascii_to_esc(b"a b", true), b"a%%20b");
        assert_eq!(esc_to_ascii(b"a%20b%2fc+d"), b"a b/c d");
        assert_eq!(esc_to_ascii(b"a%%20b"), b"a b");
    }

    #[test]
    fn esc_to_ascii_incomplete_escape_stops_at_end() {
        // C++ 版は終端を越えて読んでいた。Rust 版は後ろを 0 とみなして止まる。
        assert_eq!(esc_to_ascii(b"ab%"), b"ab\xd0");
        assert_eq!(esc_to_ascii(b"ab%4"), b"ab\xd0");
        assert_eq!(esc_to_ascii(b"%%"), b"\xd0");
    }

    #[test]
    fn esc_output_is_bounded() {
        let long = vec![b' '; 1000];
        let out = ascii_to_esc(&long, true);
        assert!(out.len() < MAX_LEN);
        assert!(esc_to_ascii(&long).len() < MAX_LEN);
        assert!(ascii_to_meta(&vec![b'%'; 1000], true).len() < MAX_LEN);
    }

    #[test]
    fn meta() {
        assert_eq!(ascii_to_meta(b"a;b%c", false), b"a:b%c");
        assert_eq!(ascii_to_meta(b"a;b%c", true), b"a:b%%c");
    }

    #[test]
    fn base64() {
        assert_eq!(base64_to_ascii(b"aGVsbG8="), b"hello\0");
        assert_eq!(base64_to_ascii(b"YWJj"), b"abc");
        assert_eq!(base64_to_ascii(b"YWJ"), b"");
        assert_eq!(base64_to_ascii(b"!!!!YWJj"), b"abc");
        assert_eq!(base64_word_to_chars(b"YQ=="), Some([b'a', 0, 0]));
        assert_eq!(base64_word_to_chars(b"Y==="), None);
    }

    #[test]
    fn unknown_to_unicode_basic() {
        assert_eq!(unknown_to_unicode(b"abc", false), b"abc");
        assert_eq!(unknown_to_unicode("日本".as_bytes(), false), "日本".as_bytes());
        assert_eq!(unknown_to_unicode(b"<&>", true), b"&lt;&amp;&gt;");
        assert_eq!(unknown_to_unicode(b"<&>", false), b"<&>");
        // Latin-1 とみなされる
        assert_eq!(unknown_to_unicode(b"\xa9", false), "\u{a9}".as_bytes());
    }

    #[test]
    fn unknown_to_unicode_truncated_utf8_stops_at_end() {
        assert_eq!(unknown_to_unicode(b"a\xe3\x81", false), b"a\xe3\x81");
        assert_eq!(unknown_to_unicode(b"\xfc\x80", false), b"\xfc\x80");
    }

    #[test]
    fn unknown_to_unicode_is_bounded() {
        let out = unknown_to_unicode(&[0xa9; 1000], true);
        assert!(out.len() < MAX_LEN);
    }

    #[test]
    fn from_string_and_unquote() {
        assert_eq!(from_string(b"abc def"), b"abc");
        assert_eq!(from_string(b"\"abc def\""), b"abc def");
        assert_eq!(from_string(b"   abc"), b"abc");
        assert_eq!(from_string(&[b'x'; 1000]).len(), MAX_LEN - 1);
        assert_eq!(unquote(b"\"abc\""), b"abc");
        assert_eq!(unquote(b"a"), b"");
        assert_eq!(unquote(&[b'x'; 1000]).len(), MAX_LEN - 2);
    }

    #[test]
    fn stopwatch() {
        assert_eq!(from_stopwatch(86400), b"1 day, 0 hour");
        assert_eq!(from_stopwatch(3600), b"1 hour, 0 min");
        assert_eq!(from_stopwatch(60), b"1 min, 0 sec");
        assert_eq!(from_stopwatch(1), b"1 sec");
        assert_eq!(from_stopwatch(0), b"-");
    }
}
