//! 仮想パスからファイルの場所を決める (core/common/mapper.cpp の `FileSystemMapper`) (段階 8c)。
//!
//! 要求のパスを文書のディレクトリの下のパスにすること、言語ごとのファイルを試す順、解決したパスが
//! 文書のディレクトリの中にあるか (ディレクトリトラバーサル) の判断。パスの解決 (`realpath`) は C++ 側。

use crate::strutil;

/// パスの区切り (`realpath` の返すもの)
const SEP: u8 = std::path::MAIN_SEPARATOR as u8;

/// `toLocalFilePath` の前半: `vpath` が `virtual_path` の下 (`virtual_path/...`) でなければ `None`。
/// 下なら、先頭の `virtual_path` を `document_root` に付け替えたパス。
pub fn local_path(virtual_path: &[u8], document_root: &[u8], vpath: &[u8]) -> Option<Vec<u8>> {
    if virtual_path == vpath || !vpath.starts_with(&[virtual_path, b"/"].concat()) {
        return None;
    }
    Some(strutil::replace_prefix(vpath, virtual_path, document_root))
}

/// `resolvePath` で試すパスと、その言語: パスそのもの、言語ごとの `.ext`、英語 (`.en`) の順。
pub fn candidates(raw: &[u8], langs: &[Vec<u8>]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut res = vec![(raw.to_vec(), Vec::new())];
    for ext in langs {
        res.push(([raw, b".", ext].concat(), ext.clone()));
    }
    res.push(([raw, &b".en"[..]].concat(), b"en".to_vec()));
    res
}

/// 解決したパスが、文書のディレクトリの中 (ディレクトリそのものではない) にあるか。
///
/// C++ 版は先頭が一致するかだけを見ていたので、文書のディレクトリと名前の先頭が同じ隣のディレクトリ
/// (`.../public` に対する `.../public2`) の中も通していた。Rust 版は、続きがパスの区切りであることも
/// 確かめる。
pub fn inside(document_root: &[u8], resolved: &[u8]) -> bool {
    if resolved == document_root || !resolved.starts_with(document_root) {
        return false;
    }
    document_root.last() == Some(&SEP) || resolved.get(document_root.len()) == Some(&SEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths() {
        assert_eq!(local_path(b"/public", b"/r/public", b"/public/a.html"), Some(b"/r/public/a.html".to_vec()));
        assert_eq!(local_path(b"/public", b"/r/public", b"/public"), None);
        assert_eq!(local_path(b"/public", b"/r/public", b"/publicx/a"), None);
        assert_eq!(local_path(b"/public", b"/r/public", b"/public/"), Some(b"/r/public/".to_vec()));
        assert_eq!(local_path(b"/public", b"/r/public", b"/public/../../etc"), Some(b"/r/public/../../etc".to_vec()));
    }

    #[test]
    fn candidate_order() {
        let c = candidates(b"/r/a.html", &[b"ja".to_vec(), b"fr".to_vec()]);
        let names: Vec<&[u8]> = c.iter().map(|(p, _)| p.as_slice()).collect();
        assert_eq!(names, vec![&b"/r/a.html"[..], b"/r/a.html.ja", b"/r/a.html.fr", b"/r/a.html.en"]);
        assert_eq!(c[0].1, b"");
        assert_eq!(c[3].1, b"en");
    }

    #[cfg(unix)]
    #[test]
    fn traversal() {
        assert!(inside(b"/r/public", b"/r/public/a.html"));
        assert!(!inside(b"/r/public", b"/r/public"));
        assert!(!inside(b"/r/public", b"/etc/passwd"));
        // 名前の先頭が同じ隣のディレクトリ (C++ 版は通していた)
        assert!(!inside(b"/r/public", b"/r/public2/secret"));
        assert!(inside(b"/", b"/etc"));
    }

    /// 乱数の入力でパニックせず、通すパスは必ず文書のディレクトリの下にある
    #[test]
    fn fuzz() {
        let mut x: u32 = 6060;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        const ALPHA: &[u8] = b"/.\\publicx2a";
        let mut s = || -> Vec<u8> {
            let n = (rnd() % 16) as usize;
            (0..n).map(|_| if rnd() % 8 == 0 { rnd() as u8 } else { ALPHA[(rnd() as usize) % ALPHA.len()] }).collect()
        };
        for _ in 0..100_000 {
            let (v, d, p) = (s(), s(), s());
            if let Some(l) = local_path(&v, &d, &p) {
                assert!(l.starts_with(&d));
            }
            if inside(&d, &p) {
                assert!(p.len() > d.len() && p.starts_with(&d));
            }
            let langs = vec![s(), s()];
            let c = candidates(&p, &langs);
            assert_eq!(c.len(), 4);
            assert!(c.iter().all(|(q, _)| q.starts_with(&p)));
        }
    }
}
