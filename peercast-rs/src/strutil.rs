//! 文字列ユーティリティ。C++ 版 core/common/str.cpp のうち、UTF-8/HTML エスケープ以外の
//! 関数に相当する (それらは utf8.rs / inspect.rs / cgi.rs / url.rs にある)。
//!
//! バイト列として扱う。`isalpha` / `toupper` / `tolower` / `isprint` は C ロケールの規則
//! (ASCII の範囲だけを対象にし、0x80 以上は対象外) に合わせている。

/// C ロケールでの `isalpha`。
fn is_alpha(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_print(b: u8) -> bool {
    (0x20..=0x7e).contains(&b)
}

/// 空白とみなす文字。C++ 版の `strip`/`rstrip` と同じ (`isspace` ではなく手書きの範囲)。
fn is_strip_space(b: u8) -> bool {
    b == b' ' || (0x09..=0x0d).contains(&b) || b == 0
}

/// 大文字16進数で1バイトずつ空白区切り (先頭には付けない)。C++版の `%02hhX` による実装と同じ。
pub fn hexdump(input: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(input.len() * 3);
    for (i, &b) in input.iter().enumerate() {
        if i != 0 {
            out.push(b' ');
        }
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 0xf) as usize]);
    }
    out
}

pub fn repeat(input: &[u8], n: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        out.extend_from_slice(input);
    }
    out
}

/// 整数部を3桁ごとに区切る。小数点 (最初に現れる `.`) より後ろはそのまま。
pub fn group_digits(input: &[u8], separator: &[u8]) -> Vec<u8> {
    let dot = input.iter().position(|&b| b == b'.').unwrap_or(input.len());
    let (integer, tail) = input.split_at(dot);
    let mut out = Vec::with_capacity(input.len() + input.len() / 3);
    for (i, &b) in integer.iter().enumerate() {
        if i != 0 && (integer.len() - i) % 3 == 0 {
            out.extend_from_slice(separator);
        }
        out.push(b);
    }
    out.extend_from_slice(tail);
    out
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `separator` で分割する (上限なし)。
///
/// C++ 版との違い: 区切りが空文字列だと、C++ 版は `strstr` が毎回先頭で即マッチして `p` が
/// 全く進まないため、無限に要素を積み続けてメモリを使い果たす (実際に確認した既知の不具合。
/// `str::split(in, "", limit)` の形の `limit` 付き版は `limit` で打ち切られるので影響しない)。
/// Rust 版は、区切りが空文字列のときは入力全体を1要素として返す。
pub fn split(input: &[u8], separator: &[u8]) -> Vec<Vec<u8>> {
    if separator.is_empty() {
        return vec![input.to_vec()];
    }
    split_limit(input, separator, i32::MAX).unwrap()
}

/// `limit <= 0` は C++ 版と同じく呼び出し側のエラー (`None` を返す)。
/// 区切りが空文字列でも、C++ 版と同じく `limit` で打ち切られるので無限ループにはならない
/// (`p` が進まないまま、空文字列の要素を `limit - 1` 個積んでから残り全体を返す)。
pub fn split_limit(input: &[u8], separator: &[u8], limit: i32) -> Option<Vec<Vec<u8>>> {
    if limit <= 0 {
        return None;
    }
    let mut res = Vec::new();
    let mut p = 0usize;
    loop {
        let rest = &input[p..];
        match find_sub(rest, separator) {
            Some(q) if (res.len() as i64) < (limit as i64) - 1 => {
                res.push(rest[..q].to_vec());
                p += q + separator.len();
            }
            _ => {
                res.push(rest.to_vec());
                return Some(res);
            }
        }
    }
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find_sub(haystack, needle).is_some()
}

pub fn replace_prefix(s: &[u8], prefix: &[u8], replacement: &[u8]) -> Vec<u8> {
    if s.starts_with(prefix) {
        let mut out = replacement.to_vec();
        out.extend_from_slice(&s[prefix.len()..]);
        out
    } else {
        s.to_vec()
    }
}

pub fn replace_suffix(s: &[u8], suffix: &[u8], replacement: &[u8]) -> Vec<u8> {
    if s.len() >= suffix.len() && &s[s.len() - suffix.len()..] == suffix {
        let mut out = s[..s.len() - suffix.len()].to_vec();
        out.extend_from_slice(replacement);
        out
    } else {
        s.to_vec()
    }
}

pub fn upcase(input: &[u8]) -> Vec<u8> {
    input.iter().map(|&b| if is_alpha(b) { b.to_ascii_uppercase() } else { b }).collect()
}

pub fn downcase(input: &[u8]) -> Vec<u8> {
    input.iter().map(|&b| if is_alpha(b) { b.to_ascii_lowercase() } else { b }).collect()
}

/// 単語の先頭だけ大文字にし、残りは小文字にする。「単語」は英字の連続。
pub fn capitalize(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut prev_was_alpha = false;
    for &b in input {
        if is_alpha(b) {
            if prev_was_alpha {
                out.push(b.to_ascii_lowercase());
            } else {
                out.push(b.to_ascii_uppercase());
                prev_was_alpha = true;
            }
        } else {
            out.push(b);
            prev_was_alpha = false;
        }
    }
    out
}

pub fn has_prefix(subject: &[u8], prefix: &[u8]) -> bool {
    subject.starts_with(prefix)
}

pub fn has_suffix(subject: &[u8], suffix: &[u8]) -> bool {
    subject.ends_with(suffix)
}

pub fn join(delimiter: &[u8], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i != 0 {
            out.extend_from_slice(delimiter);
        }
        out.extend_from_slice(p);
    }
    out
}

/// 表示可能な文字 (0x20〜0x7e) 以外を `replacement` に置き換える。
pub fn ascii_dump(input: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &b in input {
        if is_print(b) {
            out.push(b);
        } else {
            out.extend_from_slice(replacement);
        }
    }
    out
}

/// 最後の `.` より後ろ。`.` が無ければ空。
pub fn extension_without_dot(filename: &[u8]) -> Vec<u8> {
    match filename.iter().rposition(|&b| b == b'.') {
        Some(i) => filename[i + 1..].to_vec(),
        None => Vec::new(),
    }
}

/// 重なりを許して数える (1 文字ずつ進めて探す)。`needle` が空なら `None` (呼び出し側のエラー)。
pub fn count(haystack: &[u8], needle: &[u8]) -> Option<i32> {
    if needle.is_empty() {
        return None;
    }
    let mut n = 0i32;
    let mut start = 0usize;
    while start <= haystack.len() {
        match find_sub(&haystack[start..], needle) {
            Some(q) => {
                n += 1;
                start += q + 1;
            }
            None => break,
        }
    }
    Some(n)
}

pub fn rstrip(input: &[u8]) -> Vec<u8> {
    let mut end = input.len();
    while end > 0 && is_strip_space(input[end - 1]) {
        end -= 1;
    }
    input[..end].to_vec()
}

pub fn strip(input: &[u8]) -> Vec<u8> {
    let mut start = 0;
    while start < input.len() && is_strip_space(input[start]) {
        start += 1;
    }
    let mut end = input.len();
    while end > start && is_strip_space(input[end - 1]) {
        end -= 1;
    }
    input[start..end].to_vec()
}

/// シェルの単一引用符で囲む (`'` は `'"'"'` にする)。
pub fn escapeshellarg_unix(input: &[u8]) -> Vec<u8> {
    let mut out = vec![b'\''];
    for &b in input {
        if b == b'\'' {
            out.extend_from_slice(b"'\"'\"'");
        } else {
            out.push(b);
        }
    }
    out.push(b'\'');
    out
}

/// 改行 (`\n`) を含めたまま行に分ける。末尾の空行 (文字列が `\n` で終わる場合) は含めない。
pub fn to_lines(text: &[u8]) -> Vec<Vec<u8>> {
    let mut res = Vec::new();
    let mut line = Vec::new();
    for &b in text {
        line.push(b);
        if b == b'\n' {
            res.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        res.push(line);
    }
    res
}

/// 各行の先頭にタブを `n` 個つける。`n < 0` は呼び出し側のエラー (`None`)。
pub fn indent_tab(text: &[u8], n: i32) -> Option<Vec<u8>> {
    if n < 0 {
        return None;
    }
    let space = repeat(b"\t", n);
    let mut out = Vec::new();
    for line in to_lines(text) {
        out.extend_from_slice(&space);
        out.extend_from_slice(&line);
    }
    Some(out)
}

/// シェルの単語分割。エラーメッセージは C++ 版の `FormatException` の文言と同じにしてある。
pub fn shellwords(input: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
    let mut words = Vec::new();
    let mut curr = Vec::new();
    let mut allow_empty = false;
    let mut i = 0;
    let n = input.len();

    while i < n {
        match input[i] {
            b' ' | b'\t' | 0x0b => {
                if allow_empty || !curr.is_empty() {
                    allow_empty = false;
                    words.push(std::mem::take(&mut curr));
                }
                i += 1;
            }
            b'\'' => {
                i += 1;
                loop {
                    if i >= n {
                        return Err("Unterminated single-quoted string");
                    }
                    if input[i] == b'\'' {
                        allow_empty = true;
                        i += 1;
                        break;
                    }
                    curr.push(input[i]);
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                loop {
                    if i >= n {
                        return Err("Unterminated double-quoted string");
                    }
                    match input[i] {
                        b'"' => {
                            allow_empty = true;
                            i += 1;
                            break;
                        }
                        b'\\' => {
                            i += 1;
                            if i >= n {
                                return Err("Unfinished backlash escape");
                            }
                            if input[i] != b'"' && input[i] != b'\\' {
                                curr.push(b'\\');
                            }
                            curr.push(input[i]);
                            i += 1;
                        }
                        c => {
                            curr.push(c);
                            i += 1;
                        }
                    }
                }
            }
            b'\\' => {
                i += 1;
                if i >= n {
                    return Err("Unfinished backlash escape");
                }
                curr.push(input[i]);
                i += 1;
            }
            c => {
                curr.push(c);
                i += 1;
            }
        }
    }

    if allow_empty || !curr.is_empty() {
        words.push(curr);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<Vec<u8>> {
        s.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    // ---- C++ 版の単体テスト (tests/str_unittest.cpp) を移したもの ----

    #[test]
    fn hexdump_cases() {
        assert_eq!(hexdump(b""), b"");
        assert_eq!(hexdump(b"\x00"), b"00");
        assert_eq!(hexdump(b"\x00\x01\xff\xab"), b"00 01 FF AB");
    }

    #[test]
    fn repeat_cases() {
        assert_eq!(repeat(b"ab", 0), b"");
        assert_eq!(repeat(b"ab", 3), b"ababab");
        assert_eq!(repeat(b"", 5), b"");
    }

    #[test]
    fn group_digits_cases() {
        assert_eq!(group_digits(b"1", b","), b"1");
        assert_eq!(group_digits(b"12", b","), b"12");
        assert_eq!(group_digits(b"123", b","), b"123");
        assert_eq!(group_digits(b"1234", b","), b"1,234");
        assert_eq!(group_digits(b"1234567", b","), b"1,234,567");
        assert_eq!(group_digits(b"1234.5678", b","), b"1,234.5678");
        assert_eq!(group_digits(b"", b","), b"");
        assert_eq!(group_digits(b"1000", b"_"), b"1_000");
    }

    #[test]
    fn split_cases() {
        assert_eq!(split(b"a,b,c", b","), v(&["a", "b", "c"]));
        assert_eq!(split(b"", b","), v(&[""]));
        assert_eq!(split(b"a", b","), v(&["a"]));
        assert_eq!(split(b",,", b","), v(&["", "", ""]));
        assert_eq!(split(b"aXXbXXc", b"XX"), v(&["a", "b", "c"]));
    }

    #[test]
    fn split_limit_cases() {
        assert_eq!(split_limit(b"a,b,c", b",", 2).unwrap(), v(&["a", "b,c"]));
        assert_eq!(split_limit(b"a,b,c", b",", 1).unwrap(), v(&["a,b,c"]));
        assert_eq!(split_limit(b"a,b,c", b",", 10).unwrap(), v(&["a", "b", "c"]));
        assert!(split_limit(b"a", b",", 0).is_none());
        assert!(split_limit(b"a", b",", -1).is_none());
    }

    #[test]
    fn contains_cases() {
        assert!(contains(b"hello world", b"wor"));
        assert!(contains(b"x", b""));
        assert!(!contains(b"hello", b"xyz"));
    }

    #[test]
    fn replace_prefix_suffix_cases() {
        assert_eq!(replace_prefix(b"foobar", b"foo", b"baz"), b"bazbar");
        assert_eq!(replace_prefix(b"foobar", b"xxx", b"baz"), b"foobar");
        assert_eq!(replace_prefix(b"fo", b"foo", b"baz"), b"fo");
        assert_eq!(replace_suffix(b"foobar", b"bar", b"baz"), b"foobaz");
        assert_eq!(replace_suffix(b"foobar", b"xxx", b"baz"), b"foobar");
    }

    #[test]
    fn case_conversion_cases() {
        assert_eq!(upcase(b"aB3_z"), b"AB3_Z");
        assert_eq!(downcase(b"aB3_Z"), b"ab3_z");
        assert_eq!(capitalize(b"hello world"), b"Hello World");
        assert_eq!(capitalize(b"HELLO"), b"Hello");
        assert_eq!(capitalize(b""), b"");
    }

    #[test]
    fn prefix_suffix_cases() {
        assert!(has_prefix(b"hello", b"he"));
        assert!(!has_prefix(b"he", b"hello"));
        assert!(has_suffix(b"hello", b"lo"));
        assert!(!has_suffix(b"lo", b"hello"));
    }

    #[test]
    fn join_cases() {
        assert_eq!(join(b",", &v(&["a", "b", "c"])), b"a,b,c");
        assert_eq!(join(b",", &v(&[])), b"");
        assert_eq!(join(b",", &v(&["a"])), b"a");
    }

    #[test]
    fn ascii_dump_cases() {
        assert_eq!(ascii_dump(b"a\x01b\xffc", b"."), b"a.b.c");
        assert_eq!(ascii_dump(b"abc", b"."), b"abc");
    }

    #[test]
    fn extension_without_dot_cases() {
        assert_eq!(extension_without_dot(b"a.txt"), b"txt");
        assert_eq!(extension_without_dot(b"a.tar.gz"), b"gz");
        assert_eq!(extension_without_dot(b"noext"), b"");
    }

    #[test]
    fn count_cases() {
        assert_eq!(count(b"aaaa", b"aa").unwrap(), 3); // 重なりを許す
        assert_eq!(count(b"abcabc", b"abc").unwrap(), 2);
        assert_eq!(count(b"abc", b"xyz").unwrap(), 0);
        assert!(count(b"abc", b"").is_none());
    }

    #[test]
    fn strip_cases() {
        assert_eq!(strip(b"  hi  "), b"hi");
        assert_eq!(rstrip(b"  hi  "), b"  hi");
        assert_eq!(strip(b"\t\r\n hi \0"), b"hi");
        assert_eq!(strip(b"   "), b"");
    }

    #[test]
    fn escapeshellarg_unix_cases() {
        assert_eq!(escapeshellarg_unix(b"hello"), b"'hello'");
        assert_eq!(escapeshellarg_unix(b"it's"), b"'it'\"'\"'s'");
    }

    #[test]
    fn to_lines_cases() {
        assert_eq!(to_lines(b"a\nb\nc"), v(&["a\n", "b\n", "c"]));
        assert_eq!(to_lines(b"a\nb\n"), v(&["a\n", "b\n"]));
        assert_eq!(to_lines(b""), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn indent_tab_cases() {
        assert_eq!(indent_tab(b"a\nb\n", 1).unwrap(), b"\ta\n\tb\n");
        assert_eq!(indent_tab(b"a\n", 2).unwrap(), b"\t\ta\n");
        assert_eq!(indent_tab(b"a\n", 0).unwrap(), b"a\n");
        assert!(indent_tab(b"a", -1).is_none());
    }

    #[test]
    fn shellwords_cases() {
        assert_eq!(shellwords(b"a b c").unwrap(), v(&["a", "b", "c"]));
        assert_eq!(shellwords(b"  a   b  ").unwrap(), v(&["a", "b"]));
        assert_eq!(shellwords(b"'a b' c").unwrap(), v(&["a b", "c"]));
        assert_eq!(shellwords(b"\"a b\" c").unwrap(), v(&["a b", "c"]));
        assert_eq!(shellwords(b"a\\ b").unwrap(), v(&["a b"]));
        assert_eq!(shellwords(b"\"a\\\"b\"").unwrap(), v(&["a\"b"]));
        assert_eq!(shellwords(b"\"a\\\\b\"").unwrap(), v(&["a\\b"]));
        assert_eq!(shellwords(b"\"a\\nb\"").unwrap(), v(&["a\\nb"])); // \n はそのまま \n (バックスラッシュも残る)
        assert_eq!(shellwords(b"''").unwrap(), v(&[""]));
        assert_eq!(shellwords(b"").unwrap(), Vec::<Vec<u8>>::new());
        assert!(shellwords(b"'unterminated").is_err());
        assert!(shellwords(b"\"unterminated").is_err());
        assert!(shellwords(b"trailing\\").is_err());
    }

    // ---- 何を入れても panic しない (乱数によるファズ) ----

    #[test]
    fn never_panics() {
        let alphabet = b" \t\x0b'\"\\.,abc123\xff\x00\n";
        let mut x: u64 = 0xd1b5_4a32_d192_ed03;
        for _ in 0..20000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let len = (x % 30) as usize;
            let mut s = Vec::with_capacity(len);
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            let _ = split_limit(&s, b",", 2);
            let _ = count(&s, b"a");
            let _ = indent_tab(&s, 2);
            let _ = shellwords(&s);
            let _ = group_digits(&s, b",");
            let _ = strip(&s);
            let _ = capitalize(&s);
        }
    }
}
