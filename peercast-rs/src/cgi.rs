//! URL エンコード、HTML と JavaScript のエスケープ、パスの検査。
//! C++ 版 core/common/cgi.cpp の同名の関数に相当する。
//!
//! C++ 版との違いは、各関数の説明に書いてある。

use crate::entities::ENTITIES;
use crate::utf8;

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn push_percent(out: &mut Vec<u8>, b: u8) {
    out.push(b'%');
    out.push(HEX_UPPER[(b >> 4) as usize]);
    out.push(HEX_UPPER[(b & 0xf) as usize]);
}

/// URL エンコード。英数字と `_-.` はそのまま、空白は `+`、それ以外は `%XX` (大文字)。
pub fn escape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &c in input {
        match c {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'-' | b'.' => out.push(c),
            b' ' => out.push(b'+'),
            _ => push_percent(&mut out, c),
        }
    }
    out
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// URL デコード。`%XX` は 1 バイトに、`+` は空白にする。
///
/// C++ 版との違い: `%` のあとに 16 進数 2 桁が続かない場合、`%` をそのまま出力する。
/// C++ 版は `sscanf` の結果を確かめておらず、`%zz` のような入力では未初期化の変数の値が
/// 出力に混ざっていた (`%4` のように 1 桁だけの場合や、`% 4` `%+4` のように `sscanf` が
/// 受け付ける変わった書き方では、そのまま 1 バイトになっていた)。
pub fn unescape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'%' => {
                let hi = input.get(i + 1).copied().and_then(hex_value);
                let lo = input.get(i + 2).copied().and_then(hex_value);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push(hi << 4 | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// HTML のエスケープ。`& < > " '` を実体参照にする。
pub fn escape_html(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &c in input {
        match c {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\'' => out.extend_from_slice(b"&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// JavaScript の文字列リテラルの中に入れるためのエスケープ。
///
/// `< > &` は、`<script>` の中で `</script>` により要素を抜けられないよう `\xNN` にする。
pub fn escape_javascript(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &c in input {
        match c {
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(c);
            }
            b'<' | b'>' | b'&' => push_hex(&mut out, c),
            0x00..=0x1f | 0x7f => push_hex(&mut out, c), // 制御文字
            _ => out.push(c),
        }
    }
    out
}

fn push_hex(out: &mut Vec<u8>, b: u8) {
    out.extend_from_slice(b"\\x");
    out.push(HEX_UPPER[(b >> 4) as usize]);
    out.push(HEX_UPPER[(b & 0xf) as usize]);
}

fn is_decimal(r: &[u8]) -> bool {
    r.len() >= 2 && r[0] == b'#' && r[1..].iter().all(u8::is_ascii_digit)
}

fn is_hexadecimal(r: &[u8]) -> bool {
    r.len() >= 3 && r[0] == b'#' && (r[1] == b'x' || r[1] == b'X') && r[2..].iter().all(u8::is_ascii_hexdigit)
}

/// 数字の列を数値にする。u32 に収まらなければ `u32::MAX` (どのみち無効なコードポイント)。
fn parse_saturating(digits: &[u8], radix: u32) -> u32 {
    let mut v: u32 = 0;
    for &d in digits {
        let d = (d as char).to_digit(radix).unwrap_or(0);
        v = match v.checked_mul(radix).and_then(|x| x.checked_add(d)) {
            Some(x) => x,
            None => return u32::MAX,
        };
    }
    v
}

fn lookup_entity(name: &[u8]) -> Option<u32> {
    ENTITIES
        .binary_search_by(|(n, _)| n.as_bytes().cmp(name))
        .ok()
        .map(|i| ENTITIES[i].1)
}

/// HTML の文字参照 (`&lt;` `&#12354;` `&#x1f4a9;` など) を元の文字に戻す。
/// 知らない名前や、`;` で終わっていないものは、そのまま残す。
///
/// C++ 版との違い:
///  - Latin-1 の範囲 (`&copy;` `&nbsp;` `&#169;` など U+0080〜U+07FF) を正しい UTF-8 にする
///    (C++ 版は `codepoint_to_utf8` の誤りで、不正なバイト列になっていた)。
///  - 無効なコードポイント (U+10FFFF より大きいもの、サロゲート、桁あふれする数) は
///    U+FFFD にする。C++ 版は例外を投げて (処理しきれず落ちる)、あるいは不正な UTF-8 を出力していた。
pub fn unescape_html(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] != b'&' {
            out.push(input[i]);
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < input.len() && (input[i].is_ascii_alphanumeric() || input[i] == b'#') {
            i += 1;
        }
        let name = &input[start..i];

        if i == input.len() {
            // ';' が来ないまま終わった
            out.push(b'&');
            out.extend_from_slice(name);
            return out;
        }
        if input[i] != b';' {
            out.push(b'&');
            out.extend_from_slice(name);
            continue; // 区切りの文字は、次の周回で普通に処理する
        }
        i += 1; // ';'

        let codepoint = if name.is_empty() {
            None
        } else if is_decimal(name) {
            Some(parse_saturating(&name[1..], 10))
        } else if is_hexadecimal(name) {
            Some(parse_saturating(&name[2..], 16))
        } else {
            lookup_entity(name)
        };

        match codepoint {
            Some(cp) => match utf8::codepoint_to_utf8(cp) {
                Some(bytes) => out.extend_from_slice(&bytes),
                None => out.extend_from_slice("\u{fffd}".as_bytes()),
            },
            None => {
                out.push(b'&');
                out.extend_from_slice(name);
                out.push(b';');
            }
        }
    }
    out
}

/// リダイレクト先などに使える、安全なローカルパスか。
/// `/` で始まり、`//` や `/\` で始まらず (別のサイトを指せない)、制御文字を含まないもの。
pub fn is_safe_local_path(path: &[u8]) -> bool {
    if path.first() != Some(&b'/') {
        return false;
    }
    if matches!(path.get(1), Some(b'/') | Some(b'\\')) {
        return false;
    }
    !path.iter().any(|&c| c < 0x20 || c == 0x7f)
}

/// 他人から届いた URL (コンタクト URL など) を画面のリンクにするとき用。`http://` か `https://` で
/// 始まるものはそのまま、それ以外 (`javascript:` や `data:` など) は空にする。ブラウザーは前後の空白と
/// 制御文字、途中のタブと改行を無視するので、それらを除いてから見る。
pub fn link_url(url: &[u8]) -> &[u8] {
    let trimmed = match url.iter().position(|&c| c > 0x20) {
        Some(i) => &url[i..],
        None => return b"",
    };
    let head: Vec<u8> = trimmed.iter().filter(|&&c| !matches!(c, b'\t' | b'\n' | b'\r')).take(8).map(|c| c.to_ascii_lowercase()).collect();
    if head.starts_with(b"http://") || head.starts_with(b"https://") {
        url
    } else {
        b""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_url_allows_only_http() {
        for ok in [&b"http://a.example/"[..], b"https://a.example/x?y=1", b"HTTPS://A.EXAMPLE/", b" http://a/", b"ht\ttp://a/"] {
            assert_eq!(link_url(ok), ok, "{:?}", String::from_utf8_lossy(ok));
        }
        for ng in [
            &b"javascript:alert(1)"[..],
            b"JavaScript:alert(1)",
            b" \x01javascript:alert(1)",
            b"java\tscript:alert(1)",
            b"java\nscript:alert(1)",
            b"data:text/html,<script>alert(1)</script>",
            b"vbscript:x",
            b"www.example.com",
            b"//evil.example/",
            b"http:/a",
            b"",
            b"   ",
        ] {
            assert_eq!(link_url(ng), b"", "{:?}", String::from_utf8_lossy(ng));
        }
    }

    fn all_bytes() -> Vec<u8> {
        (0..=255u8).collect()
    }

    // ---- C++ 版の単体テスト (tests/cgi_unittest.cpp) を移したもの ----

    #[test]
    fn escape_unescape_roundtrip_all_bytes() {
        let all = all_bytes();
        assert_eq!(unescape(&escape(&all)), all);
    }

    #[test]
    fn unescape_accepts_upper_and_lower_case_hex() {
        let all = all_bytes();
        let upper: String = all.iter().map(|b| format!("%{:02X}", b)).collect();
        let lower: String = all.iter().map(|b| format!("%{:02x}", b)).collect();
        assert_eq!(unescape(upper.as_bytes()), all);
        assert_eq!(unescape(lower.as_bytes()), all);
    }

    #[test]
    fn escape_examples() {
        assert_eq!(escape(b"abc XYZ-_.019"), b"abc+XYZ-_.019");
        assert_eq!(escape("あ".as_bytes()), b"%E3%81%82");
        assert_eq!(escape(b"a/b?c=d&e"), b"a%2Fb%3Fc%3Dd%26e");
    }

    #[test]
    fn escape_html_cases() {
        assert_eq!(escape_html(b""), b"");
        assert_eq!(escape_html(b"aa"), b"aa");
        assert_eq!(escape_html(b"<&>'\""), b"&lt;&amp;&gt;&#39;&quot;");
    }

    #[test]
    fn unescape_html_cases_from_cpp_tests() {
        assert_eq!(unescape_html(b""), b"");
        assert_eq!(unescape_html(b"aa"), b"aa");
        assert_eq!(unescape_html(b"&lt;&amp;&gt;"), b"<&>");
        assert_eq!(unescape_html(b"&lt;&&gt;"), b"<&>");
        assert_eq!(unescape_html(b"&lt;a&b&gt;"), b"<a&b>");
        assert_eq!(unescape_html(b"&gt"), b"&gt");
        assert_eq!(unescape_html(b"&YY;"), b"&YY;");
        assert_eq!(unescape_html(b"&#12354;"), "あ".as_bytes());
        assert_eq!(unescape_html(b"&#x1f4a9;"), "💩".as_bytes());
        assert_eq!(unescape_html(b"&#X1F4A9;"), "💩".as_bytes());
    }

    #[test]
    fn escape_javascript_cases_from_cpp_tests() {
        assert_eq!(escape_javascript(b""), b"");
        assert_eq!(escape_javascript(b"aa"), b"aa");
        assert_eq!(escape_javascript(b"'\"\\"), b"\\'\\\"\\\\");
        assert_eq!(escape_javascript("あ".as_bytes()), "あ".as_bytes());
        assert_eq!(escape_javascript(b"\r\n"), b"\\x0D\\x0A");
        assert_eq!(escape_javascript(b"</script>"), b"\\x3C/script\\x3E");
        assert_eq!(escape_javascript(b"a&b"), b"a\\x26b");
    }

    #[test]
    fn safe_local_path_cases() {
        assert!(is_safe_local_path(b"/"));
        assert!(is_safe_local_path(b"/html/en/index.html"));
        assert!(is_safe_local_path(b"/html/en/play.html?id=0123456789ABCDEF&x=%20y"));
        assert!(!is_safe_local_path(b""));
        assert!(!is_safe_local_path(b"html/en/index.html"));
        assert!(!is_safe_local_path(b"//evil.example/"));
        assert!(!is_safe_local_path(b"/\\evil.example/"));
        assert!(!is_safe_local_path(b"http://evil.example/"));
        assert!(!is_safe_local_path(b"/a\r\nSet-Cookie: x=y"));
        assert!(!is_safe_local_path(b"/a\x7f"));
        assert!(!is_safe_local_path(b"/a\0b"));
    }

    // ---- C++ 版から意図的に変えたところ ----

    #[test]
    fn unescape_leaves_malformed_percent_sequences_alone() {
        assert_eq!(unescape(b"%"), b"%");
        assert_eq!(unescape(b"%z"), b"%z");
        assert_eq!(unescape(b"%zz"), b"%zz");
        assert_eq!(unescape(b"%4"), b"%4");
        assert_eq!(unescape(b"%4z"), b"%4z");
        assert_eq!(unescape(b"% 4"), b"% 4");
        assert_eq!(unescape(b"%+4"), b"% 4");
        assert_eq!(unescape(b"100%"), b"100%");
        assert_eq!(unescape(b"%%41"), b"%A");
    }

    #[test]
    fn unescape_html_encodes_latin1_range_correctly() {
        assert_eq!(unescape_html(b"&copy;"), "©".as_bytes());
        assert_eq!(unescape_html(b"&nbsp;"), "\u{a0}".as_bytes());
        assert_eq!(unescape_html(b"&#169;"), "©".as_bytes());
        assert_eq!(unescape_html(b"&#x410;"), "А".as_bytes());
        assert!(utf8::validate(&unescape_html(b"&yen;&sect;&uml;&laquo;")));
    }

    #[test]
    fn unescape_html_replaces_invalid_codepoints() {
        let fffd = "\u{fffd}".as_bytes();
        assert_eq!(unescape_html(b"&#xD800;"), fffd);
        assert_eq!(unescape_html(b"&#xDFFF;"), fffd);
        assert_eq!(unescape_html(b"&#x110000;"), fffd);
        assert_eq!(unescape_html(b"&#99999999999999999999999;"), fffd);
        assert_eq!(unescape_html(b"&#xFFFFFFFFFFFFFFFF;"), fffd);
        assert_eq!(unescape_html(b"&#4294967361;"), fffd); // 2^32 + 65。C++ 版は桁が回って 'A' になっていた
        assert_eq!(unescape_html(b"&#x10FFFF;"), "\u{10ffff}".as_bytes());
        assert_eq!(unescape_html(b"&#0;"), b"\0"); // NUL はそのまま (C++ 版と同じ)
    }

    #[test]
    fn unescape_html_odd_inputs() {
        assert_eq!(unescape_html(b"&"), b"&");
        assert_eq!(unescape_html(b"&;"), b"&;");
        assert_eq!(unescape_html(b"&#;"), b"&#;");
        assert_eq!(unescape_html(b"&#x;"), b"&#x;");
        assert_eq!(unescape_html(b"&#12a;"), b"&#12a;");
        assert_eq!(unescape_html(b"&&&"), b"&&&");
        assert_eq!(unescape_html(b"&amp"), b"&amp");
        assert_eq!(unescape_html(b"&amp&amp;"), b"&amp&");
        assert_eq!(unescape_html(b"&AMP;"), b"&AMP;"); // 大文字小文字は区別する
    }

    #[test]
    fn entity_table_is_sorted_and_all_entries_are_valid() {
        for w in ENTITIES.windows(2) {
            assert!(w[0].0.as_bytes() < w[1].0.as_bytes(), "{} {}", w[0].0, w[1].0);
        }
        for (name, cp) in ENTITIES.iter() {
            assert!(char::from_u32(*cp).is_some(), "{}", name);
            assert_eq!(lookup_entity(name.as_bytes()), Some(*cp));
        }
    }

    // ---- 何を入れても panic しない、出力の性質を守る ----

    fn pseudo_random_inputs() -> impl Iterator<Item = Vec<u8>> {
        let alphabet = b"&#;xX%+0123456789aAfFgG<>\"'\\/ \r\n\x00\x7f\x80\xc0\xff".to_vec();
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        (0..20000).map(move |_| {
            let mut v = Vec::new();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            for _ in 0..(x % 24) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.push(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            v
        })
    }

    #[test]
    fn never_panics_and_keeps_invariants() {
        for input in pseudo_random_inputs() {
            assert_eq!(unescape(&escape(&input)), input);
            let e = escape_html(&input);
            assert!(!e.iter().any(|&c| matches!(c, b'<' | b'>' | b'"' | b'\'')));
            let j = escape_javascript(&input);
            assert!(!j.iter().any(|&c| matches!(c, b'<' | b'>' | b'&' | b'\n' | b'\r' | 0..=0x1f | 0x7f)));
            let _ = unescape(&input);
            let u = unescape_html(&input);
            // 文字参照から出てくる部分は必ず正しい UTF-8 (入力側が不正でない限り)
            if utf8::validate(&input) {
                assert!(utf8::validate(&u), "{:?} -> {:?}", input, u);
            }
            let _ = is_safe_local_path(&input);
        }
    }
}
