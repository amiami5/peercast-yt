//! メッセージカタログ (`catalogs/*.json`) と、`{#メッセージ}` の差し込み (もとは message-interpolate)。
//!
//! カタログは、文字列から文字列 (か null) への JSON のオブジェクトで、`//` と `/* */` のコメントを書ける
//! (Ruby の JSON.load と同じ)。同じキーが 2 度出てきたら後のものを使う。

use std::collections::BTreeMap;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err<T>(&self, what: &str) -> Result<T, String> {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&c| c == b'\n').count() + 1;
        Err(format!("line {}: {}", line, what))
    }

    fn skip_space(&mut self) -> Result<(), String> {
        loop {
            while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match self.s[self.i + 2..].windows(2).position(|w| w == b"*/") {
                    Some(p) => self.i += 2 + p + 2,
                    None => return self.err("unterminated comment"),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.skip_space()?;
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| "short \\u escape".to_string())?;
        let v = std::str::from_utf8(h).ok().and_then(|h| u32::from_str_radix(h, 16).ok());
        match v {
            Some(v) => {
                self.i += 4;
                Ok(v)
            }
            None => self.err("bad \\u escape"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = match self.s.get(self.i) {
                Some(&c) => c,
                None => return self.err("unterminated string"),
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = match self.s.get(self.i) {
                        Some(&e) => e,
                        None => return self.err("unterminated string"),
                    };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xd800..0xdc00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff)
                            } else {
                                hi
                            };
                            char::from_u32(cp).unwrap_or('\u{fffd}')
                        }
                        _ => return self.err("bad escape"),
                    };
                    let mut b = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).or_else(|_| self.err("not UTF-8"))
    }
}

/// カタログを読む
pub fn parse(text: &str) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let mut map = BTreeMap::new();
    p.expect(b'{')?;
    p.skip_space()?;
    if p.s.get(p.i) == Some(&b'}') {
        p.i += 1;
    } else {
        loop {
            let key = p.string()?;
            p.expect(b':')?;
            p.skip_space()?;
            let value = if p.s[p.i..].starts_with(b"null") {
                p.i += 4;
                None
            } else {
                Some(p.string()?)
            };
            map.insert(key, value);
            p.skip_space()?;
            match p.s.get(p.i) {
                Some(b',') => p.i += 1,
                Some(b'}') => {
                    p.i += 1;
                    break;
                }
                _ => return p.err("expected ',' or '}'"),
            }
        }
    }
    p.skip_space()?;
    if p.i != p.s.len() {
        return p.err("garbage after the object");
    }
    Ok(map)
}

/// `{#メッセージ}` をカタログの訳にする (訳がなければメッセージそのもの)
pub fn interpolate(text: &str, catalog: &BTreeMap<String, Option<String>>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{#") {
        match rest[i + 2..].find('}') {
            Some(j) => {
                let key = &rest[i + 2..i + 2 + j];
                out.push_str(&rest[..i]);
                match catalog.get(key) {
                    Some(Some(t)) => out.push_str(t),
                    _ => out.push_str(key),
                }
                rest = &rest[i + 2 + j + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs() {
        let c = parse("{\n // c\n \"a\" : \"x\\u3042\\\"\", /* b */ \"b\": null, \"a\": \"y\"\n}\n").unwrap();
        assert_eq!(c.get("a"), Some(&Some("y".to_string())));
        assert_eq!(c.get("b"), Some(&None));
        assert!(parse("{\"a\": 1}").is_err());
        let mut c = BTreeMap::new();
        c.insert("Hi".to_string(), Some("やあ".to_string()));
        c.insert("N".to_string(), None);
        assert_eq!(interpolate("{#Hi} {#N} {#Z} {#x", &c), "やあ N Z {#x");
    }
}
