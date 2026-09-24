//! 公開ディレクトリ (core/common/public.cpp の `PublicController`) の、入力を解釈する部分。

use crate::strtod::atof;
use crate::strutil::{replace_prefix, split};

/// `PublicController::formatUptime`: "時:分" (時は 2 桁以上)
pub fn format_uptime(total_seconds: u32) -> String {
    let total_minutes = total_seconds / 60;
    format!("{:02}:{:02}", total_minutes / 60, total_minutes % 60)
}

/// `PublicController::acceptableLanguages`: Accept-Language ヘッダーの言語タグを、q 値の大きい順に
/// 並べる。q 値が同じなら書かれた順 (安定)。
///
/// C++ 版は `std::sort` を使っていた。タグが 16 個以下のときは挿入ソートになり、ここと同じ
/// 結果になる。17 個以上のときは、q 値が同じタグの順序が実装次第で、さらに q 値が NaN
/// (`q=nan`) だと比較が一貫せず、配列の外を読むことがあった (未定義動作)。Rust 版は、個数に
/// よらず同じ挿入ソートを使う。
pub fn acceptable_languages(accept_language: &[u8]) -> Vec<Vec<u8>> {
    if accept_language.is_empty() {
        return Vec::new();
    }
    let mut tags: Vec<(Vec<u8>, f64)> = Vec::new();
    for spec in split(accept_language, b",") {
        let mut ws = split(&spec, b";");
        // split は常に 1 つ以上の要素を返す (C++ 版の "parse error" は起きない)
        let q = if ws.len() == 1 { 1.0 } else { atof(&replace_prefix(&ws[1], b"q=", b"")) };
        tags.push((ws.swap_remove(0), q));
    }
    insertion_sort_desc(&mut tags);
    tags.into_iter().map(|(t, _)| t).collect()
}

/// libstdc++ の `std::__insertion_sort` を、比較 `a.q > b.q` で行う
fn insertion_sort_desc(v: &mut [(Vec<u8>, f64)]) {
    let comp = |a: f64, b: f64| a > b;
    for i in 1..v.len() {
        if comp(v[i].1, v[0].1) {
            // 先頭より前に来るものは、先頭へ
            v[..=i].rotate_right(1);
        } else {
            // 先頭との比較が偽なので、先頭で必ず止まる
            let mut j = i;
            while comp(v[j].1, v[j - 1].1) {
                v.swap(j, j - 1);
                j -= 1;
            }
        }
    }
}

/// `PublicController::operator()` の振り分け (段階 8c)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// `/public` を `/public/` へ
    RedirectSlash = 0,
    /// `/public/` を `/public/index.html` へ
    RedirectIndex = 1,
    /// チャンネルの一覧 (index.txt)
    IndexTxt = 2,
    /// 視聴ページ
    Play = 3,
    /// ほかのファイル
    File = 4,
}

pub fn route(path: &[u8]) -> Route {
    match path {
        b"/public" => Route::RedirectSlash,
        b"/public/" => Route::RedirectIndex,
        b"/public/index.txt" => Route::IndexTxt,
        b"/public/play.html" => Route::Play,
        _ => Route::File,
    }
}

const OCTET_STREAM: &[u8] = b"application/octet-stream";

/// public.cpp の `MIMEType`: パスのどこかに拡張子が含まれるか (大文字小文字を区別する)
pub fn mime_type(path: &[u8]) -> &'static [u8] {
    const TABLE: [(&[u8], &[u8]); 6] = [
        (b".htm", b"text/html"),
        (b".css", b"text/css"),
        (b".jpg", b"image/jpeg"),
        (b".gif", b"image/gif"),
        (b".png", b"image/png"),
        (b".js", b"application/javascript; charset=utf-8"),
    ];
    TABLE.iter().find(|(ext, _)| crate::strutil::contains(path, ext)).map_or(OCTET_STREAM, |&(_, m)| m)
}

/// assets.cpp の `MIMEType`: パスの終わりの拡張子
pub fn assets_mime_type(path: &[u8]) -> &'static [u8] {
    const TABLE: [(&[u8], &[u8]); 9] = [
        (b".htm", b"text/html"),
        (b".html", b"text/html"),
        (b".css", b"text/css"),
        (b".jpg", b"image/jpeg"),
        (b".gif", b"image/gif"),
        (b".png", b"image/png"),
        (b".js", b"application/javascript; charset=utf-8"),
        (b".svg", b"image/svg+xml"),
        (b".ico", b"image/vnd.microsoft.icon"),
    ];
    TABLE.iter().find(|(ext, _)| path.ends_with(ext)).map_or(OCTET_STREAM, |&(_, m)| m)
}

/// `AssetsController::operator()` で 304 を返すか。`last_modified` はファイルの更新時刻 (わからなければ -1)、
/// `if_modified_since` は If-Modified-Since ヘッダー (なければ空)。
pub fn not_modified(last_modified: i64, if_modified_since: &[u8]) -> bool {
    let since = if if_modified_since.is_empty() { -1 } else { crate::http::parse_http_date(if_modified_since) };
    last_modified != -1 && since != -1 && last_modified <= since
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs(s: &str) -> Vec<String> {
        acceptable_languages(s.as_bytes()).into_iter().map(|t| String::from_utf8(t).unwrap()).collect()
    }

    #[test]
    fn uptime() {
        assert_eq!(format_uptime(0), "00:00");
        assert_eq!(format_uptime(59), "00:00");
        assert_eq!(format_uptime(60), "00:01");
        assert_eq!(format_uptime(3599), "00:59");
        assert_eq!(format_uptime(3600), "01:00");
        assert_eq!(format_uptime(100 * 3600 - 1), "99:59");
        assert_eq!(format_uptime(100 * 3600), "100:00");
        assert_eq!(format_uptime(u32::MAX), "1193046:28");
    }

    #[test]
    fn languages() {
        assert!(langs("").is_empty());
        assert_eq!(langs("fr-CA"), ["fr-CA"]);
        assert_eq!(langs("ja,en;q=0.5"), ["ja", "en"]);
        assert_eq!(langs("ja;q=0.5,en"), ["en", "ja"]);
        // 空白のあとの q= は読めずに 0 になる (C++ 版と同じ)
        assert_eq!(langs("ja; q=0.9,en;q=0.1"), ["en", "ja"]);
        assert_eq!(langs("a;q=0.5,b;q=0.5,c;q=0.5"), ["a", "b", "c"]);
        assert_eq!(langs(",x"), ["", "x"]);
        // NaN は、その前の要素と入れ替わらない
        assert_eq!(langs("a;q=0.1,b;q=nan,c;q=1"), ["c", "a", "b"]);
        let many: Vec<String> = (0..40).map(|i| format!("l{};q=nan", i)).collect();
        assert_eq!(langs(&many.join(",")).len(), 40);
    }

    #[test]
    fn routes_and_mime() {
        assert_eq!(route(b"/public"), Route::RedirectSlash);
        assert_eq!(route(b"/public/"), Route::RedirectIndex);
        assert_eq!(route(b"/public/index.txt"), Route::IndexTxt);
        assert_eq!(route(b"/public/play.html"), Route::Play);
        assert_eq!(route(b"/public/x"), Route::File);
        assert_eq!(mime_type(b"/r/a.html.ja"), b"text/html");
        assert_eq!(mime_type(b"/r/a.HTM"), b"application/octet-stream");
        assert_eq!(mime_type(b"/r/a.json"), b"application/javascript; charset=utf-8");
        assert_eq!(assets_mime_type(b"/r/a.svg"), b"image/svg+xml");
        assert_eq!(assets_mime_type(b"/r/a.js.map"), b"application/octet-stream");
        assert_eq!(assets_mime_type(b"/r/a.html"), b"text/html");
    }

    #[test]
    fn modified_since() {
        let t = crate::http::parse_http_date(b"Sun, 06 Nov 1994 08:49:37 GMT");
        assert!(t > 0);
        assert!(not_modified(t, b"Sun, 06 Nov 1994 08:49:37 GMT"));
        assert!(!not_modified(t + 1, b"Sun, 06 Nov 1994 08:49:37 GMT"));
        assert!(!not_modified(-1, b"Sun, 06 Nov 1994 08:49:37 GMT"));
        assert!(!not_modified(t, b""));
        assert!(!not_modified(t, b"yesterday"));
    }

    /// 乱数の入力でパニックしない
    #[test]
    fn fuzz() {
        let mut x: u32 = 8080;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        const ALPHA: &[u8] = b"/?.htmljscsvgpubi ,:0123GMTSunNov";
        for _ in 0..100_000 {
            let n = (rnd() % 32) as usize;
            let s: Vec<u8> = (0..n).map(|_| if rnd() % 6 == 0 { rnd() as u8 } else { ALPHA[(rnd() as usize) % ALPHA.len()] }).collect();
            let _ = route(&s);
            let _ = mime_type(&s);
            let _ = assets_mime_type(&s);
            let _ = not_modified(rnd() as i64 - 1, &s);
            let (p, q) = crate::http::split_request_url(&s);
            assert!(p.len() + q.len() <= s.len());
            let _ = acceptable_languages(&s);
        }
    }
}
