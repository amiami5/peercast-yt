//! 正規表現 (core/common/regexp.cpp の `Regexp`。C++ 版は `std::regex` の ECMAScript の文法)。
//!
//! バイト列を 1 バイトずつ文字として扱う (`std::regex` を `char` で使うのと同じ)。バックトラックで
//! 照合するので、選択 (`|`) と量指定子の優先順位は ECMAScript と同じ。照合は明示的なスタックで行い、
//! 再帰しない (長い入力でもスタックを使い果たさない)。
//!
//! 対応するもの: `^` `$` `\b` `\B`、`.`、文字クラス (`[...]`、`[^...]`、範囲、`\d` `\w` `\s` と大文字)、
//! グループ (`(...)`、`(?:...)`、先読み `(?=...)` `(?!...)`)、後方参照 (`\1` など)、量指定子
//! (`*` `+` `?` `{n}` `{n,}` `{n,m}` と、それぞれの最短一致 `?`)、エスケープ (`\t` `\n` `\r` `\v`
//! `\f` `\0` `\xHH` `\cX` と記号)。

use std::fmt;

/// 正規表現の誤り (`std::regex_error`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexError(pub &'static str);

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug, Clone)]
enum Node {
    Empty,
    Byte(u8),
    Any,
    Class(Box<[bool; 256]>),
    Start,
    End,
    WordBoundary(bool),
    Group(Box<Node>, Option<usize>),
    Look(Box<Node>, bool),
    BackRef(usize),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool },
}

/// 量指定子の回数の上限 (これを超えると展開が大きくなりすぎる)
const MAX_REPEAT: u32 = 1000;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    groups: usize,
    /// 開いているグループの番号
    open: Vec<usize>,
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn class_of(f: impl Fn(u8) -> bool) -> [bool; 256] {
    let mut t = [false; 256];
    for (c, v) in t.iter_mut().enumerate() {
        *v = f(c as u8);
    }
    t
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alternation(&mut self) -> Result<Node, RegexError> {
        let mut alts = vec![self.concat()?];
        while self.eat(b'|') {
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Alt(alts) })
    }

    fn concat(&mut self) -> Result<Node, RegexError> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'|' || c == b')' {
                break;
            }
            let mut atom = self.atom()?;
            // libstdc++ は量指定子を重ねたもの (`a**`、`a*+`) も受け付ける
            while matches!(self.peek(), Some(b'*' | b'+' | b'?' | b'{')) {
                atom = self.quantifier(atom)?;
            }
            items.push(atom);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.i;
        let mut v: u32 = 0;
        while let Some(c) = self.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            v = v.saturating_mul(10).saturating_add((c - b'0') as u32);
            self.i += 1;
        }
        if self.i == start {
            None
        } else {
            Some(v)
        }
    }

    fn quantifier(&mut self, atom: Node) -> Result<Node, RegexError> {
        let (min, max) = match self.peek() {
            Some(b'*') => {
                self.i += 1;
                (0, None)
            }
            Some(b'+') => {
                self.i += 1;
                (1, None)
            }
            Some(b'?') => {
                self.i += 1;
                (0, Some(1))
            }
            Some(b'{') => {
                self.i += 1;
                let min = self.number().ok_or(RegexError("error_badbrace"))?;
                let max = if self.eat(b',') {
                    if self.peek() == Some(b'}') {
                        None
                    } else {
                        Some(self.number().ok_or(RegexError("error_badbrace"))?)
                    }
                } else {
                    Some(min)
                };
                if !self.eat(b'}') {
                    return Err(RegexError("error_brace"));
                }
                if let Some(m) = max {
                    if m < min {
                        return Err(RegexError("error_badbrace"));
                    }
                }
                if min > MAX_REPEAT || max.map_or(false, |m| m > MAX_REPEAT) {
                    return Err(RegexError("error_complexity"));
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        if matches!(atom, Node::Start | Node::End | Node::WordBoundary(_) | Node::Look(..)) {
            return Err(RegexError("error_badrepeat"));
        }
        let greedy = !self.eat(b'?');
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    fn atom(&mut self) -> Result<Node, RegexError> {
        let c = self.peek().ok_or(RegexError("error_badrepeat"))?;
        self.i += 1;
        match c {
            b'^' => Ok(Node::Start),
            b'$' => Ok(Node::End),
            b'.' => Ok(Node::Any),
            b'*' | b'+' | b'?' => Err(RegexError("error_badrepeat")),
            b'{' => Err(RegexError("error_badrepeat")),
            b'(' => {
                let node = if self.eat(b'?') {
                    match self.peek() {
                        Some(b':') => {
                            self.i += 1;
                            let n = self.alternation()?;
                            Node::Group(Box::new(n), None)
                        }
                        Some(b'=') | Some(b'!') => {
                            let positive = self.peek() == Some(b'=');
                            self.i += 1;
                            let n = self.alternation()?;
                            Node::Look(Box::new(n), positive)
                        }
                        _ => return Err(RegexError("error_paren")),
                    }
                } else {
                    self.groups += 1;
                    let idx = self.groups;
                    self.open.push(idx);
                    let n = self.alternation()?;
                    self.open.pop();
                    Node::Group(Box::new(n), Some(idx))
                };
                if !self.eat(b')') {
                    return Err(RegexError("error_paren"));
                }
                Ok(node)
            }
            b')' => Err(RegexError("error_paren")),
            b'[' => self.bracket(),
            b'\\' => self.escape(false).map(|e| match e {
                Esc::Byte(b) => Node::Byte(b),
                Esc::Class(t) => Node::Class(Box::new(t)),
                Esc::Boundary(b) => Node::WordBoundary(b),
                Esc::BackRef(n) => Node::BackRef(n),
            }),
            _ => Ok(Node::Byte(c)),
        }
    }

    /// `\` の次から読む。`in_class` なら文字クラスの中 (`\b` は後退文字)。
    fn escape(&mut self, in_class: bool) -> Result<Esc, RegexError> {
        let c = self.peek().ok_or(RegexError("error_escape"))?;
        self.i += 1;
        Ok(match c {
            b'd' => Esc::Class(class_of(|c| c.is_ascii_digit())),
            b'D' => Esc::Class(class_of(|c| !c.is_ascii_digit())),
            b'w' => Esc::Class(class_of(is_word)),
            b'W' => Esc::Class(class_of(|c| !is_word(c))),
            b's' => Esc::Class(class_of(is_space)),
            b'S' => Esc::Class(class_of(|c| !is_space(c))),
            b't' => Esc::Byte(b'\t'),
            b'n' => Esc::Byte(b'\n'),
            b'r' => Esc::Byte(b'\r'),
            b'v' => Esc::Byte(0x0b),
            b'f' => Esc::Byte(0x0c),
            b'0' => Esc::Byte(0),
            b'b' if in_class => Esc::Byte(0x08),
            b'b' => Esc::Boundary(true),
            b'B' if !in_class => Esc::Boundary(false),
            b'c' => {
                let l = self.peek().filter(|l| l.is_ascii_alphabetic()).ok_or(RegexError("error_escape"))?;
                self.i += 1;
                Esc::Byte(l % 32)
            }
            b'x' => {
                let h = self.s.get(self.i..self.i + 2).ok_or(RegexError("error_escape"))?;
                let v = hex(h[0]).zip(hex(h[1])).ok_or(RegexError("error_escape"))?;
                self.i += 2;
                Esc::Byte(v.0 << 4 | v.1)
            }
            b'1'..=b'9' if !in_class => {
                let mut n = (c - b'0') as usize;
                while let Some(d) = self.peek().filter(|d| d.is_ascii_digit()) {
                    n = n.saturating_mul(10).saturating_add((d - b'0') as usize);
                    self.i += 1;
                }
                // libstdc++ と同じく、それまでに閉じたグループだけを参照できる
                if n > self.groups || self.open.contains(&n) {
                    return Err(RegexError("error_backref"));
                }
                Esc::BackRef(n)
            }
            c if c.is_ascii_alphanumeric() => return Err(RegexError("error_escape")),
            c => Esc::Byte(c),
        })
    }

    fn bracket(&mut self) -> Result<Node, RegexError> {
        let negate = self.eat(b'^');
        let mut set = [false; 256];
        loop {
            // ECMAScript では、先頭の ] も閉じ括弧 ([] は何にも一致せず、[^] は何にでも一致する)
            let c = self.peek().ok_or(RegexError("error_brack"))?;
            if c == b']' {
                self.i += 1;
                break;
            }
            let lo = self.class_atom()?;
            // 範囲
            if self.peek() == Some(b'-') && self.s.get(self.i + 1).map_or(false, |&n| n != b']') {
                self.i += 1;
                match (lo, self.class_atom()?) {
                    (ClassAtom::Byte(a), ClassAtom::Byte(b)) => {
                        if b < a {
                            return Err(RegexError("error_range"));
                        }
                        for v in a..=b {
                            set[v as usize] = true;
                        }
                        continue;
                    }
                    // \d などを範囲の端にはできない
                    _ => return Err(RegexError("error_range")),
                }
            }
            match lo {
                ClassAtom::Byte(b) => set[b as usize] = true,
                ClassAtom::Set(t) => {
                    for (d, s) in set.iter_mut().zip(t.iter()) {
                        *d |= *s;
                    }
                }
            }
        }
        if negate {
            for v in set.iter_mut() {
                *v = !*v;
            }
        }
        Ok(Node::Class(Box::new(set)))
    }

    fn class_atom(&mut self) -> Result<ClassAtom, RegexError> {
        let c = self.peek().ok_or(RegexError("error_brack"))?;
        self.i += 1;
        if c == b'[' {
            if let Some(kind @ (b':' | b'.' | b'=')) = self.peek() {
                // [:name:]、[.c.]、[=c=] (POSIX の書き方。libstdc++ は ECMAScript でも受け付ける)
                self.i += 1;
                let start = self.i;
                let end = loop {
                    match (self.s.get(self.i), self.s.get(self.i + 1)) {
                        (Some(&a), Some(&b']')) if a == kind => break self.i,
                        (Some(_), _) => self.i += 1,
                        (None, _) => return Err(RegexError(if kind == b':' { "error_ctype" } else { "error_collate" })),
                    }
                };
                self.i = end + 2;
                let name = &self.s[start..end];
                return match kind {
                    b':' => class_name(name).map(|t| ClassAtom::Set(Box::new(t))).ok_or(RegexError("error_ctype")),
                    _ => match name {
                        [c] => Ok(ClassAtom::Byte(*c)),
                        _ => Err(RegexError("error_collate")),
                    },
                };
            }
        }
        if c == b'\\' {
            match self.escape(true)? {
                Esc::Byte(b) => Ok(ClassAtom::Byte(b)),
                Esc::Class(t) => Ok(ClassAtom::Set(Box::new(t))),
                _ => Err(RegexError("error_escape")),
            }
        } else {
            Ok(ClassAtom::Byte(c))
        }
    }
}

/// `[:name:]` の文字の集まり (C のロケールの `<cctype>`)
fn class_name(name: &[u8]) -> Option<[bool; 256]> {
    Some(match name {
        b"alnum" => class_of(|c| c.is_ascii_alphanumeric()),
        b"alpha" => class_of(|c| c.is_ascii_alphabetic()),
        b"blank" => class_of(|c| c == b' ' || c == b'\t'),
        b"cntrl" => class_of(|c| c < 0x20 || c == 0x7f),
        b"digit" | b"d" => class_of(|c| c.is_ascii_digit()),
        b"graph" => class_of(|c| c.is_ascii_graphic()),
        b"lower" => class_of(|c| c.is_ascii_lowercase()),
        b"print" => class_of(|c| c.is_ascii_graphic() || c == b' '),
        b"punct" => class_of(|c| c.is_ascii_punctuation()),
        b"space" | b"s" => class_of(is_space),
        b"upper" => class_of(|c| c.is_ascii_uppercase()),
        b"xdigit" => class_of(|c| c.is_ascii_hexdigit()),
        b"w" => class_of(is_word),
        _ => return None,
    })
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|v| v as u8)
}

enum Esc {
    Byte(u8),
    Class([bool; 256]),
    Boundary(bool),
    BackRef(usize),
}

enum ClassAtom {
    Byte(u8),
    Set(Box<[bool; 256]>),
}

// ---------------------------------------------------------------- 命令列

#[derive(Debug, Clone)]
enum Inst {
    Byte(u8),
    Any,
    Class(Box<[bool; 256]>),
    Start,
    End,
    WordBoundary(bool),
    /// 捕獲の位置 (スロット) を記録する
    Save(usize),
    /// 先に x、だめなら y
    Split(usize, usize),
    Jmp(usize),
    /// 量指定子の 1 回分に入る。libstdc++ (`_M_rep_once_more`) と同じく、同じ位置からは続けて 2 回まで
    /// (空に一致する繰り返しが終わるように)
    Rep(usize),
    BackRef(usize),
    /// 先読み。中身は別の命令列
    Look(usize, bool),
    Match,
}

/// コンパイルした正規表現
#[derive(Debug, Clone)]
pub struct Regex {
    progs: Vec<Vec<Inst>>,
    ngroups: usize,
    nmarks: usize,
}

struct Compiler {
    progs: Vec<Vec<Inst>>,
    nmarks: usize,
}

impl Compiler {
    fn emit(&mut self, p: usize, i: Inst) -> usize {
        self.progs[p].push(i);
        self.progs[p].len() - 1
    }

    fn compile(&mut self, p: usize, n: &Node) {
        match n {
            Node::Empty => {}
            Node::Byte(b) => {
                self.emit(p, Inst::Byte(*b));
            }
            Node::Any => {
                self.emit(p, Inst::Any);
            }
            Node::Class(t) => {
                self.emit(p, Inst::Class(t.clone()));
            }
            Node::Start => {
                self.emit(p, Inst::Start);
            }
            Node::End => {
                self.emit(p, Inst::End);
            }
            Node::WordBoundary(b) => {
                self.emit(p, Inst::WordBoundary(*b));
            }
            Node::BackRef(k) => {
                self.emit(p, Inst::BackRef(*k));
            }
            Node::Group(inner, idx) => {
                if let Some(k) = idx {
                    self.emit(p, Inst::Save(2 * k));
                    self.compile(p, inner);
                    self.emit(p, Inst::Save(2 * k + 1));
                } else {
                    self.compile(p, inner);
                }
            }
            Node::Look(inner, positive) => {
                let sub = self.progs.len();
                self.progs.push(Vec::new());
                self.compile(sub, inner);
                self.emit(sub, Inst::Match);
                self.emit(p, Inst::Look(sub, *positive));
            }
            Node::Concat(items) => {
                for i in items {
                    self.compile(p, i);
                }
            }
            Node::Alt(alts) => {
                let mut jumps = Vec::new();
                for (k, a) in alts.iter().enumerate() {
                    if k + 1 < alts.len() {
                        let split = self.emit(p, Inst::Split(0, 0));
                        self.compile(p, a);
                        jumps.push(self.emit(p, Inst::Jmp(0)));
                        let next = self.progs[p].len();
                        self.progs[p][split] = Inst::Split(split + 1, next);
                    } else {
                        self.compile(p, a);
                    }
                }
                let end = self.progs[p].len();
                for j in jumps {
                    self.progs[p][j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max, greedy } => {
                for _ in 0..*min {
                    self.compile(p, node);
                }
                match max {
                    None => {
                        // L: split body, out / body: mark; node; progress; jmp L
                        let mark = self.nmarks;
                        self.nmarks += 1;
                        let split = self.emit(p, Inst::Split(0, 0));
                        self.emit(p, Inst::Rep(mark));
                        self.compile(p, node);
                        self.emit(p, Inst::Jmp(split));
                        let out = self.progs[p].len();
                        self.progs[p][split] =
                            if *greedy { Inst::Split(split + 1, out) } else { Inst::Split(out, split + 1) };
                    }
                    Some(m) => {
                        let mut splits = Vec::new();
                        for _ in *min..*m {
                            let mark = self.nmarks;
                            self.nmarks += 1;
                            let split = self.emit(p, Inst::Split(0, 0));
                            splits.push(split);
                            self.emit(p, Inst::Rep(mark));
                            self.compile(p, node);
                        }
                        let out = self.progs[p].len();
                        for s in splits {
                            self.progs[p][s] = if *greedy { Inst::Split(s + 1, out) } else { Inst::Split(out, s + 1) };
                        }
                    }
                }
            }
        }
    }
}

/// 照合の途中の状態を戻すための記録
enum Undo {
    /// ここから pc と位置で続きを試す
    Branch(usize, usize),
    /// スロットを元の値に戻す
    Slot(usize, Option<usize>),
    Mark(usize, (usize, u8)),
}

impl Regex {
    /// `std::regex` のコンストラクター。文法の誤りは `RegexError`。
    pub fn new(pattern: &[u8]) -> Result<Regex, RegexError> {
        let mut p = Parser { s: pattern, i: 0, groups: 0, open: Vec::new() };
        let node = p.alternation()?;
        if p.i != pattern.len() {
            return Err(RegexError("error_paren"));
        }
        let ngroups = p.groups;
        check_backrefs(&node, ngroups)?;
        let mut c = Compiler { progs: vec![Vec::new()], nmarks: 0 };
        c.compile(0, &Node::Group(Box::new(node), Some(0)));
        c.emit(0, Inst::Match);
        Ok(Regex { progs: c.progs, ngroups, nmarks: c.nmarks })
    }

    /// `std::regex_search` と同じく、最初に見つかった一致の各グループ (0 は全体)。一致しなければ
    /// `None`。一致しなかったグループは `None`。
    pub fn search(&self, subject: &[u8]) -> Option<Vec<Option<(usize, usize)>>> {
        for start in 0..=subject.len() {
            let mut slots = vec![None; 2 * (self.ngroups + 1)];
            let mut marks = vec![(0, 0); self.nmarks];
            if self.run(0, subject, start, &mut slots, &mut marks).is_some() {
                return Some(
                    (0..=self.ngroups)
                        .map(|k| match (slots[2 * k], slots[2 * k + 1]) {
                            (Some(a), Some(b)) => Some((a, b)),
                            _ => None,
                        })
                        .collect(),
                );
            }
        }
        None
    }

    /// `Regexp::matches`
    pub fn is_match(&self, subject: &[u8]) -> bool {
        self.search(subject).is_some()
    }

    /// `Regexp::exec`: 一致すれば各グループの文字列 (一致しなかったグループは空)、しなければ空
    pub fn exec(&self, subject: &[u8]) -> Vec<Vec<u8>> {
        match self.search(subject) {
            Some(groups) => groups.into_iter().map(|g| g.map_or(Vec::new(), |(a, b)| subject[a..b].to_vec())).collect(),
            None => Vec::new(),
        }
    }

    /// 命令列 `prog` を位置 `pos` から照合し、一致すれば終わりの位置
    fn run(
        &self,
        prog: usize,
        s: &[u8],
        pos: usize,
        slots: &mut [Option<usize>],
        marks: &mut [(usize, u8)],
    ) -> Option<usize> {
        let code = &self.progs[prog];
        let mut stack: Vec<Undo> = Vec::new();
        let mut pc = 0;
        let mut pos = pos;
        loop {
            let ok = match &code[pc] {
                Inst::Byte(b) => {
                    if s.get(pos) == Some(b) {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Any => match s.get(pos) {
                    Some(&c) if c != b'\n' && c != b'\r' => {
                        pos += 1;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Class(t) => match s.get(pos) {
                    Some(&c) if t[c as usize] => {
                        pos += 1;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Start => {
                    pc += 1;
                    pos == 0
                }
                Inst::End => {
                    pc += 1;
                    pos == s.len()
                }
                Inst::WordBoundary(want) => {
                    let a = pos > 0 && is_word(s[pos - 1]);
                    let b = pos < s.len() && is_word(s[pos]);
                    pc += 1;
                    (a != b) == *want
                }
                Inst::Save(k) => {
                    stack.push(Undo::Slot(*k, slots[*k]));
                    slots[*k] = Some(pos);
                    pc += 1;
                    true
                }
                Inst::Split(x, y) => {
                    stack.push(Undo::Branch(*y, pos));
                    pc = *x;
                    true
                }
                Inst::Jmp(x) => {
                    pc = *x;
                    true
                }
                Inst::Rep(k) => {
                    let (at, count) = marks[*k];
                    pc += 1;
                    if count == 0 || at != pos {
                        stack.push(Undo::Mark(*k, marks[*k]));
                        marks[*k] = (pos, 1);
                        true
                    } else if count < 2 {
                        stack.push(Undo::Mark(*k, marks[*k]));
                        marks[*k].1 += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::BackRef(k) => {
                    match (slots.get(2 * k).copied().flatten(), slots.get(2 * k + 1).copied().flatten()) {
                        (Some(a), Some(b)) => {
                            let sub = &s[a..b];
                            if s.get(pos..pos + sub.len()) == Some(sub) {
                                pos += sub.len();
                                pc += 1;
                                true
                            } else {
                                false
                            }
                        }
                        // 一致しなかったグループへの参照は失敗する (libstdc++。ECMAScript の仕様では
                        // 空文字列に一致する)
                        _ => false,
                    }
                }
                Inst::Look(sub, positive) => {
                    let mut tmp_slots = slots.to_vec();
                    let mut tmp_marks = marks.to_vec();
                    let m = self.run(*sub, s, pos, &mut tmp_slots, &mut tmp_marks).is_some();
                    if m && *positive {
                        // 肯定の先読みの中の捕獲は残る
                        for (k, v) in tmp_slots.into_iter().enumerate() {
                            if slots[k] != v {
                                stack.push(Undo::Slot(k, slots[k]));
                                slots[k] = v;
                            }
                        }
                    }
                    pc += 1;
                    m == *positive
                }
                Inst::Match => return Some(pos),
            };
            if !ok {
                // 戻る
                loop {
                    match stack.pop() {
                        None => return None,
                        Some(Undo::Slot(k, v)) => slots[k] = v,
                        Some(Undo::Mark(k, v)) => marks[k] = v,
                        Some(Undo::Branch(p, at)) => {
                            pc = p;
                            pos = at;
                            break;
                        }
                    }
                }
            }
        }
    }
}

fn check_backrefs(n: &Node, ngroups: usize) -> Result<(), RegexError> {
    match n {
        Node::BackRef(k) if *k > ngroups => Err(RegexError("error_backref")),
        Node::Group(i, _) | Node::Look(i, _) => check_backrefs(i, ngroups),
        Node::Repeat { node, .. } => check_backrefs(node, ngroups),
        Node::Concat(v) | Node::Alt(v) => v.iter().try_for_each(|i| check_backrefs(i, ngroups)),
        _ => Ok(()),
    }
}

/// `Regexp::escape`
pub fn escape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &c in s {
        if matches!(c, b'[' | b']' | b'\\' | b'^' | b'$' | b'.' | b'|' | b'?' | b'*' | b'+' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Regex::new(p.as_bytes()).unwrap().is_match(s.as_bytes())
    }

    fn ex(p: &str, s: &str) -> Vec<String> {
        Regex::new(p.as_bytes()).unwrap().exec(s.as_bytes()).into_iter().map(|v| String::from_utf8(v).unwrap()).collect()
    }

    #[test]
    fn basics() {
        assert!(m("/index.html$", "/html/en/index.html"));
        assert!(!m("/index.html$", "/html/en/index.html?x"));
        assert!(m("^\\d+\\.\\d+\\.\\d+\\.\\d+$", "127.0.0.1"));
        assert!(!m("^\\d+\\.\\d+\\.\\d+\\.\\d+$", "127.0.0.1:80"));
        assert!(m("a|b", "xb"));
        assert!(m("^(ab)*$", "ababab"));
        assert!(!m("^(ab)*$", "ababa"));
        assert!(m("^a{2,3}$", "aaa"));
        assert!(!m("^a{2,3}$", "aaaa"));
        assert!(m("^a{2,}$", "aaaaa"));
        assert!(m("^[A-z]{3}$", "a_Z"));
        assert!(m("^[^/]*$", "abc"));
        assert!(!m("^[^/]*$", "a/c"));
        assert!(m("\\bfoo\\b", "a foo b"));
        assert!(!m("\\bfoo\\b", "afoob"));
        assert!(m("^(a*)*$", "aaa"));
        assert!(m("^(?:a|ab)(?:c|bcd)(?:d*)$", "abcd"));
        assert!(m("^(\\w)\\1$", "aa"));
        assert!(!m("^(\\w)\\1$", "ab"));
        assert!(m("^a(?=b)", "ab"));
        assert!(!m("^a(?!b)", "ab"));
        assert!(!m("^.$", "\n"));
        // 量指定子を重ねたもの (libstdc++ と同じく受け付ける)
        assert!(m("^a**$", "aaa"));
        assert!(m("^a{2}{2}$", "aaaa"));
        assert!(!m("^a{2}{2}$", "aaa"));
    }

    #[test]
    fn groups() {
        assert_eq!(ex("^\\s*\\[(\\w+)\\]\\s*$", " [Server] "), vec![" [Server] ", "Server"]);
        assert_eq!(ex("\\s*(\\w+)\\s*=\\s*(.*)$", "serverPort = 7144"), vec!["serverPort = 7144", "serverPort", "7144"]);
        assert_eq!(ex("(a)|(b)", "b"), vec!["b", "", "b"]);
        assert!(ex("x", "y").is_empty());
        // 最短一致
        assert_eq!(ex("<(.+?)>", "<a><b>"), vec!["<a>", "a"]);
        assert_eq!(ex("<(.+)>", "<a><b>"), vec!["<a><b>", "a><b"]);
        assert_eq!(ex("/[^/]*$", "http://a/b/c.txt"), vec!["/c.txt"]);
    }

    #[test]
    fn errors() {
        for p in ["(", ")", "[a", "*", "a{", "a{2,1}", "[z-a]", "\\q", "(?<x)", "\\2(a)"] {
            assert!(Regex::new(p.as_bytes()).is_err(), "{}", p);
        }
    }

    #[test]
    fn ipv6_pattern() {
        const IPV6: &str = "^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|[fF][eE]80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::([fF][fF][fF][fF](:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))$";
        let r = Regex::new(IPV6.as_bytes()).unwrap();
        for ok in ["::1", "::", "2001:db8::1", "fe80::1%eth0", "::ffff:1.2.3.4", "1:2:3:4:5:6:7:8"] {
            assert!(r.is_match(ok.as_bytes()), "{}", ok);
        }
        for ng in ["1.2.3.4", "localhost", ":::", "1:2:3:4:5:6:7:8:9"] {
            assert!(!r.is_match(ng.as_bytes()), "{}", ng);
        }
    }

    /// 乱数の正規表現と入力でパニックせず、終わる
    #[test]
    fn fuzz() {
        let mut x: u32 = 12345;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        const ALPHA: &[u8] = b"ab()[]^$.*+?{}|\\dw1,-:=!";
        for _ in 0..50_000 {
            let n = (rnd() % 12) as usize;
            let p: Vec<u8> = (0..n).map(|_| ALPHA[(rnd() as usize) % ALPHA.len()]).collect();
            if let Ok(r) = Regex::new(&p) {
                let n = (rnd() % 12) as usize;
                let s: Vec<u8> = (0..n).map(|_| b"ab1-"[(rnd() % 4) as usize]).collect();
                let _ = r.exec(&s);
            }
        }
    }
}
