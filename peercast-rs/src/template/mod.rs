//! HTML テンプレートエンジン (core/common/template.cpp の `Template`)。
//!
//! テンプレートの読み出し (`{$式}`, `{@if}` などのディレクティブ)、式の字句解析・構文解析・評価を
//! 行う。変数を持つスコープ (`servMgr` などの状態、`GenericScope`) と、`=~` の正規表現
//! (`std::regex`) は C++ 側に残り、`Host` のコールバックで使う。
//!
//! C++ 版は、`==` やの関数の引数などの評価順序を決めていなかった (未規定)。Rust 版は、
//! x86-64 の GCC でビルドした C++ 版と同じ順序にしている (右辺から評価する箇所がある)。

pub mod value;

use std::collections::{BTreeMap, VecDeque};

use crate::reader::Abort;
pub use value::Value;
use value::{is_truish, to_s};

/// `Template::TMPL_*`
pub const TMPL_UNKNOWN: i32 = 0;
pub const TMPL_LOOP: i32 = 1;
pub const TMPL_IF: i32 = 2;
pub const TMPL_ELSE: i32 = 3;
pub const TMPL_ELSIF: i32 = 4;
pub const TMPL_END: i32 = 5;
pub const TMPL_FRAGMENT: i32 = 6;
pub const TMPL_FOREACH: i32 = 7;
pub const TMPL_LET: i32 = 8;

/// 式の入れ子と、評価の入れ子 (関数の再帰呼び出しなど) の上限。C++ 版は上限がなく、
/// 深すぎるとスタックを使い果たして落ちていた。
pub const MAX_DEPTH: usize = 200;

/// C++ 側で投げる例外の種類とメッセージ
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// コールバックの中で起きた C++ の例外 (C++ 側で投げ直す)
    Abort,
    /// `GeneralException`
    General(Vec<u8>),
    /// `StreamException`
    Stream(Vec<u8>),
    /// `std::runtime_error` (`amf0::Value` の型の取り違え)
    Runtime(Vec<u8>),
    /// `std::out_of_range` (`std::vector::at`)
    OutOfRange(Vec<u8>),
    /// `std::invalid_argument` (`str::json_inspect`)
    InvalidArgument(Vec<u8>),
}

impl Error {
    fn general(parts: &[&[u8]]) -> Error {
        Error::General(parts.concat())
    }
    fn stream(parts: &[&[u8]]) -> Error {
        Error::Stream(parts.concat())
    }
    fn runtime(msg: &str) -> Error {
        Error::Runtime(msg.as_bytes().to_vec())
    }
}

impl From<Abort> for Error {
    fn from(_: Abort) -> Self {
        Error::Abort
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// テンプレートから見た C++ 側 (入力の `Stream`、出力、スコープ、正規表現)。
pub trait Host {
    /// テンプレートの `Stream::readChar`
    fn read_char(&mut self) -> std::result::Result<u8, Abort>;
    fn eof(&mut self) -> std::result::Result<bool, Abort>;
    fn position(&mut self) -> std::result::Result<i32, Abort>;
    fn seek(&mut self, pos: i32) -> std::result::Result<(), Abort>;
    /// 出力 (`outp`)
    fn write(&mut self, data: &[u8]) -> std::result::Result<(), Abort>;
    /// `Template::writeVariable` (見つからなければ null)。名前は `String` に変換される
    /// (NUL まで、255 バイトまで)。
    fn lookup(&mut self, name: &[u8]) -> std::result::Result<Value, Abort>;
    /// 新しい `GenericScope` を先頭に置く
    fn push_scope(&mut self);
    fn pop_scope(&mut self);
    /// 先頭のスコープが `GenericScope` か
    fn front_is_generic(&mut self) -> bool;
    /// 先頭のスコープ (`GenericScope`) の変数を設定する
    fn set_front(&mut self, name: &[u8], value: &Value) -> std::result::Result<(), Abort>;
    /// `Regexp` を作る (不正な正規表現なら例外)
    fn regex_check(&mut self, pattern: &[u8]) -> std::result::Result<(), Abort>;
    /// `Regexp(pattern).matches(subject)`
    fn regex_match(&mut self, pattern: &[u8], subject: &[u8]) -> std::result::Result<bool, Abort>;
    fn selected_fragment(&mut self) -> Vec<u8>;
    fn current_fragment(&mut self) -> Vec<u8>;
    fn set_current_fragment(&mut self, f: &[u8]);
    fn log_error(&mut self, msg: &str);
}

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// C++ の `String` に入れたときの名前 (NUL まで、`MAX_LEN - 1` = 255 バイトまで)
fn string_name(s: &[u8]) -> &[u8] {
    let s = until_nul(s);
    &s[..s.len().min(255)]
}

/// `std::vector::at`
fn at(arr: &[Value], i: usize) -> Result<&Value> {
    arr.get(i).ok_or_else(|| {
        Error::OutOfRange(
            format!("vector::_M_range_check: __n (which is {}) >= this->size() (which is {})", i, arr.len()).into_bytes(),
        )
    })
}

// ---------------------------------------------------------------- 字句解析

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.'
}

/// `Template::evalStringLiteral`: 引用符を外し、バックスラッシュの次の文字をそのまま使う
pub fn eval_string_literal(input: &[u8]) -> Result<Vec<u8>> {
    let first = input.first().copied().unwrap_or(0);
    if first != b'"' && first != b'\'' {
        return Err(Error::stream(&[b"Malformed string literal: ", input]));
    }
    let quote = first;
    if input[input.len() - 1] != quote {
        return Err(Error::stream(&[b"Malformed string literal: ", input]));
    }
    let mut res = Vec::new();
    let mut s = &input[1..];
    while !s.is_empty() && s[0] != quote {
        if s[0] == b'\\' {
            res.push(s.get(1).copied().unwrap_or(0));
            s = &s[s.len().min(2)..];
        } else {
            res.push(s[0]);
            s = &s[1..];
        }
    }
    if s.is_empty() {
        return Err(Error::stream(&[b"Premature end of string: ", input]));
    }
    Ok(res)
}

/// `Template::readStringLiteral`: 先頭の文字列リテラル (引用符付き) と残りに分ける
pub fn read_string_literal(input: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    if input.is_empty() {
        return Err(Error::stream(&[b"empty input"]));
    }
    if input[0] != b'"' && input[0] != b'\'' {
        return Err(Error::stream(&[b"no string literal"]));
    }
    let quote = input[0];
    let mut res = vec![quote];
    let mut s = &input[1..];
    while !s.is_empty() && s[0] != quote {
        if s[0] == b'\\' {
            res.push(s[0]);
            s = &s[1..];
            if !s.is_empty() {
                res.push(s[0]);
                s = &s[1..];
            }
        } else {
            res.push(s[0]);
            s = &s[1..];
        }
    }
    if s.is_empty() {
        return Err(Error::stream(&[b"Premature end of string: ", input]));
    }
    res.push(quote);
    Ok((res, s[1..].to_vec()))
}

/// `Template::tokenize`
pub fn tokenize(input: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut s: Vec<u8> = input.to_vec();
    let mut pos = 0;
    let mut tokens = Vec::new();
    while pos < s.len() {
        let rest = &s[pos..];
        if is_space(rest[0]) {
            pos += 1;
            while pos < s.len() && is_space(s[pos]) {
                pos += 1;
            }
        } else if [&b"=="[..], b"!=", b"=~", b"!~"].iter().any(|op| rest.starts_with(op)) {
            tokens.push(rest[..2].to_vec());
            pos += 2;
        } else if b"()!,={}:[]".contains(&rest[0]) {
            tokens.push(vec![rest[0]]);
            pos += 1;
        } else if rest[0] == b'"' || rest[0] == b'\'' {
            let (t, remaining) = read_string_literal(rest)?;
            tokens.push(t);
            s = remaining;
            pos = 0;
        } else if is_ident(rest[0]) {
            let n = rest.iter().take_while(|&&c| is_ident(c)).count();
            tokens.push(rest[..n].to_vec());
            pos += n;
        } else {
            let msg = crate::inspect::inspect(&rest[..1]);
            return Err(Error::stream(&[b"Unrecognized token. Error at ", &msg]));
        }
    }
    Ok(tokens)
}

// ---------------------------------------------------------------- 構文解析

/// C++ 版がトークンの判定に使っていた正規表現 (`std::regex_search`)。同じ判定を手で書いたもの。
#[derive(Clone, Copy)]
enum Sym {
    Op,
    Not,
    Ident,
    LParen,
    RParen,
    Str,
    Comma,
    Colon,
    Assign,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
}

impl Sym {
    /// 元の正規表現 (エラーメッセージに使う)
    fn source(self) -> &'static [u8] {
        match self {
            Sym::Op => b"^==|=~|!=|!~$",
            Sym::Not => b"^!$",
            Sym::Ident => b"^[A-z0-9_.]+$",
            Sym::LParen => b"^\\($",
            Sym::RParen => b"^\\)$",
            Sym::Str => b"^\".*?\"|'.*?'$",
            Sym::Comma => b"^,$",
            Sym::Colon => b"^:$",
            Sym::Assign => b"^=$",
            Sym::LBrace => b"^\\{$",
            Sym::RBrace => b"^\\}$",
            Sym::LBracket => b"^\\[$",
            Sym::RBracket => b"^\\]$",
        }
    }

    fn matches(self, t: &[u8]) -> bool {
        let contains = |pat: &[u8]| t.windows(pat.len()).any(|w| w == pat);
        // ECMAScript の `.` は改行 (\n と \r) にだけ一致しない
        let line_end = |c: u8| c == b'\n' || c == b'\r';
        match self {
            // ^== | =~ | != | !~$ (選択肢ごとに ^ と $ が付く)
            Sym::Op => t.starts_with(b"==") || contains(b"=~") || contains(b"!=") || t.ends_with(b"!~"),
            Sym::Not => t == b"!",
            // [A-z] は 'A' (0x41) から 'z' (0x7a) まで
            Sym::Ident => !t.is_empty() && t.iter().all(|&c| (b'A'..=b'z').contains(&c) || c.is_ascii_digit() || c == b'.'),
            Sym::LParen => t == b"(",
            Sym::RParen => t == b")",
            // ^".*?" | '.*?'$
            Sym::Str => {
                let dq = t.first() == Some(&b'"')
                    && t[1..].iter().find(|&&c| c == b'"' || line_end(c)) == Some(&b'"');
                let sq = t.len() >= 2
                    && t.last() == Some(&b'\'')
                    && t[..t.len() - 1].iter().rev().find(|&&c| c == b'\'' || line_end(c)) == Some(&b'\'');
                dq || sq
            }
            Sym::Comma => t == b",",
            Sym::Colon => t == b":",
            Sym::Assign => t == b"=",
            Sym::LBrace => t == b"{",
            Sym::RBrace => t == b"}",
            Sym::LBracket => t == b"[",
            Sym::RBracket => t == b"]",
        }
    }
}

fn sa(v: Vec<Value>) -> Value {
    Value::StrictArray(v)
}

struct Parser<'a> {
    tokens: &'a mut VecDeque<Vec<u8>>,
    depth: usize,
}

impl Parser<'_> {
    fn accept(&mut self, sym: Sym) -> Option<Vec<u8>> {
        if self.tokens.front().is_some_and(|t| sym.matches(t)) {
            self.tokens.pop_front()
        } else {
            None
        }
    }

    fn expect(&mut self, sym: Sym) -> Result<()> {
        match self.tokens.front() {
            None => Err(Error::general(&[b"Premature end while expecting ", sym.source()])),
            Some(t) if !sym.matches(t) => Err(Error::general(&[b"Got ", t, b" while expecting ", sym.source()])),
            Some(_) => {
                self.tokens.pop_front();
                Ok(())
            }
        }
    }

    fn peek(&self, sym: Sym) -> bool {
        self.tokens.front().is_some_and(|t| sym.matches(t))
    }

    /// `exp()` を呼ぶ (入れ子の深さを数える)
    fn exp(&mut self) -> Result<Option<Value>> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::general(&[b"Template: nesting too deep"]));
        }
        let r = self.exp_inner();
        self.depth -= 1;
        r
    }

    /// 引数の並び `e (',' e)* ')'` (開きかっこは読んだあと)。途中で式がなければ `None`。
    fn args(&mut self, mut f: Vec<Value>, close: Sym, missing_is_error: bool) -> Result<Option<Value>> {
        loop {
            match self.exp()? {
                Some(e) => f.push(e),
                None if missing_is_error => return Err(Error::general(&[b"exp expected"])),
                None => return Ok(None),
            }
            if self.accept(Sym::Comma).is_some() {
                continue;
            }
            self.expect(close)?;
            return Ok(Some(sa(f)));
        }
    }

    // EXP := EXP3 ( '(' 引数 ')' | '[' EXP ']' | OP EXP )?
    fn exp_inner(&mut self) -> Result<Option<Value>> {
        let e = match self.exp3()? {
            Some(e) => e,
            None => return Ok(None),
        };
        if self.peek(Sym::LParen) {
            self.expect(Sym::LParen)?;
            let f = vec![e];
            if self.accept(Sym::RParen).is_some() {
                return Ok(Some(sa(f)));
            }
            self.args(f, Sym::RParen, true)
        } else if self.peek(Sym::LBracket) {
            self.expect(Sym::LBracket)?;
            let mut f = vec![Value::str(b"prop"), e];
            match self.exp()? {
                Some(e2) => f.push(e2),
                None => return Err(Error::general(&[b"exp expected"])),
            }
            self.expect(Sym::RBracket)?;
            Ok(Some(sa(f)))
        } else if self.peek(Sym::Op) {
            // 右結合になっちゃった。
            let op = self.accept(Sym::Op).unwrap();
            Ok(self.exp()?.map(|e2| sa(vec![Value::String(op), e, e2])))
        } else {
            Ok(Some(e))
        }
    }

    // EXP3 := '!' EXP | '{' (EXP ':' EXP (',' ...)*)? '}' | '[' (EXP (',' EXP)*)? ']' | EXP2
    fn exp3(&mut self) -> Result<Option<Value>> {
        if self.accept(Sym::Not).is_some() {
            Ok(self.exp()?.map(|r| sa(vec![Value::str(b"!"), r])))
        } else if self.accept(Sym::LBrace).is_some() {
            let mut f = vec![Value::str(b"object")];
            if self.accept(Sym::RBrace).is_some() {
                return Ok(Some(sa(f)));
            }
            loop {
                match self.exp()? {
                    Some(e) => f.push(e),
                    None => return Ok(None),
                }
                self.expect(Sym::Colon)?;
                match self.exp()? {
                    Some(e) => f.push(e),
                    None => return Ok(None),
                }
                if self.accept(Sym::Comma).is_some() {
                    continue;
                }
                self.expect(Sym::RBrace)?;
                return Ok(Some(sa(f)));
            }
        } else if self.accept(Sym::LBracket).is_some() {
            let f = vec![Value::str(b"array")];
            if self.accept(Sym::RBracket).is_some() {
                return Ok(Some(sa(f)));
            }
            self.args(f, Sym::RBracket, false)
        } else {
            self.exp2()
        }
    }

    // EXP2 := IDENT ('(' 引数 ')')? | '(' EXP ')' '(' 引数 ')' | STRING
    fn exp2(&mut self) -> Result<Option<Value>> {
        if let Some(ident) = self.accept(Sym::Ident) {
            if self.accept(Sym::LParen).is_some() {
                let f = vec![Value::String(ident)];
                if self.accept(Sym::RParen).is_some() {
                    return Ok(Some(sa(f)));
                }
                self.args(f, Sym::RParen, false)
            } else {
                Ok(Some(Value::String(ident)))
            }
        } else if self.accept(Sym::LParen).is_some() {
            match self.exp()? {
                Some(e) => {
                    self.expect(Sym::RParen)?;
                    self.expect(Sym::LParen)?;
                    let f = vec![e];
                    if self.accept(Sym::RParen).is_some() {
                        return Ok(Some(sa(f)));
                    }
                    self.args(f, Sym::RParen, false)
                }
                None => Ok(None),
            }
        } else if let Some(s) = self.accept(Sym::Str) {
            Ok(Some(sa(vec![Value::str(b"quote"), Value::String(eval_string_literal(&s)?)])))
        } else {
            Ok(None)
        }
    }
}

/// `Template::parse`: トークンを読んだ分だけ取り除き、式 (リスト形式) を返す
pub fn parse(tokens: &mut VecDeque<Vec<u8>>) -> Result<Value> {
    Parser { tokens, depth: 0 }.exp()?.ok_or_else(|| Error::general(&[b" something weird "]))
}

/// `Template::parseLetSpec`: `名前 = 式 (, 名前 = 式)*`
pub fn parse_let_spec(tokens: &mut VecDeque<Vec<u8>>) -> Result<Vec<(Vec<u8>, Value)>> {
    let mut result = Vec::new();
    loop {
        let ident = Parser { tokens, depth: 0 }.accept(Sym::Ident);
        match ident {
            Some(ident) => {
                Parser { tokens, depth: 0 }.expect(Sym::Assign)?;
                let exp = parse(tokens)?;
                result.push((ident, exp));
                if tokens.is_empty() {
                    break;
                }
                Parser { tokens, depth: 0 }.expect(Sym::Comma)?;
            }
            None => return Err(Error::general(&[b"Identifier expected"])),
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------- 評価と読み出し

/// glibc の `atoi` (`strtol` の結果を int に切り詰める)。数字だけの文字列に使う。
fn atoi_digits(s: &[u8]) -> i32 {
    let mut v: i64 = 0;
    for &c in s {
        v = v.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    v as i32
}

/// `std::stoi`。数字がなければ `None` (`std::invalid_argument`)、`int` に収まらなければ
/// `Err` ("stoi")。
fn stoi(s: &[u8]) -> std::result::Result<Option<i32>, ()> {
    let s = until_nul(s);
    let mut i = s.iter().take_while(|&&c| is_space(c)).count();
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let digits = s[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return Ok(None);
    }
    let mut v: i64 = 0;
    for &c in &s[i..i + digits] {
        v = v.checked_mul(10).and_then(|v| v.checked_add((c - b'0') as i64)).ok_or(())?;
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).map(Some).map_err(|_| ())
}

/// 1 回の呼び出しの間の状態 (出力は溜めておき、最後にまとめて書く)
pub struct Engine<'a> {
    h: &'a mut dyn Host,
    out: Vec<u8>,
    depth: usize,
}

impl<'a> Engine<'a> {
    pub fn new(h: &'a mut dyn Host) -> Engine<'a> {
        Engine { h, out: Vec::new(), depth: 0 }
    }

    /// 溜めた出力を書く (エラーのときも、それまでの出力は C++ 版と同じく書く)
    pub fn finish<T>(mut self, r: Result<T>) -> Result<T> {
        if !self.out.is_empty() {
            let out = std::mem::take(&mut self.out);
            self.h.write(&out)?;
        }
        r
    }

    fn lookup(&mut self, name: &[u8]) -> Result<Value> {
        Ok(self.h.lookup(until_nul(name))?)
    }

    fn in_selected_fragment(&mut self) -> bool {
        let sel = self.h.selected_fragment();
        sel.is_empty() || sel == self.h.current_fragment()
    }

    /// `Template::apply`: `lambda([引数...], 式...)` を呼ぶ
    pub fn apply(&mut self, lambda: &Value, arr: &[Value]) -> Result<Value> {
        let name = at(arr, 0)?.inspect()?;
        let not_function = || Error::general(&[&name, b" is not a function"]);

        let array = match lambda {
            Value::StrictArray(a) => a,
            _ => return Err(not_function()),
        };
        if !(array.len() > 2 && array[0] == Value::str(b"lambda")) {
            return Err(not_function());
        }
        let params = match &array[1] {
            Value::StrictArray(p) => p,
            _ => return Err(not_function()),
        };
        if !(!params.is_empty() && params[0] == Value::str(b"array")) {
            return Err(not_function());
        }

        // 引数は呼び出し元のスコープで評価する (右辺、名前の順)
        let mut frame = Vec::new();
        for i in 1..params.len() {
            let v = if i >= arr.len() { Value::Null } else { self.eval(&arr[i])? };
            let n = params[i].string()?.to_vec();
            frame.push((n, v));
        }

        self.h.push_scope();
        for (n, v) in &frame {
            self.h.set_front(n, v)?;
        }
        let mut v = Value::Null;
        for i in 2..array.len() {
            if i < array.len() - 1 {
                self.eval(&array[i])?;
            } else {
                v = self.eval(&array[i])?;
            }
        }
        self.h.pop_scope();
        Ok(v)
    }

    /// `Template::evalExpression(const amf0::Value&)`
    pub fn eval(&mut self, exp: &Value) -> Result<Value> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::general(&[b"Template: nesting too deep"]));
        }
        let r = match exp {
            Value::StrictArray(a) => self.eval_form(a),
            Value::String(s) => {
                if !s.is_empty() && s.iter().all(|c| c.is_ascii_digit()) {
                    Ok(Value::Number(atoi_digits(s) as f64))
                } else {
                    // 変数 (Template::writeVariable は、見つからなければ null を返す)
                    self.lookup(s)
                }
            }
            _ => Err(Error::general(&[b"evalExpression: unknown type of expression"])),
        };
        self.depth -= 1;
        r
    }

    fn eval_at(&mut self, arr: &[Value], i: usize) -> Result<Value> {
        let e = at(arr, i)?;
        self.eval(e)
    }

    fn eval_string_at(&mut self, arr: &[Value], i: usize) -> Result<Vec<u8>> {
        Ok(self.eval_at(arr, i)?.string()?.to_vec())
    }

    /// `Template::evalForm`
    pub fn eval_form(&mut self, arr: &[Value]) -> Result<Value> {
        if arr.is_empty() {
            return Err(Error::general(&[b"empty list form"]));
        }
        let name = match &arr[0] {
            Value::String(s) => s.as_slice(),
            head => {
                let f = self.eval(head)?;
                return self.apply(&f, arr);
            }
        };
        let n = arr.len();
        match name {
            // 二項演算子は右辺から評価する
            b"==" | b"!=" => {
                let b = self.eval_at(arr, 2)?;
                let a = self.eval_at(arr, 1)?;
                Ok(Value::Bool((a == b) == (name == b"==")))
            }
            b"=~" | b"!~" => {
                let pattern = self.eval_string_at(arr, 2)?;
                self.h.regex_check(&pattern)?;
                let subject = self.eval_string_at(arr, 1)?;
                let m = self.h.regex_match(&pattern, &subject)?;
                Ok(Value::Bool(m == (name == b"=~")))
            }
            b"!" => Ok(Value::Bool(!is_truish(&self.eval_at(arr, 1)?))),
            b"quote" => Ok(at(arr, 1)?.clone()),
            b"length" => Ok(Value::Number(self.eval_at(arr, 1)?.strict_array()?.len() as f64)),
            b"inspect" => Ok(Value::String(self.eval_at(arr, 1)?.inspect()?)),
            b"if" => match n {
                3 => {
                    if is_truish(&self.eval_at(arr, 1)?) {
                        self.eval_at(arr, 2)
                    } else {
                        Ok(Value::Null)
                    }
                }
                4 => {
                    if is_truish(&self.eval_at(arr, 1)?) {
                        self.eval_at(arr, 2)
                    } else {
                        self.eval_at(arr, 3)
                    }
                }
                _ => Err(Error::general(&[b"malformed if form"])),
            },
            b"cond" => {
                if n % 2 != 1 {
                    return Err(Error::general(&[b"Malformed cond"]));
                }
                for i in (1..n).step_by(2) {
                    if is_truish(&self.eval_at(arr, i)?) {
                        return self.eval_at(arr, i + 1);
                    }
                }
                Ok(Value::Null)
            }
            b"and" | b"or" => {
                let is_and = name == b"and";
                let mut v = Value::Bool(is_and);
                for e in &arr[1..] {
                    v = self.eval(e)?;
                    if is_truish(&v) != is_and {
                        break;
                    }
                }
                Ok(v)
            }
            b"object" => {
                if (n - 1) % 2 != 0 {
                    return Err(Error::general(&[b"object: Odd number of arguments given"]));
                }
                let mut object = BTreeMap::new();
                for i in (1..n).step_by(2) {
                    let key = self.eval_at(arr, i)?;
                    let key = match key {
                        Value::String(k) => k,
                        _ => return Err(Error::general(&[b"object: Non-string key"])),
                    };
                    let value = self.eval_at(arr, i + 1)?;
                    object.insert(key, value);
                }
                Ok(Value::Object(object))
            }
            b"array" => {
                let mut array = Vec::new();
                for e in &arr[1..] {
                    array.push(self.eval(e)?);
                }
                Ok(Value::StrictArray(array))
            }
            b"merge" => {
                if n < 2 {
                    return Err(Error::general(&[b"merge: Wrong number of arguments"]));
                }
                let mut result = self.eval(&arr[1])?.object()?.clone();
                for e in &arr[2..] {
                    let evaluated = self.eval(e)?;
                    for (k, v) in evaluated.object()? {
                        result.insert(k.clone(), v.clone());
                    }
                }
                Ok(Value::Object(result))
            }
            b"toQueryString" => {
                if n != 2 {
                    return Err(Error::general(&[b"merge: Wrong number of arguments"]));
                }
                let dict = self.eval(&arr[1])?;
                let mut res = Vec::new();
                for (i, (k, v)) in dict.object()?.iter().enumerate() {
                    if i > 0 {
                        res.push(b'&');
                    }
                    let s = to_s(v)?;
                    res.extend(crate::cgi::escape(k));
                    res.push(b'=');
                    res.extend(crate::cgi::escape(&s));
                }
                Ok(Value::String(res))
            }
            b"nth" => {
                if n != 3 {
                    return Err(Error::general(&[b"merge: Wrong number of arguments"]));
                }
                let subscript = self.eval(&arr[1])?;
                let array = self.eval(&arr[2])?;
                let i = subscript.number()?;
                let list = array.strict_array()?;
                // C++ 版は範囲を確かめずに読んでいた (未定義動作)。Rust 版はエラーにする。
                if i >= 0.0 && (i as usize) < list.len() {
                    Ok(list[i as usize].clone())
                } else {
                    Err(Error::general(&[b"nth: index out of range"]))
                }
            }
            b"prop" => {
                if n != 3 {
                    return Err(Error::general(&[b"prop: Wrong number of arguments"]));
                }
                let object = self.eval(&arr[1])?;
                let key = self.eval(&arr[2])?;
                let map = match &object {
                    Value::Object(m) => m,
                    _ => return Err(Error::general(&[b"prop: ", &object.inspect()?, b" is not an object"])),
                };
                Ok(map.get(key.string()?).cloned().unwrap_or(Value::Null))
            }
            b"removeKey" => {
                if n < 2 {
                    return Err(Error::general(&[b"removeKey: Wrong number of arguments"]));
                }
                let mut object = self.eval(&arr[1])?.object()?.clone();
                for e in &arr[2..] {
                    let key = self.eval(e)?.string()?.to_vec();
                    object.remove(&key);
                }
                Ok(Value::Object(object))
            }
            b"keys" => {
                if n != 2 {
                    return Err(Error::general(&[b"keys: Wrong number of arguments"]));
                }
                let object = self.eval(&arr[1])?;
                Ok(Value::StrictArray(object.object()?.keys().map(|k| Value::String(k.clone())).collect()))
            }
            b"lambda" => Ok(Value::StrictArray(arr.to_vec())),
            b"define" => {
                if !self.h.front_is_generic() {
                    return Err(Error::general(&[b"Cannot change this scope."]));
                }
                let v = self.eval_at(arr, 2)?;
                let name = at(arr, 1)?.string()?.to_vec();
                self.h.set_front(&name, &v)?;
                Ok(Value::Null)
            }
            // 引数は右から評価する
            b"replacePrefix" | b"replaceSuffix" => {
                let c = self.eval_string_at(arr, 3)?;
                let b = self.eval_string_at(arr, 2)?;
                let a = self.eval_string_at(arr, 1)?;
                Ok(Value::String(if name == b"replacePrefix" {
                    crate::strutil::replace_prefix(&a, &b, &c)
                } else {
                    crate::strutil::replace_suffix(&a, &b, &c)
                }))
            }
            b"str" => {
                let mut result = Vec::new();
                for e in &arr[1..] {
                    let v = self.eval(e)?;
                    result.extend(to_s(&v)?);
                }
                Ok(Value::String(result))
            }
            b"evalString" => {
                if n != 2 {
                    return Err(Error::general(&[b"eval: Wrong number of arguments"]));
                }
                let s = self.eval_string_at(arr, 1)?;
                let mut tokens: VecDeque<_> = tokenize(&s)?.into();
                let e = parse(&mut tokens)?;
                self.eval(&e)
            }
            _ => {
                // 変数の値を関数として呼ぶ (Template::writeVariable は常に成功する)
                let f = self.lookup(name)?;
                self.apply(&f, arr)
            }
        }
    }

    /// `Template::evalExpression(const std::string&)`
    pub fn eval_str(&mut self, s: &[u8]) -> Result<Value> {
        let mut tokens: VecDeque<_> = tokenize(s)?.into();
        let exp = parse(&mut tokens)?;
        if let Some(t) = tokens.front() {
            return Err(Error::general(&[b"Unexpected token ", t]));
        }
        self.eval(&exp)
    }

    /// `Template::evalCondition`
    pub fn eval_condition(&mut self, cond: &[u8]) -> Result<bool> {
        Ok(is_truish(&self.eval_str(cond)?))
    }

    /// `Template::getIntVariable`
    pub fn get_int_variable(&mut self, name: &[u8]) -> Result<i32> {
        let name = string_name(name).to_vec();
        match self.lookup(&name)? {
            // C++ 版は int に収まらない値 (と NaN) で未定義動作だった。Rust 版は、どの CPU でも
            // x86 での C++ 版と同じく -2^31 にする ({@loop} は 1 回も回らない)。
            Value::Number(n) => Ok(if n.is_nan() || n.trunc() < i32::MIN as f64 || n.trunc() > i32::MAX as f64 {
                i32::MIN
            } else {
                n as i32
            }),
            Value::String(s) => match stoi(&s) {
                Ok(Some(v)) => Ok(v),
                // 数字で始まっていない。
                Ok(None) => Ok(0),
                // 値が大きすぎる、あるいは小さすぎる。
                Err(()) => Err(Error::general(&[b"stoi"])),
            },
            v => Err(Error::general(&[&name, b" is not a Number. Value: ", &v.inspect()?])),
        }
    }

    /// `Template::getBoolVariable`
    pub fn get_bool_variable(&mut self, name: &[u8]) -> Result<bool> {
        Ok(is_truish(&self.lookup(string_name(name))?))
    }

    /// `Template::getStringVariable`
    pub fn get_string_variable(&mut self, name: &[u8]) -> Result<Vec<u8>> {
        match self.lookup(name)? {
            Value::String(s) => Ok(s),
            v => v.inspect(),
        }
    }

    // ---- テンプレートの読み出し

    /// `readUntil`: `pred` を満たす文字 (バックスラッシュでエスケープされていないもの) まで読む。
    /// 見つからずに終わったら false。
    fn read_until(&mut self, pred: impl Fn(u8) -> bool) -> Result<(bool, Vec<u8>)> {
        let mut var = Vec::new();
        let mut escape_next = false;
        while !self.h.eof()? {
            let c = self.h.read_char()?;
            if !escape_next && c == b'\\' {
                escape_next = true;
                continue;
            }
            if !escape_next && pred(c) {
                return Ok((true, var));
            }
            var.push(c);
            escape_next = false;
        }
        Ok((false, var))
    }

    fn write(&mut self, data: &[u8]) {
        self.out.extend_from_slice(data);
    }

    /// `Template::readFragment`
    pub fn read_fragment(&mut self, out: bool) -> Result<()> {
        let mut frag = Vec::new();
        while !self.h.eof()? {
            let c = self.h.read_char()?;
            if c == b'}' {
                let outer = self.h.current_fragment();
                self.h.set_current_fragment(&frag);
                self.read_template(out)?;
                self.h.set_current_fragment(&outer);
                return Ok(());
            }
            frag.push(c);
        }
        self.h.log_error("Premature end while processing fragment directive");
        Ok(())
    }

    /// `Template::readIf`
    pub fn read_if(&mut self, out: bool) -> Result<()> {
        let mut had_active = false;
        let mut cmd = TMPL_IF;
        while cmd != TMPL_END {
            if cmd == TMPL_ELSE {
                cmd = self.read_template(out && !had_active)?;
            } else if cmd == TMPL_IF || cmd == TMPL_ELSIF {
                let (_, cond) = self.read_until(|c| c == b'}')?;
                if !had_active && self.eval_condition(until_nul(&cond))? {
                    had_active = true;
                    cmd = self.read_template(out)?;
                } else {
                    cmd = self.read_template(false)?;
                }
            }
        }
        Ok(())
    }

    /// `Template::readLoop`
    pub fn read_loop(&mut self, out: bool) -> Result<()> {
        let (ok, var) = self.read_until(|c| c == b'}')?;
        if !ok {
            return Ok(());
        }
        let cnt = self.get_int_variable(&var)?;
        if cnt != 0 {
            let spos = self.h.position()?;
            for _ in 0..cnt.max(0) {
                self.h.seek(spos)?;
                self.read_template(out)?;
            }
        } else {
            self.read_template(false)?;
        }
        Ok(())
    }

    /// `Template::readForeach`
    pub fn read_foreach(&mut self, out: bool) -> Result<()> {
        let (ok, var) = self.read_until(|c| c == b'}')?;
        if !ok {
            return Ok(());
        }
        let mut tokens: VecDeque<_> = tokenize(until_nul(&var))?.into();
        let exp = parse(&mut tokens)?;
        let value = self.eval(&exp)?;
        let coll = match value {
            Value::StrictArray(a) => a,
            v => return Err(Error::general(&[&var, b" is not a strictArray. Value: ", &v.inspect()?])),
        };
        if coll.is_empty() {
            self.read_template(false)?;
        } else {
            self.h.push_scope();
            let start = self.h.position()?;
            for (i, v) in coll.iter().enumerate() {
                self.h.set_front(b"this", v)?;
                self.h.set_front(b"loop.index", &Value::Number(i as f64))?;
                self.h.set_front(b"loop.indexBaseOne", &Value::Number((i + 1) as f64))?;
                self.h.seek(start)?;
                self.read_template(out)?;
            }
            self.h.pop_scope();
        }
        Ok(())
    }

    /// `Template::readLet`
    pub fn read_let(&mut self, out: bool) -> Result<()> {
        let (ok, var) = self.read_until(|c| c == b'}')?;
        if !ok {
            return Ok(());
        }
        let mut tokens: VecDeque<_> = tokenize(until_nul(&var))?.into();
        let spec = parse_let_spec(&mut tokens)?;
        self.h.push_scope();
        for (name, exp) in &spec {
            let v = self.eval(exp)?;
            self.h.set_front(name, &v)?;
        }
        self.read_template(out)?;
        self.h.pop_scope();
        Ok(())
    }

    /// `Template::readCmd`
    pub fn read_cmd(&mut self, out: bool) -> Result<i32> {
        let (ok, cmd) = self.read_until(|c| c == b' ' || c == b'\t' || c == b'}')?;
        if !ok {
            return Ok(TMPL_UNKNOWN);
        }
        Ok(match cmd.as_slice() {
            b"loop" => {
                self.read_loop(out)?;
                TMPL_LOOP
            }
            b"if" => {
                self.read_if(out)?;
                TMPL_IF
            }
            b"elsif" => TMPL_ELSIF,
            b"fragment" => {
                self.read_fragment(out)?;
                TMPL_FRAGMENT
            }
            b"foreach" => {
                self.read_foreach(out)?;
                TMPL_FOREACH
            }
            b"let" => {
                self.read_let(out)?;
                TMPL_LET
            }
            b"end" => TMPL_END,
            b"else" => TMPL_ELSE,
            _ => TMPL_UNKNOWN,
        })
    }

    /// `Template::readVariable_` の、表示する文字列を求めるところまで。表示しないときは `None`。
    pub fn read_variable_value(&mut self, out: bool) -> Result<Option<Vec<u8>>> {
        let (ok, var) = self.read_until(|c| c == b'}')?;
        if !ok {
            return Ok(None);
        }
        let v = self.eval_str(&var)?;
        if !self.in_selected_fragment() || !out {
            return Ok(None); // eval するけど表示しない。
        }
        Ok(Some(to_s(&v)?))
    }

    /// `{$...}` (HTML)、`{\...}` (JavaScript)、`{!...}` (そのまま)
    pub fn read_variable(&mut self, out: bool, filter: fn(&[u8]) -> Vec<u8>) -> Result<()> {
        if let Some(s) = self.read_variable_value(out)? {
            let s = filter(&s);
            self.write(&s);
        }
        Ok(())
    }

    /// `Template::readTemplate`: 1 ブロック分を処理する。EOF か `{@end}` で `TMPL_END`、
    /// `{@else}` で `TMPL_ELSE`、`{@elsif ...}` で (条件式を読む前に) `TMPL_ELSIF` を返す。
    pub fn read_template(&mut self, out: bool) -> Result<i32> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::general(&[b"Template: nesting too deep"]));
        }
        let r = self.read_template_inner(out);
        self.depth -= 1;
        r
    }

    fn read_template_inner(&mut self, out: bool) -> Result<i32> {
        let p = out && self.in_selected_fragment();
        while !self.h.eof()? {
            let c = self.h.read_char()?;
            if c != b'{' {
                if p {
                    self.write(&[c]);
                }
                continue;
            }
            let c = self.h.read_char()?;
            match c {
                b'$' => self.read_variable(out, crate::cgi::escape_html)?,
                b'\\' => self.read_variable(out, crate::cgi::escape_javascript)?,
                b'!' => self.read_variable(out, |s| s.to_vec())?,
                b'@' => {
                    let t = self.read_cmd(out)?;
                    if t == TMPL_END || t == TMPL_ELSE || t == TMPL_ELSIF {
                        return Ok(t);
                    }
                }
                // テンプレートに関係のない波括弧はそのまま表示する
                _ => {
                    if p {
                        self.write(&[b'{', c]);
                    }
                }
            }
        }
        Ok(TMPL_END)
    }
}

#[cfg(test)]
mod tests;
