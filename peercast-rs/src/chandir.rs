//! イエローページのチャンネル一覧 (core/common/chandir.cpp の `ChannelEntry` と、
//! `ChannelDirectory` のうち状態を持たない部分)。
//!
//! index.txt は 1 行が 1 チャンネルで、19 個の欄を `<>` で区切る。一覧を取りに行く処理 (スレッドと
//! HTTP) と、読んだ一覧の入れ物は、今は C++ 側にある。

use crate::{gnuid, http, strutil, url};

/// index.txt の 1 行の欄の数
pub const NUM_FIELDS: usize = 19;

/// `ChannelEntry` (`feedUrl` は呼び出し側で持つ)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: Vec<u8>,
    pub id: [u8; 16],
    pub tip: Vec<u8>,
    /// http(s) の URL でなければ空
    pub url: Vec<u8>,
    pub genre: Vec<u8>,
    pub desc: Vec<u8>,
    pub num_directs: i32,
    pub num_relays: i32,
    pub bitrate: i32,
    pub content_type: Vec<u8>,
    pub track_artist: Vec<u8>,
    pub track_album: Vec<u8>,
    pub track_name: Vec<u8>,
    /// http(s) の URL でなければ空
    pub track_contact: Vec<u8>,
    pub encoded_name: Vec<u8>,
    pub uptime: Vec<u8>,
    pub status: Vec<u8>,
    pub comment: Vec<u8>,
    pub direct: i32,
}

/// C++ 版は `c_str()` を渡すので、NUL の手前までを使う
fn c_str(s: &[u8]) -> &[u8] {
    s.iter().position(|&b| b == 0).map_or(s, |i| &s[..i])
}

fn http_url_or_empty(s: &[u8]) -> Vec<u8> {
    if url::is_http_url(s) {
        s.to_vec()
    } else {
        Vec::new()
    }
}

impl Entry {
    /// `ChannelEntry(fields, feedUrl)`。欄が 19 個に足りなければ `None` (C++ 版は例外を投げる)。
    /// 20 個目以降は使わない。
    pub fn from_fields<F: AsRef<[u8]>>(f: &[F]) -> Option<Entry> {
        if f.len() < NUM_FIELDS {
            return None;
        }
        let s = |i: usize| f[i].as_ref().to_vec();
        let int = |i: usize| http::atoi(c_str(f[i].as_ref()));
        Some(Entry {
            name: s(0),
            id: gnuid::from_str(c_str(f[1].as_ref())),
            tip: s(2),
            url: http_url_or_empty(f[3].as_ref()),
            genre: s(4),
            desc: s(5),
            num_directs: int(6),
            num_relays: int(7),
            bitrate: int(8),
            content_type: s(9),
            track_artist: s(10),
            track_album: s(11),
            track_name: s(12),
            track_contact: http_url_or_empty(f[13].as_ref()),
            encoded_name: s(14),
            uptime: s(15),
            status: s(16),
            comment: s(17),
            direct: int(18),
        })
    }
}

/// `std::getline` と同じ行の分け方 (`\n` で切る。最後の `\n` のあとの空の行は数えない)
fn lines(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    let body = text.strip_suffix(b"\n").unwrap_or(text);
    let empty = text.is_empty();
    body.split(|&b| b == b'\n').filter(move |_| !empty)
}

/// 1 行の結果
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    Entry(Entry),
    /// 欄の数が 19 でない行 (1 から数えた行番号)
    Error(i32),
}

/// `ChannelEntry::textToChannelEntries`。行ごとに `f` を呼ぶ。
pub fn parse_index(text: &[u8], mut f: impl FnMut(Line)) {
    let mut lineno: i32 = 0;
    for line in lines(text) {
        lineno = lineno.wrapping_add(1);
        let fields = strutil::split(line, b"<>");
        if fields.len() != NUM_FIELDS {
            f(Line::Error(lineno));
        } else {
            f(Line::Entry(Entry::from_fields(&fields).expect("19 fields")));
        }
    }
}

/// エラーの行の文言 (`Parse error at line %d.`)
pub fn parse_error_message(lineno: i32) -> Vec<u8> {
    format!("Parse error at line {}.", lineno).into_bytes()
}

/// `chatUrl` / `statsUrl`: 一覧の URL の最後の `/` までに `page` と名前を付ける。
/// 名前が空か、URL に `/` がなければ空。
fn side_url(feed_url: &[u8], encoded_name: &[u8], page: &[u8]) -> Vec<u8> {
    if encoded_name.is_empty() {
        return Vec::new();
    }
    match feed_url.iter().rposition(|&b| b == b'/') {
        None => Vec::new(),
        Some(i) => [&feed_url[..i], page, encoded_name].concat(),
    }
}

pub fn chat_url(feed_url: &[u8], encoded_name: &[u8]) -> Vec<u8> {
    side_url(feed_url, encoded_name, b"/chat.php?cn=")
}

pub fn stats_url(feed_url: &[u8], encoded_name: &[u8]) -> Vec<u8> {
    side_url(feed_url, encoded_name, b"/getgmt.php?cn=")
}

/// `directoryUrlOf`: 最後の `/` より後ろを取り除く (C++ 版は正規表現 `/[^/]*$` で探して
/// `/` に置き換える)。`/` がなければそのまま。
pub fn directory_url(url: &[u8]) -> Vec<u8> {
    match url.iter().rposition(|&b| b == b'/') {
        None => url.to_vec(),
        Some(i) => url[..=i].to_vec(),
    }
}

/// `formatTime`: 経過秒を `"%ds"` か `"%dm %ds"` にする
pub fn format_time(diff: u32) -> Vec<u8> {
    let (min, sec) = (diff / 60, diff % 60);
    if min == 0 {
        format!("{}s", sec).into_bytes()
    } else {
        format!("{}m {}s", min, sec).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "予定地<>97968780D09CC97BB98D4A2BF221EDE7<>127.0.0.1:7144<>http://www.example.com/<>プログラミング<>peercastをいじる - &lt;Free&gt;<>-1<>-1<>428<>FLV<><><><><>%E4%BA%88%E5%AE%9A%E5%9C%B0<>1:14<>click<><>1";

    fn parse(text: &[u8]) -> Vec<Line> {
        let mut v = Vec::new();
        parse_index(text, |l| v.push(l));
        v
    }

    #[test]
    fn parses_a_line() {
        let v = parse(format!("{}\n", LINE).as_bytes());
        assert_eq!(v.len(), 1);
        let Line::Entry(e) = &v[0] else { panic!() };
        assert_eq!(e.name, "予定地".as_bytes());
        assert_eq!(&gnuid::to_str(&e.id), b"97968780D09CC97BB98D4A2BF221EDE7");
        assert_eq!((e.num_directs, e.num_relays, e.bitrate, e.direct), (-1, -1, 428, 1));
        assert_eq!(e.url, b"http://www.example.com/");
        assert_eq!(e.encoded_name, b"%E4%BA%88%E5%AE%9A%E5%9C%B0");
    }

    #[test]
    fn drops_non_http_urls_and_counts_lines() {
        let bad = "ch<>97968780D09CC97BB98D4A2BF221EDE7<>t<>javascript:alert(1)<>g<>d<>-1<>-1<>428<>FLV<><><><>data:text/html,x<>ch<>1:14<>click<><>1";
        let v = parse(format!("x\n{}\n\n", bad).as_bytes());
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], Line::Error(1));
        let Line::Entry(e) = &v[1] else { panic!() };
        assert!(e.url.is_empty() && e.track_contact.is_empty());
        assert_eq!(v[2], Line::Error(3));
        assert!(parse(b"").is_empty());
        assert_eq!(parse(b"\n"), vec![Line::Error(1)]);
        // 最後の行に改行がなくても読む
        assert_eq!(parse(LINE.as_bytes()).len(), 1);
    }

    #[test]
    fn too_few_fields() {
        assert!(Entry::from_fields::<&[u8]>(&[]).is_none());
        let e = Entry::from_fields(&["a"; 20]).unwrap();
        assert_eq!(e.id, [0; 16]);
    }

    #[test]
    fn urls_and_time() {
        assert_eq!(chat_url(b"http://yp/index.txt", b"%20"), b"http://yp/chat.php?cn=%20");
        assert_eq!(stats_url(b"http://yp/index.txt", b"x"), b"http://yp/getgmt.php?cn=x");
        assert_eq!(chat_url(b"http://yp/index.txt", b""), b"");
        assert_eq!(chat_url(b"index.txt", b"x"), b"");
        assert_eq!(directory_url(b"http://yp/a/index.txt"), b"http://yp/a/");
        assert_eq!(directory_url(b"noslash"), b"noslash");
        assert_eq!(format_time(59), b"59s");
        assert_eq!(format_time(61), b"1m 1s");
        assert_eq!(format_time(u32::MAX), b"71582788m 15s");
    }
}
