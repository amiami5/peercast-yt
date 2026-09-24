//! マクロの展開 (もとは macro-expand)。
//!
//! * `{^define 名前}...{^end}`: 中身を展開したものを変数にする
//! * `{^define 名前 値}`: 値 (空白を含まない) を変数にする
//! * `{^include ファイル}`: `Templates/` の下のファイルを展開したもので置き換える
//! * `{^名前}`: 変数の値で置き換える (なければ誤り)
//!
//! 先に `Templates/defs.html` を展開し (出力は捨てる)、ページを展開したあと、変数 `LAYOUT` があれば
//! 変数 `yield` にページを入れて `Templates/<LAYOUT>` を展開したものを出力にする。

use std::collections::BTreeMap;
use std::path::Path;

type Env = BTreeMap<String, String>;

/// Ruby の `\s`
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c')
}

/// Ruby の `\w` (ASCII)
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `/\{\^[^}]+\}|[^{]+|\{/` で切る
fn tokenize(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            if b.get(i + 1) == Some(&b'^') {
                if let Some(j) = s[i + 2..].find('}') {
                    if j > 0 {
                        toks.push(&s[i..i + 2 + j + 1]);
                        i += 2 + j + 1;
                        continue;
                    }
                }
            }
            toks.push(&s[i..i + 1]);
            i += 1;
        } else {
            let j = s[i..].find('{').map_or(b.len(), |j| i + j);
            toks.push(&s[i..j]);
            i = j;
        }
    }
    toks
}

/// `word` で始まり、空白が 1 つ以上続くなら、その後ろ
fn after_keyword<'a>(stmt: &'a str, word: &str) -> Option<&'a str> {
    let rest = stmt.strip_prefix(word)?;
    let trimmed = rest.trim_start_matches(is_space);
    if trimmed.len() == rest.len() {
        None
    } else {
        Some(trimmed)
    }
}

enum Tag<'a> {
    DefineBlock(&'a str),
    DefineValue(&'a str, &'a str),
    Include(&'a str),
    Var(&'a str),
}

fn parse_tag(stmt: &str) -> Tag<'_> {
    if let Some(rest) = after_keyword(stmt, "define") {
        // (\w+)\s*\z か (\w+)\s+(\S+)\s*\z
        let n = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        if n > 0 {
            let name = &rest[..n];
            let after = &rest[n..];
            if after.chars().all(is_space) {
                return Tag::DefineBlock(name);
            }
            let value_part = after.trim_start_matches(is_space);
            if value_part.len() < after.len() {
                let m = value_part.find(is_space).unwrap_or(value_part.len());
                if m > 0 && value_part[m..].chars().all(is_space) {
                    return Tag::DefineValue(name, &value_part[..m]);
                }
            }
        }
    }
    if let Some(rest) = after_keyword(stmt, "include") {
        let m = rest.find(is_space).unwrap_or(rest.len());
        if m > 0 && rest[m..].chars().all(is_space) {
            return Tag::Include(&rest[..m]);
        }
    }
    // Ruby の String#strip (前後の空白と NUL)
    Tag::Var(stmt.trim_matches(|c: char| is_space(c) || c == '\0'))
}

struct Expander<'a> {
    templates: &'a Path,
    env: Env,
    depth: usize,
}

impl Expander<'_> {
    fn eval_toks(&mut self, toks: &[&str], pos: &mut usize) -> Result<String, String> {
        let mut res = String::new();
        while *pos < toks.len() {
            let tok = toks[*pos];
            *pos += 1;
            if tok.starts_with("{^") && tok.len() > 3 && tok.ends_with('}') {
                let stmt = &tok[2..tok.len() - 1];
                if stmt == "end" {
                    return Ok(res);
                }
                res.push_str(&self.tag(stmt, toks, pos)?);
            } else {
                res.push_str(tok);
            }
        }
        Ok(res)
    }

    fn tag(&mut self, stmt: &str, toks: &[&str], pos: &mut usize) -> Result<String, String> {
        match parse_tag(stmt) {
            Tag::DefineBlock(name) => {
                let v = self.eval_toks(toks, pos)?;
                self.env.insert(name.to_string(), v);
                Ok(String::new())
            }
            Tag::DefineValue(name, value) => {
                self.env.insert(name.to_string(), value.to_string());
                Ok(String::new())
            }
            Tag::Include(file) => {
                let text = self.read(file)?;
                self.eval(&text)
            }
            Tag::Var(name) => match self.env.get(name) {
                Some(v) => Ok(v.clone()),
                None => Err(format!("undefined variable {}", name)),
            },
        }
    }

    fn eval(&mut self, text: &str) -> Result<String, String> {
        self.depth += 1;
        if self.depth > 100 {
            return Err("include nesting too deep".to_string());
        }
        let toks = tokenize(text);
        let mut pos = 0;
        let r = self.eval_toks(&toks, &mut pos);
        self.depth -= 1;
        r
    }

    fn read(&self, file: &str) -> Result<String, String> {
        let p = self.templates.join(file);
        let b = std::fs::read(&p).map_err(|e| format!("{}: {}", p.display(), e))?;
        String::from_utf8(b).map_err(|_| format!("{}: not UTF-8", p.display()))
    }
}

/// ページを展開する
pub fn expand_page(templates: &Path, page: &str) -> Result<String, String> {
    let mut x = Expander { templates, env: Env::new(), depth: 0 };
    let defs = x.read("defs.html")?;
    x.eval(&defs)?;
    let html = x.eval(page)?;
    match x.env.get("LAYOUT").cloned() {
        Some(layout) => {
            x.env.insert("yield".to_string(), html);
            let text = x.read(&layout)?;
            x.eval(&text)
        }
        None => Ok(html),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(tokenize("a{^b}c{d{^}e"), vec!["a", "{^b}", "c", "{", "d", "{", "^}e"]);
    }

    #[test]
    fn expand() {
        let mut x = Expander { templates: Path::new("/nonexistent"), env: Env::new(), depth: 0 };
        assert_eq!(x.eval("{^define B 1}{^define A}x{^B}y{^end}").unwrap(), "");
        assert_eq!(x.eval("{^A}").unwrap(), "x1y");
        // A は定義したとき (B がない) に展開するので、誤りになる
        let mut x2 = Expander { templates: Path::new("/nonexistent"), env: Env::new(), depth: 0 };
        assert!(x2.eval("{^define A}x{^B}y{^end}").is_err());
        assert_eq!(x.eval("[{^ B }]").unwrap(), "[1]");
        assert!(x.eval("{^define A b c}").is_err());
    }
}
