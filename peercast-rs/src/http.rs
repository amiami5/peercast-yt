//! HTTP の行の解析 (core/common/http.cpp の `HTTP` クラスのうち、ソケットを読まない部分) と、
//! HTTP の日付の解析 (core/common/cgi.cpp の `cgi::parseHttpDate`)。
//!
//! 入力はどれも、C++ 側の NUL 終端文字列の中身 (NUL の手前まで)。

use crate::pcstring;

/// C の `isspace` (C ロケール)。
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// C の `atoi`。先頭の空白を読み飛ばし、符号と数字を読む。
///
/// 範囲を超える値は `i32` の最小値・最大値に丸める。C 標準では未定義で、C++ 版の結果は
/// CPU と OS (`long` の幅) によって違っていた (x86-64 の Linux では `(int)LONG_MAX` の -1 になる)。
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && is_c_space(s[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i64).min(i32::MAX as i64 + 1);
        i += 1;
    }
    let v = if negative { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `sys.cpp` の `stristr`: ASCII の大文字小文字を無視して `needle` を探す。
/// C++ 版と同じく、`needle` が空なら見つからない扱い。
pub fn stristr(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

/// `HTTP::readResponse` のステータス行の解析。
///
/// 返り値は (ステータスコード, C++ 版が行に NUL を書き込む位置)。C++ 版は、ステータスコードの
/// 後ろの空白 (なければ行末) を NUL にして、行をそこで切っていた (エラー表示に使われる)。
///
/// C++ 版の走査をそのままなぞる。先頭の 1 文字は区切りとして見ない (`*++cp` で読むため)。
pub fn parse_status_line(line: &[u8]) -> (i32, usize) {
    let at = |i: usize| line.get(i).copied().unwrap_or(0);
    let mut cp = 0;
    while at(cp) != 0 {
        cp += 1;
        if at(cp) == b' ' {
            break;
        }
    }
    while at(cp) != 0 {
        cp += 1;
        if at(cp) != b' ' {
            break;
        }
    }
    let scp = cp;
    while at(cp) != 0 {
        cp += 1;
        if at(cp) == b' ' {
            break;
        }
    }
    // 入力は NUL の手前までなので、cp は line.len() を超えない
    (atoi(&line[scp.min(line.len())..cp.min(line.len())]), cp.min(line.len()))
}

/// `HTTP::nextHeader` で 1 行を解析した結果。
#[derive(Debug, PartialEq, Eq)]
pub struct HeaderLine {
    /// 値の先頭の位置 (`:` の後ろの空白を飛ばしたところ)。C++ 版の `arg`。
    pub arg_offset: usize,
    /// ヘッダー名 (大文字にしたもの)
    pub name: Vec<u8>,
    /// 値 (`\r` か `\n` の手前まで)
    pub value: Vec<u8>,
}

/// ヘッダー行を解析する。`:` がない行は `None` (C++ 版では `arg` が NULL になる)。
pub fn parse_header_line(line: &[u8]) -> Option<HeaderLine> {
    let colon = line.iter().position(|&c| c == b':')?;
    let mut ap = colon + 1;
    while ap < line.len() && line[ap] == b' ' {
        ap += 1;
    }
    let rest = &line[ap..];
    let end = rest
        .iter()
        .position(|&c| c == b'\r')
        .or_else(|| rest.iter().position(|&c| c == b'\n'))
        .unwrap_or(rest.len());
    Some(HeaderLine {
        arg_offset: ap,
        name: line[..colon].to_ascii_uppercase(),
        value: rest[..end].to_vec(),
    })
}

/// `HTTP::parseAuthorizationHeader`: `Basic` 認証のユーザー名とパスワードを取り出す。
///
/// C++ 版と同じく、"Basic" (大文字小文字を問わない) の後ろの最初の空白の次から、255 文字を
/// base64 として復号し、最初の NUL までを見て、最初の `:` で分ける。取り出せなければ `None`。
/// 長さの切り詰め (呼び出し側のバッファの大きさ) は C++ 側で行う。
pub fn parse_basic_auth(arg: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut s = stristr(arg, b"Basic")?;
    while s < arg.len() {
        s += 1;
        if arg[s - 1] == b' ' {
            break;
        }
    }
    let encoded = &arg[s..];
    let encoded = &encoded[..encoded.len().min(pcstring::MAX_LEN - 1)];
    let mut decoded = pcstring::base64_to_ascii(encoded);
    if let Some(nul) = decoded.iter().position(|&c| c == 0) {
        decoded.truncate(nul);
    }
    let colon = decoded.iter().position(|&c| c == b':')?;
    Some((decoded[..colon].to_vec(), decoded[colon + 1..].to_vec()))
}

/// `HTTP::isCrossOriginRequest`
pub fn is_cross_origin_request(sec_fetch_site: &[u8], origin: &[u8], host: &[u8]) -> bool {
    if !sec_fetch_site.is_empty() {
        let site = sec_fetch_site.to_ascii_lowercase();
        return site != b"same-origin" && site != b"none";
    }
    if origin.is_empty() {
        return false;
    }
    // "null" など、scheme://host 形式でないものは拒否。
    match origin.windows(3).position(|w| w == b"://") {
        None => true,
        Some(pos) => origin[pos + 3..].to_ascii_lowercase() != host.to_ascii_lowercase(),
    }
}

/// `HTTP::isLoopbackHostHeader`
pub fn is_loopback_host_header(host: &[u8]) -> bool {
    if host.is_empty() {
        return true;
    }
    // IPv6 リテラル: "[::1]" または "[::1]:7144"
    if host[0] == b'[' {
        return host.contains(&b']');
    }
    let end = host.iter().position(|&c| c == b':').unwrap_or(host.len());
    let name = host[..end].to_ascii_lowercase();
    if name == b"localhost" {
        return true;
    }
    // IPv4 リテラル (数字とドットのみ、ドットが3つ)
    if name.is_empty() {
        return false;
    }
    let mut dots = 0;
    for &c in &name {
        if c == b'.' {
            dots += 1;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    dots == 3
}

// ---------------------------------------------------------------- 日付

const DAYS_OF_WEEK: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
const MONTH_NAMES: [&[u8]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
];

/// C++ 版の正規表現の `[A-z]` (0x41〜0x7a。`[` `\` `]` `^` `_` `` ` `` も含む)
fn is_a_to_z(c: u8) -> bool {
    (b'A'..=b'z').contains(&c)
}

/// 先頭から、正規表現の部品を 1 つずつ読む小さな読み取り器。
struct Scanner<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn letters(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos + n;
        if end <= self.s.len() && self.s[self.pos..end].iter().all(|&c| is_a_to_z(c)) {
            let r = &self.s[self.pos..end];
            self.pos = end;
            Some(r)
        } else {
            None
        }
    }

    fn literal(&mut self, lit: &[u8]) -> Option<()> {
        if self.s[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Some(())
        } else {
            None
        }
    }

    /// `\d+`
    fn digits(&mut self) -> Option<&'a [u8]> {
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos > start {
            Some(&self.s[start..self.pos])
        } else {
            None
        }
    }

    /// ` +` (1 個以上の空白)
    fn spaces(&mut self) -> Option<()> {
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] == b' ' {
            self.pos += 1;
        }
        (self.pos > start).then_some(())
    }

    fn zone_and_end(&mut self) -> Option<()> {
        let rest = &self.s[self.pos..];
        (rest == b"GMT" || rest == b"UTC").then_some(())
    }

    fn end(&self) -> Option<()> {
        (self.pos == self.s.len()).then_some(())
    }
}

/// `std::stoi`。範囲を超えたら `None` (C++ 版は std::out_of_range を投げていた)。
fn stoi(d: &[u8]) -> Option<i64> {
    let mut v: i64 = 0;
    for &c in d {
        v = v.checked_mul(10)?.checked_add((c - b'0') as i64)?;
        if v > i32::MAX as i64 {
            return None;
        }
    }
    Some(v)
}

fn index_of(table: &[&[u8]], name: &[u8]) -> Option<i64> {
    table.iter().position(|&t| t == name).map(|i| i as i64)
}

/// 1970-01-01 からの日数 (先発グレゴリオ暦)。H. Hinnant の days_from_civil。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `timegm` (範囲外の日・時・分・秒は、C ライブラリと同じく繰り上げて計算する)。
/// `year` は 1900 年からの年数、`mon` は 0〜11。
fn timegm(year: i64, mon: i64, mday: i64, hour: i64, min: i64, sec: i64) -> i64 {
    let days = days_from_civil(year + 1900, mon + 1, 1) + (mday - 1);
    days * 86400 + hour * 3600 + min * 60 + sec
}

struct Fields {
    wday: Option<i64>,
    mon: Option<i64>,
    mday: i64,
    year: i64,
    hour: i64,
    min: i64,
    sec: i64,
}

fn rfc1123(s: &[u8]) -> Option<Option<Fields>> {
    // ^([A-z]{3}), (\d+) ([A-z]{3}) (\d+) (\d+):(\d+):(\d+) (GMT|UTC)$
    let mut p = Scanner { s, pos: 0 };
    let wd = p.letters(3)?;
    p.literal(b", ")?;
    let mday = p.digits()?;
    p.literal(b" ")?;
    let mon = p.letters(3)?;
    p.literal(b" ")?;
    let year = p.digits()?;
    p.literal(b" ")?;
    let (h, m, sec) = hms(&mut p)?;
    p.literal(b" ")?;
    p.zone_and_end()?;
    Some(fields(wd, mon, mday, year, -1900, h, m, sec))
}

fn rfc1036(s: &[u8]) -> Option<Option<Fields>> {
    // ^([A-z]+)day, (\d+)-([A-z]{3})-(\d{2}) (\d+):(\d+):(\d+) (GMT|UTC)$
    // [A-z] は ',' を含まないので、"day" は [A-z] の連続の末尾にしか来られない。
    let run = s.iter().take_while(|&&c| is_a_to_z(c)).count();
    if run < 4 || &s[run - 3..run] != b"day" {
        return None;
    }
    let wd = &s[..run - 3];
    let mut p = Scanner { s, pos: run };
    p.literal(b", ")?;
    let mday = p.digits()?;
    p.literal(b"-")?;
    let mon = p.letters(3)?;
    p.literal(b"-")?;
    let year = p.digits()?;
    if year.len() != 2 {
        return None;
    }
    p.literal(b" ")?;
    let (h, m, sec) = hms(&mut p)?;
    p.literal(b" ")?;
    p.zone_and_end()?;
    Some(fields(wd, mon, mday, year, 0, h, m, sec))
}

fn asctime(s: &[u8]) -> Option<Option<Fields>> {
    // ^([A-z]{3}) ([A-z]{3}) +(\d+) (\d+):(\d+):(\d+) (\d+)$
    let mut p = Scanner { s, pos: 0 };
    let wd = p.letters(3)?;
    p.literal(b" ")?;
    let mon = p.letters(3)?;
    p.spaces()?;
    let mday = p.digits()?;
    p.literal(b" ")?;
    let (h, m, sec) = hms(&mut p)?;
    p.literal(b" ")?;
    let year = p.digits()?;
    p.end()?;
    Some(fields(wd, mon, mday, year, -1900, h, m, sec))
}

fn hms<'a>(p: &mut Scanner<'a>) -> Option<(&'a [u8], &'a [u8], &'a [u8])> {
    let h = p.digits()?;
    p.literal(b":")?;
    let m = p.digits()?;
    p.literal(b":")?;
    let s = p.digits()?;
    Some((h, m, s))
}

/// 正規表現には一致したが、数値が大きすぎたら内側が `None`。
#[allow(clippy::too_many_arguments)]
fn fields(wd: &[u8], mon: &[u8], mday: &[u8], year: &[u8], year_offset: i64, h: &[u8], m: &[u8], s: &[u8]) -> Option<Fields> {
    Some(Fields {
        wday: index_of(&DAYS_OF_WEEK, wd),
        mon: index_of(&MONTH_NAMES, mon),
        mday: stoi(mday)?,
        year: stoi(year)? + year_offset,
        hour: stoi(h)?,
        min: stoi(m)?,
        sec: stoi(s)?,
    })
}

/// `cgi::parseHttpDate`: RFC 1123, RFC 1036, asctime の 3 形式の日付を UNIX 時刻にする。
/// 解釈できなければ -1。
///
/// C++ 版と同じく、RFC 1036 形式は曜日の名前から "day" を除いた部分を 3 文字の略号と
/// 比べるので、Sunday, Monday, Friday 以外は解釈できない (-1 になる)。
/// C++ 版は数字が `int` に収まらないと std::out_of_range を投げていたが、Rust 版は -1 を返す。
pub fn parse_http_date(s: &[u8]) -> i64 {
    let parsed = rfc1123(s).or_else(|| rfc1036(s)).or_else(|| asctime(s));
    match parsed {
        Some(Some(Fields { wday: Some(_), mon: Some(mon), mday, year, hour, min, sec })) => {
            timegm(year, mon, mday, hour, min, sec)
        }
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi(b"200 OK"), 200);
        assert_eq!(atoi(b"  -12x"), -12);
        assert_eq!(atoi(b"+7"), 7);
        assert_eq!(atoi(b"x"), 0);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"99999999999999999999"), i32::MAX);
        assert_eq!(atoi(b"-99999999999999999999"), i32::MIN);
    }

    #[test]
    fn status_line() {
        assert_eq!(parse_status_line(b"HTTP/1.0 200 OK"), (200, 12));
        assert_eq!(parse_status_line(b"HTTP/1.1   404"), (404, 14));
        assert_eq!(parse_status_line(b""), (0, 0));
        assert_eq!(parse_status_line(b"X"), (0, 1));
        // 先頭の空白は区切りとして見ない
        assert_eq!(parse_status_line(b" 200 OK"), (0, 7));
    }

    #[test]
    fn header_line() {
        let h = parse_header_line(b"Content-Type:  text/html\r\n").unwrap();
        assert_eq!(h.name, b"CONTENT-TYPE");
        assert_eq!(h.value, b"text/html");
        assert_eq!(h.arg_offset, 15);
        assert_eq!(parse_header_line(b"no colon"), None);
        let h = parse_header_line(b"X:").unwrap();
        assert_eq!((h.arg_offset, h.value.as_slice()), (2, b"".as_slice()));
    }

    #[test]
    fn basic_auth() {
        // "user:pass"
        assert_eq!(
            parse_basic_auth(b"Basic dXNlcjpwYXNz"),
            Some((b"user".to_vec(), b"pass".to_vec()))
        );
        assert_eq!(parse_basic_auth(b"bAsIc dXNlcjpwYXNz").map(|x| x.0), Some(b"user".to_vec()));
        assert_eq!(parse_basic_auth(b"Bearer abc"), None);
        assert_eq!(parse_basic_auth(b"Basic dXNlcg=="), None); // "user" (':' なし)
    }

    #[test]
    fn cross_origin_and_loopback() {
        assert!(!is_cross_origin_request(b"same-origin", b"", b""));
        assert!(is_cross_origin_request(b"cross-site", b"", b""));
        assert!(!is_cross_origin_request(b"", b"", b"h"));
        assert!(is_cross_origin_request(b"", b"null", b"h"));
        assert!(!is_cross_origin_request(b"", b"http://LOCALHOST:7144", b"localhost:7144"));
        assert!(is_loopback_host_header(b""));
        assert!(is_loopback_host_header(b"[::1]:7144"));
        assert!(is_loopback_host_header(b"LocalHost:7144"));
        assert!(is_loopback_host_header(b"127.0.0.1:7144"));
        assert!(!is_loopback_host_header(b"example.com"));
        assert!(!is_loopback_host_header(b":7144"));
    }

    #[test]
    fn http_date() {
        assert_eq!(parse_http_date(b"Sun, 06 Nov 1994 08:49:37 GMT"), 784111777);
        assert_eq!(parse_http_date(b"Sunday, 06-Nov-94 08:49:37 GMT"), 784111777);
        assert_eq!(parse_http_date(b"Sun Nov  6 08:49:37 1994"), 784111777);
        assert_eq!(parse_http_date(b"Tuesday, 08-Nov-94 08:49:37 GMT"), -1); // C++ 版と同じ
        assert_eq!(parse_http_date(b"Sun, 06 Nov 1994 08:49:37 JST"), -1);
        assert_eq!(parse_http_date(b"Sun, 99999999999 Nov 1994 08:49:37 GMT"), -1);
        assert_eq!(parse_http_date(b""), -1);
        // 範囲外の日は繰り上がる (timegm と同じ)
        assert_eq!(parse_http_date(b"Sun, 31 Feb 2021 00:00:00 GMT"), parse_http_date(b"Sun, 03 Mar 2021 00:00:00 GMT"));
    }
}
