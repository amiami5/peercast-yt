//! PeerCast YT のブラウザ UI を作る (もとは Ruby の ui/generate-html、generate-public、macro-expand、
//! message-interpolate と sed)。
//!
//! `UI_DIR/catalogs/*.json` の言語ごとに、
//! * `UI_DIR/html-master/` の下を `OUT_DIR/html/<言語>/` に
//! * `UI_DIR/public-master/` の下を `OUT_DIR/public/` に (HTML は `<名前>.<言語>`)
//!
//! 書き出す。HTML は、マクロの展開 (`{^define ...}`、`{^include ...}`、`{^変数}`、`Templates/` の下の
//! ファイル)、メッセージの差し込み (`{#メッセージ}` をカタログの訳に)、コメントと行頭の空白と空行の
//! 削除をする。ほかのファイル (画像など) はそのまま写す。

mod catalog;
mod macros;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

/// `ui` (リポジトリの ui/) から、`out` の下に html/ と public/ を作る
pub fn run(ui: &Path, out: &Path) -> Result<()> {
    let langs = languages(ui)?;
    for (lang, catalog) in &langs {
        let dest = out.join("html").join(lang);
        generate(ui, &ui.join("html-master"), &dest, catalog, None)?;
    }
    for (lang, catalog) in &langs {
        generate(ui, &ui.join("public-master"), &out.join("public"), catalog, Some(lang))?;
    }
    Ok(())
}

/// `catalogs/*.json` の言語とカタログ (名前の順)
fn languages(ui: &Path) -> Result<Vec<(String, BTreeMap<String, Option<String>>)>> {
    let dir = ui.join("catalogs");
    let mut res = Vec::new();
    for e in read_dir_sorted(&dir)? {
        let name = file_name(&e);
        if let Some(lang) = name.strip_suffix(".json") {
            let text = read_string(&e)?;
            let mut cat = catalog::parse(&text).map_err(|m| format!("{}: {}", e.display(), m))?;
            cat.insert("#lang".to_string(), Some(lang.to_string()));
            res.push((lang.to_string(), cat));
        }
    }
    if res.is_empty() {
        return Err(format!("{}: no catalogs", dir.display()));
    }
    Ok(res)
}

/// `src` の下を `dest` の下に作る。`lang_suffix` があれば HTML の名前の後ろに `.<言語>` を付ける
fn generate(ui: &Path, src: &Path, dest: &Path, catalog: &BTreeMap<String, Option<String>>, lang_suffix: Option<&str>) -> Result<()> {
    mkdir_p(dest)?;
    for path in read_dir_sorted(src)? {
        let name = file_name(&path);
        let meta = fs::metadata(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        if meta.is_dir() {
            generate(ui, &path, &dest.join(&name), catalog, lang_suffix)?;
        } else if name.ends_with(".htm") || name.ends_with(".html") {
            let out_name = match lang_suffix {
                Some(l) => format!("{}.{}", name, l),
                None => name.clone(),
            };
            let text = read_string(&path)?;
            let html = process_html(ui, &text, catalog).map_err(|e| format!("{}: {}", path.display(), e))?;
            let out_path = dest.join(out_name);
            fs::write(&out_path, html).map_err(|e| format!("{}: {}", out_path.display(), e))?;
        } else {
            let out_path = dest.join(&name);
            fs::copy(&path, &out_path).map_err(|e| format!("{} -> {}: {}", path.display(), out_path.display(), e))?;
        }
    }
    Ok(())
}

/// HTML 1 つ分: マクロの展開 → メッセージの差し込み → コメントと空白の削除
pub fn process_html(ui: &Path, text: &str, catalog: &BTreeMap<String, Option<String>>) -> Result<String> {
    let expanded = macros::expand_page(&ui.join("Templates"), text)?;
    let interpolated = catalog::interpolate(&expanded, catalog);
    Ok(clean_lines(&interpolated))
}

/// `sed -r 's/<!--[^-]+-->//g' | sed -r -e 's/^\s+//g' -e 's/\r//' -e '/^$/d'`
pub fn clean_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let (line, had_newline) = match rest.find('\n') {
            Some(i) => {
                let l = &rest[..i];
                rest = &rest[i + 1..];
                (l, true)
            }
            None => {
                let l = rest;
                rest = "";
                (l, false)
            }
        };
        let line = remove_comments(line);
        // 行頭の空白 ([[:space:]]。改行は行に含まれない)
        let line = line.trim_start_matches([' ', '\t', '\r', '\x0b', '\x0c']);
        // 最初の CR を 1 つだけ消す
        let line = match line.find('\r') {
            Some(i) => format!("{}{}", &line[..i], &line[i + 1..]),
            None => line.to_string(),
        };
        if line.is_empty() {
            continue;
        }
        out.push_str(&line);
        if had_newline {
            out.push('\n');
        }
    }
    out
}

/// 行の中の `<!--[^-]+-->` をすべて消す (中に `-` があるコメントは消さない)
fn remove_comments(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let mut copied = 0;
    while let Some(off) = line[i..].find("<!--") {
        let start = i + off;
        let body = start + 4;
        let mut k = body;
        while k < b.len() && b[k] != b'-' {
            k += 1;
        }
        if k > body && line[k..].starts_with("-->") {
            out.push_str(&line[copied..start]);
            i = k + 3;
            copied = i;
        } else {
            i = start + 1;
        }
    }
    out.push_str(&line[copied..]);
    out
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    v.sort();
    Ok(v)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn read_string(p: &Path) -> Result<String> {
    let b = fs::read(p).map_err(|e| format!("{}: {}", p.display(), e))?;
    String::from_utf8(b).map_err(|_| format!("{}: not UTF-8", p.display()))
}

fn mkdir_p(p: &Path) -> Result<()> {
    fs::create_dir_all(p).map_err(|e| format!("{}: {}", p.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning() {
        assert_eq!(clean_lines("  a<!-- x -->b\n\n\t\r\nc\r\r\n  <!-- a-b -->"), "ab\nc\r\n<!-- a-b -->");
        assert_eq!(clean_lines("<!---->x<!--y-->\n"), "<!---->x\n");
        assert_eq!(clean_lines("a\n  \n"), "a\n");
    }
}
