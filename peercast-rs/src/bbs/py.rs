//! もとの Python 版が使っていた標準ライブラリの関数のうち、結果が Rust の標準ライブラリと違うものを
//! 写したもの: `html.unescape`、`str.splitlines`、`str.isspace` (正規表現の `\s`)、`int(str)`。

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// `str.isspace` (正規表現の `\s` も同じ)。Rust の `char::is_whitespace` に U+001C〜U+001F を足したもの
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `str.splitlines()`: 行の区切りは \n、\r\n、\r、\v、\f、\x1c〜\x1e、\x85、U+2028、U+2029
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut res = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let is_break = matches!(c, '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if is_break {
            res.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(_, '\n')) = it.peek() {
                    it.next();
                    end += 1;
                }
            }
            start = end;
        }
    }
    if start < s.len() {
        res.push(&s[start..]);
    }
    res
}

/// `int(str)` (10 進)。前後の空白、符号、数字の間の `_` 1 つ、Unicode の数字 (Nd) のうち
/// 全角数字などよく使うもの。読めなければ `None`
pub fn int(s: &str) -> Option<i64> {
    let t = s.trim_matches(is_space);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'+') => (false, &t[1..]),
        Some(b'-') => (true, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    let mut prev_underscore = true; // 先頭の _ は不可
    for c in digits.chars() {
        if c == '_' {
            if prev_underscore {
                return None;
            }
            prev_underscore = true;
            continue;
        }
        let d = digit_value(c)?;
        v = v.checked_mul(10)?.checked_add(d as i64)?;
        prev_underscore = false;
    }
    if prev_underscore {
        return None;
    }
    Some(if neg { -v } else { v })
}

/// 10 進の数字 (Unicode の Nd のうち、ASCII、全角、アラビア・インド系など連続した 10 文字の組)
fn digit_value(c: char) -> Option<u32> {
    const ZEROS: [u32; 12] = [0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xe50, 0xff10];
    let u = c as u32;
    ZEROS.iter().find(|&&z| (z..z + 10).contains(&u)).map(|&z| u - z)
}

/// 10 進の数字か (`\d`)
pub fn is_digit(c: char) -> bool {
    digit_value(c).is_some()
}

struct Entities {
    html5: BTreeMap<String, String>,
    invalid_charrefs: BTreeMap<u32, String>,
    invalid_codepoints: Vec<u32>,
}

fn hex_chars(s: &str) -> String {
    s.split_whitespace().filter_map(|h| u32::from_str_radix(h, 16).ok()).filter_map(char::from_u32).collect()
}

fn entities() -> &'static Entities {
    static E: OnceLock<Entities> = OnceLock::new();
    E.get_or_init(|| {
        let mut html5 = BTreeMap::new();
        for line in include_str!("data/html5_entities.txt").lines() {
            if let Some((k, v)) = line.split_once('\t') {
                html5.insert(k.to_string(), hex_chars(v));
            }
        }
        let mut invalid_charrefs = BTreeMap::new();
        for line in include_str!("data/html_invalid_charrefs.txt").lines() {
            if let Some((k, v)) = line.split_once('\t') {
                if let Ok(k) = u32::from_str_radix(k, 16) {
                    invalid_charrefs.insert(k, hex_chars(v));
                }
            }
        }
        let invalid_codepoints = include_str!("data/html_invalid_codepoints.txt").lines().filter_map(|l| u32::from_str_radix(l.trim(), 16).ok()).collect();
        Entities { html5, invalid_charrefs, invalid_codepoints }
    })
}

/// `html.unescape`
pub fn html_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let e = entities();
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match charref(after) {
            Some(len) => {
                out.push_str(&replace_charref(e, &after[..len]));
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `&` の後ろが `#[0-9]+;?`、`#[xX][0-9a-fA-F]+;?`、`[^\t\n\f <&#;]{1,32};?` のどれかなら、その長さ (バイト)
fn charref(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() == Some(&b'#') {
        let (start, hex) = if matches!(b.get(1), Some(b'x') | Some(b'X')) { (2, true) } else { (1, false) };
        let n = b[start..].iter().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
        if n == 0 {
            return None;
        }
        let end = start + n;
        return Some(if b.get(end) == Some(&b';') { end + 1 } else { end });
    }
    let mut len = 0;
    for (count, (i, c)) in s.char_indices().enumerate() {
        if count >= 32 || matches!(c, '\t' | '\n' | '\x0c' | ' ' | '<' | '&' | '#' | ';') {
            break;
        }
        len = i + c.len_utf8();
    }
    if len == 0 {
        return None;
    }
    Some(if s.as_bytes().get(len) == Some(&b';') { len + 1 } else { len })
}

fn replace_charref(e: &Entities, s: &str) -> String {
    if let Some(num) = s.strip_prefix('#') {
        let num = num.trim_end_matches(';');
        let v = if let Some(h) = num.strip_prefix(['x', 'X']) {
            h.bytes().try_fold(0u64, |a, c| a.checked_mul(16)?.checked_add((c as char).to_digit(16)? as u64))
        } else {
            num.bytes().try_fold(0u64, |a, c| a.checked_mul(10)?.checked_add((c - b'0') as u64))
        };
        // 大きすぎるもの (Python は多倍長) も 0x10FFFF より大きいものとして扱う
        let v = v.unwrap_or(u64::MAX);
        if v <= u32::MAX as u64 {
            if let Some(r) = e.invalid_charrefs.get(&(v as u32)) {
                return r.clone();
            }
        }
        if (0xd800..=0xdfff).contains(&v) || v > 0x10ffff {
            return "\u{fffd}".to_string();
        }
        if e.invalid_codepoints.contains(&(v as u32)) {
            return String::new();
        }
        return char::from_u32(v as u32).map(String::from).unwrap_or_default();
    }
    if let Some(v) = e.html5.get(s) {
        return v.clone();
    }
    // 名前の前のほうで一致する、いちばん長いもの
    let idx: Vec<usize> = s.char_indices().map(|(i, _)| i).chain(std::iter::once(s.len())).collect();
    for k in (2..idx.len() - 1).rev() {
        let x = idx[k];
        if let Some(v) = e.html5.get(&s[..x]) {
            return format!("{}{}", v, &s[x..]);
        }
    }
    format!("&{}", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape() {
        assert_eq!(html_unescape("a&amp;b &lt;x&gt; &#39;&#x41;"), "a&b <x> 'A");
        assert_eq!(html_unescape("&notit;"), "¬it;");
        assert_eq!(html_unescape("&#128;&#0;&#55296;&#1;"), "€\u{fffd}\u{fffd}");
        assert_eq!(html_unescape("&zz; & &#;"), "&zz; & &#;");
    }

    #[test]
    fn lines_and_ints() {
        assert_eq!(splitlines("a\r\nb\rc\n\nd\u{2028}"), vec!["a", "b", "c", "", "d"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(int(" 12 "), Some(12));
        assert_eq!(int("1_000"), Some(1000));
        assert_eq!(int("1__0"), None);
        assert_eq!(int("０１２"), Some(12));
        assert_eq!(int(""), None);
    }
}
