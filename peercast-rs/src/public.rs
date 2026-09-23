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
}
