//! nlohmann::json 3.7.3 の `json::parse` (字句解析器 `lexer` と構文解析器 `parser`、DOM を作る
//! `json_sax_dom_parser`) の移植。受け付ける入力、数の種類、例外の文言を同じにするため、
//! 読み進め方 (`get` / `unget`、位置の数え方、`token_string` のため方) もそのまま写している。
//!
//! 構文解析器は入れ子を再帰でなくスタックで扱う (nlohmann と同じ)。

use super::{Object, Value};

/// `json::parse` が投げる例外。`what()` の文字列を持つ。
#[derive(Clone, Debug, PartialEq)]
pub enum ParseError {
    /// `parse_error` (101)
    Parse(Vec<u8>),
    /// `out_of_range` (406、浮動小数点数が有限でない)。`parse_error` ではない。
    OutOfRange(Vec<u8>),
}

impl ParseError {
    pub fn what(&self) -> &[u8] {
        match self {
            ParseError::Parse(w) | ParseError::OutOfRange(w) => w,
        }
    }
}

const EOF: i32 = -1;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Uninitialized,
    LiteralTrue,
    LiteralFalse,
    LiteralNull,
    ValueString,
    ValueUnsigned,
    ValueInteger,
    ValueFloat,
    BeginArray,
    BeginObject,
    EndArray,
    EndObject,
    NameSeparator,
    ValueSeparator,
    ParseError,
    EndOfInput,
    LiteralOrValue,
}

fn token_type_name(t: Tok) -> &'static str {
    match t {
        Tok::Uninitialized => "<uninitialized>",
        Tok::LiteralTrue => "true literal",
        Tok::LiteralFalse => "false literal",
        Tok::LiteralNull => "null literal",
        Tok::ValueString => "string literal",
        Tok::ValueUnsigned | Tok::ValueInteger | Tok::ValueFloat => "number literal",
        Tok::BeginArray => "'['",
        Tok::BeginObject => "'{'",
        Tok::EndArray => "']'",
        Tok::EndObject => "'}'",
        Tok::NameSeparator => "':'",
        Tok::ValueSeparator => "','",
        Tok::ParseError => "<parse error>",
        Tok::EndOfInput => "end of input",
        Tok::LiteralOrValue => "'[', '{', or a literal",
    }
}

#[derive(Clone, Copy, Default)]
struct Position {
    chars_read_total: usize,
    chars_read_current_line: usize,
    lines_read: usize,
}

struct Lexer<'a> {
    input: &'a [u8],
    next: usize,
    current: i32,
    next_unget: bool,
    position: Position,
    token_string: Vec<u8>,
    token_buffer: Vec<u8>,
    error_message: &'static str,
    value_integer: i64,
    value_unsigned: u64,
    value_float: f64,
}

/// 制御文字 (U+0000〜U+001F) が文字列の中にあったときの文言。
const CONTROL_MESSAGES: [&str; 32] = [
    "invalid string: control character U+0000 (NUL) must be escaped to \\u0000",
    "invalid string: control character U+0001 (SOH) must be escaped to \\u0001",
    "invalid string: control character U+0002 (STX) must be escaped to \\u0002",
    "invalid string: control character U+0003 (ETX) must be escaped to \\u0003",
    "invalid string: control character U+0004 (EOT) must be escaped to \\u0004",
    "invalid string: control character U+0005 (ENQ) must be escaped to \\u0005",
    "invalid string: control character U+0006 (ACK) must be escaped to \\u0006",
    "invalid string: control character U+0007 (BEL) must be escaped to \\u0007",
    "invalid string: control character U+0008 (BS) must be escaped to \\u0008 or \\b",
    "invalid string: control character U+0009 (HT) must be escaped to \\u0009 or \\t",
    "invalid string: control character U+000A (LF) must be escaped to \\u000A or \\n",
    "invalid string: control character U+000B (VT) must be escaped to \\u000B",
    "invalid string: control character U+000C (FF) must be escaped to \\u000C or \\f",
    "invalid string: control character U+000D (CR) must be escaped to \\u000D or \\r",
    "invalid string: control character U+000E (SO) must be escaped to \\u000E",
    "invalid string: control character U+000F (SI) must be escaped to \\u000F",
    "invalid string: control character U+0010 (DLE) must be escaped to \\u0010",
    "invalid string: control character U+0011 (DC1) must be escaped to \\u0011",
    "invalid string: control character U+0012 (DC2) must be escaped to \\u0012",
    "invalid string: control character U+0013 (DC3) must be escaped to \\u0013",
    "invalid string: control character U+0014 (DC4) must be escaped to \\u0014",
    "invalid string: control character U+0015 (NAK) must be escaped to \\u0015",
    "invalid string: control character U+0016 (SYN) must be escaped to \\u0016",
    "invalid string: control character U+0017 (ETB) must be escaped to \\u0017",
    "invalid string: control character U+0018 (CAN) must be escaped to \\u0018",
    "invalid string: control character U+0019 (EM) must be escaped to \\u0019",
    "invalid string: control character U+001A (SUB) must be escaped to \\u001A",
    "invalid string: control character U+001B (ESC) must be escaped to \\u001B",
    "invalid string: control character U+001C (FS) must be escaped to \\u001C",
    "invalid string: control character U+001D (GS) must be escaped to \\u001D",
    "invalid string: control character U+001E (RS) must be escaped to \\u001E",
    "invalid string: control character U+001F (US) must be escaped to \\u001F",
];

const ILL_FORMED: &str = "invalid string: ill-formed UTF-8 byte";
const BAD_HEX: &str = "invalid string: '\\u' must be followed by 4 hex digits";
const BAD_SURROGATE: &str = "invalid string: surrogate U+DC00..U+DFFF must be followed by U+DC00..U+DFFF";

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8]) -> Lexer<'a> {
        Lexer {
            input,
            next: 0,
            current: EOF,
            next_unget: false,
            position: Position::default(),
            token_string: Vec::new(),
            token_buffer: Vec::new(),
            error_message: "",
            value_integer: 0,
            value_unsigned: 0,
            value_float: 0.0,
        }
    }

    fn get(&mut self) -> i32 {
        self.position.chars_read_total += 1;
        self.position.chars_read_current_line += 1;
        if self.next_unget {
            self.next_unget = false;
        } else if self.next < self.input.len() {
            self.current = self.input[self.next] as i32;
            self.next += 1;
        } else {
            self.current = EOF;
        }
        if self.current != EOF {
            self.token_string.push(self.current as u8);
        }
        if self.current == b'\n' as i32 {
            self.position.lines_read += 1;
            self.position.chars_read_current_line = 0;
        }
        self.current
    }

    fn unget(&mut self) {
        self.next_unget = true;
        self.position.chars_read_total -= 1;
        if self.position.chars_read_current_line == 0 {
            if self.position.lines_read > 0 {
                self.position.lines_read -= 1;
            }
        } else {
            self.position.chars_read_current_line -= 1;
        }
        if self.current != EOF {
            self.token_string.pop();
        }
    }

    fn add(&mut self, c: i32) {
        self.token_buffer.push(c as u8);
    }

    fn reset(&mut self) {
        self.token_buffer.clear();
        self.token_string.clear();
        self.token_string.push(self.current as u8);
    }

    /// `last read: '...'` に出す文字列。制御文字は `<U+XXXX>` にする。
    fn get_token_string(&self) -> Vec<u8> {
        let mut r = Vec::new();
        for &c in &self.token_string {
            if c <= 0x1f {
                r.extend_from_slice(format!("<U+{:04X}>", c).as_bytes());
            } else {
                r.push(c);
            }
        }
        r
    }

    fn get_codepoint(&mut self) -> i32 {
        let mut codepoint = 0;
        for factor in [12u32, 8, 4, 0] {
            let c = self.get();
            let d = if (b'0' as i32..=b'9' as i32).contains(&c) {
                c - 0x30
            } else if (b'A' as i32..=b'F' as i32).contains(&c) {
                c - 0x37
            } else if (b'a' as i32..=b'f' as i32).contains(&c) {
                c - 0x57
            } else {
                return -1;
            };
            codepoint += d << factor;
        }
        codepoint
    }

    fn next_byte_in_range(&mut self, ranges: &[i32]) -> bool {
        self.add(self.current);
        for pair in ranges.chunks(2) {
            self.get();
            if pair[0] <= self.current && self.current <= pair[1] {
                self.add(self.current);
            } else {
                self.error_message = ILL_FORMED;
                return false;
            }
        }
        true
    }

    fn scan_string(&mut self) -> Tok {
        self.reset();
        loop {
            let c = self.get();
            match c {
                EOF => {
                    self.error_message = "invalid string: missing closing quote";
                    return Tok::ParseError;
                }
                0x22 => return Tok::ValueString,
                0x5c => {
                    match self.get() {
                        0x22 => self.add(0x22),
                        0x5c => self.add(0x5c),
                        0x2f => self.add(0x2f),
                        0x62 => self.add(0x08),
                        0x66 => self.add(0x0c),
                        0x6e => self.add(0x0a),
                        0x72 => self.add(0x0d),
                        0x74 => self.add(0x09),
                        0x75 => {
                            let codepoint1 = self.get_codepoint();
                            let mut codepoint = codepoint1;
                            if codepoint1 == -1 {
                                self.error_message = BAD_HEX;
                                return Tok::ParseError;
                            }
                            if (0xd800..=0xdbff).contains(&codepoint1) {
                                if self.get() == 0x5c && self.get() == 0x75 {
                                    let codepoint2 = self.get_codepoint();
                                    if codepoint2 == -1 {
                                        self.error_message = BAD_HEX;
                                        return Tok::ParseError;
                                    }
                                    if (0xdc00..=0xdfff).contains(&codepoint2) {
                                        codepoint = (((codepoint1 as u32) << 10) + codepoint2 as u32 - 0x35fdc00) as i32;
                                    } else {
                                        self.error_message = BAD_SURROGATE;
                                        return Tok::ParseError;
                                    }
                                } else {
                                    self.error_message = BAD_SURROGATE;
                                    return Tok::ParseError;
                                }
                            } else if (0xdc00..=0xdfff).contains(&codepoint1) {
                                self.error_message = "invalid string: surrogate U+DC00..U+DFFF must follow U+D800..U+DBFF";
                                return Tok::ParseError;
                            }
                            let cp = codepoint as u32;
                            if cp < 0x80 {
                                self.add(cp as i32);
                            } else if cp <= 0x7ff {
                                self.add((0xc0 | (cp >> 6)) as i32);
                                self.add((0x80 | (cp & 0x3f)) as i32);
                            } else if cp <= 0xffff {
                                self.add((0xe0 | (cp >> 12)) as i32);
                                self.add((0x80 | ((cp >> 6) & 0x3f)) as i32);
                                self.add((0x80 | (cp & 0x3f)) as i32);
                            } else {
                                self.add((0xf0 | (cp >> 18)) as i32);
                                self.add((0x80 | ((cp >> 12) & 0x3f)) as i32);
                                self.add((0x80 | ((cp >> 6) & 0x3f)) as i32);
                                self.add((0x80 | (cp & 0x3f)) as i32);
                            }
                        }
                        _ => {
                            self.error_message = "invalid string: forbidden character after backslash";
                            return Tok::ParseError;
                        }
                    }
                }
                0x00..=0x1f => {
                    self.error_message = CONTROL_MESSAGES[c as usize];
                    return Tok::ParseError;
                }
                0x20..=0x7f => self.add(c),
                0xc2..=0xdf => {
                    if !self.next_byte_in_range(&[0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xe0 => {
                    if !self.next_byte_in_range(&[0xa0, 0xbf, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xe1..=0xec | 0xee | 0xef => {
                    if !self.next_byte_in_range(&[0x80, 0xbf, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xed => {
                    if !self.next_byte_in_range(&[0x80, 0x9f, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xf0 => {
                    if !self.next_byte_in_range(&[0x90, 0xbf, 0x80, 0xbf, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xf1..=0xf3 => {
                    if !self.next_byte_in_range(&[0x80, 0xbf, 0x80, 0xbf, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                0xf4 => {
                    if !self.next_byte_in_range(&[0x80, 0x8f, 0x80, 0xbf, 0x80, 0xbf]) {
                        return Tok::ParseError;
                    }
                }
                _ => {
                    self.error_message = ILL_FORMED;
                    return Tok::ParseError;
                }
            }
        }
    }

    fn is_digit(c: i32) -> bool {
        (b'0' as i32..=b'9' as i32).contains(&c)
    }

    fn scan_number(&mut self) -> Tok {
        #[derive(Clone, Copy)]
        enum St {
            Minus,
            Zero,
            Any1,
            Decimal1,
            Decimal2,
            Exponent,
            Sign,
            Any2,
            Done,
        }
        self.reset();
        let mut number_type = Tok::ValueUnsigned;
        let c = self.current;
        self.add(c);
        let mut st = match c {
            0x2d => St::Minus,
            0x30 => St::Zero,
            _ => St::Any1,
        };
        loop {
            st = match st {
                St::Minus => {
                    number_type = Tok::ValueInteger;
                    let c = self.get();
                    if c == b'0' as i32 {
                        self.add(c);
                        St::Zero
                    } else if Self::is_digit(c) {
                        self.add(c);
                        St::Any1
                    } else {
                        self.error_message = "invalid number; expected digit after '-'";
                        return Tok::ParseError;
                    }
                }
                St::Zero => {
                    let c = self.get();
                    if c == b'.' as i32 {
                        self.add(b'.' as i32);
                        St::Decimal1
                    } else if c == b'e' as i32 || c == b'E' as i32 {
                        self.add(c);
                        St::Exponent
                    } else {
                        St::Done
                    }
                }
                St::Any1 => {
                    let c = self.get();
                    if Self::is_digit(c) {
                        self.add(c);
                        St::Any1
                    } else if c == b'.' as i32 {
                        self.add(b'.' as i32);
                        St::Decimal1
                    } else if c == b'e' as i32 || c == b'E' as i32 {
                        self.add(c);
                        St::Exponent
                    } else {
                        St::Done
                    }
                }
                St::Decimal1 => {
                    number_type = Tok::ValueFloat;
                    let c = self.get();
                    if Self::is_digit(c) {
                        self.add(c);
                        St::Decimal2
                    } else {
                        self.error_message = "invalid number; expected digit after '.'";
                        return Tok::ParseError;
                    }
                }
                St::Decimal2 => {
                    let c = self.get();
                    if Self::is_digit(c) {
                        self.add(c);
                        St::Decimal2
                    } else if c == b'e' as i32 || c == b'E' as i32 {
                        self.add(c);
                        St::Exponent
                    } else {
                        St::Done
                    }
                }
                St::Exponent => {
                    number_type = Tok::ValueFloat;
                    let c = self.get();
                    if c == b'+' as i32 || c == b'-' as i32 {
                        self.add(c);
                        St::Sign
                    } else if Self::is_digit(c) {
                        self.add(c);
                        St::Any2
                    } else {
                        self.error_message = "invalid number; expected '+', '-', or digit after exponent";
                        return Tok::ParseError;
                    }
                }
                St::Sign => {
                    let c = self.get();
                    if Self::is_digit(c) {
                        self.add(c);
                        St::Any2
                    } else {
                        self.error_message = "invalid number; expected digit after exponent sign";
                        return Tok::ParseError;
                    }
                }
                St::Any2 => {
                    let c = self.get();
                    if Self::is_digit(c) {
                        self.add(c);
                        St::Any2
                    } else {
                        St::Done
                    }
                }
                St::Done => break,
            };
        }
        self.unget();
        // 字句は ASCII の数字と記号だけ
        let text = std::str::from_utf8(&self.token_buffer).unwrap_or("");
        // strtoull / strtoll が桁あふれしたら (errno が ERANGE)、浮動小数点数として読む
        if number_type == Tok::ValueUnsigned {
            if let Ok(x) = text.parse::<u64>() {
                self.value_unsigned = x;
                return Tok::ValueUnsigned;
            }
        } else if number_type == Tok::ValueInteger {
            if let Ok(x) = text.parse::<i64>() {
                self.value_integer = x;
                return Tok::ValueInteger;
            }
        }
        // strtod と同じく、最も近い値に丸める
        self.value_float = text.parse::<f64>().unwrap_or(0.0);
        Tok::ValueFloat
    }

    fn scan_literal(&mut self, text: &[u8], t: Tok) -> Tok {
        for &b in &text[1..] {
            if self.get() != b as i32 {
                self.error_message = "invalid literal";
                return Tok::ParseError;
            }
        }
        t
    }

    fn skip_bom(&mut self) -> bool {
        if self.get() == 0xef {
            return self.get() == 0xbb && self.get() == 0xbf;
        }
        self.unget();
        true
    }

    fn scan(&mut self) -> Tok {
        if self.position.chars_read_total == 0 && !self.skip_bom() {
            self.error_message = "invalid BOM; must be 0xEF 0xBB 0xBF if given";
            return Tok::ParseError;
        }
        loop {
            self.get();
            if !matches!(self.current, 0x20 | 0x09 | 0x0a | 0x0d) {
                break;
            }
        }
        match self.current {
            0x5b => Tok::BeginArray,
            0x5d => Tok::EndArray,
            0x7b => Tok::BeginObject,
            0x7d => Tok::EndObject,
            0x3a => Tok::NameSeparator,
            0x2c => Tok::ValueSeparator,
            0x74 => self.scan_literal(b"true", Tok::LiteralTrue),
            0x66 => self.scan_literal(b"false", Tok::LiteralFalse),
            0x6e => self.scan_literal(b"null", Tok::LiteralNull),
            0x22 => self.scan_string(),
            0x2d | 0x30..=0x39 => self.scan_number(),
            // NUL も入力の終わりとみなす
            0x00 | EOF => Tok::EndOfInput,
            _ => {
                self.error_message = "invalid literal";
                Tok::ParseError
            }
        }
    }
}

/// 組み立て中の配列かオブジェクト
enum Frame {
    Array(Vec<Value>),
    Object(Object, Vec<u8>),
}

struct Builder {
    stack: Vec<Frame>,
    root: Value,
}

impl Builder {
    fn value(&mut self, v: Value) {
        match self.stack.last_mut() {
            None => self.root = v,
            Some(Frame::Array(a)) => a.push(v),
            Some(Frame::Object(o, key)) => {
                o.insert(std::mem::take(key), v);
            }
        }
    }

    fn end(&mut self) {
        let v = match self.stack.pop() {
            Some(Frame::Array(a)) => Value::Array(a),
            Some(Frame::Object(o, _)) => Value::Object(o),
            None => return,
        };
        self.value(v);
    }

    fn key(&mut self, k: &[u8]) {
        if let Some(Frame::Object(_, key)) = self.stack.last_mut() {
            *key = k.to_vec();
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    last_token: Tok,
}

impl<'a> Parser<'a> {
    fn get_token(&mut self) -> Tok {
        self.last_token = self.lexer.scan();
        self.last_token
    }

    fn exception_message(&self, expected: Tok, context: &str) -> Vec<u8> {
        let mut msg = b"syntax error ".to_vec();
        if !context.is_empty() {
            msg.extend_from_slice(format!("while parsing {} ", context).as_bytes());
        }
        msg.extend_from_slice(b"- ");
        if self.last_token == Tok::ParseError {
            msg.extend_from_slice(self.lexer.error_message.as_bytes());
            msg.extend_from_slice(b"; last read: '");
            msg.extend_from_slice(&self.lexer.get_token_string());
            msg.push(b'\'');
        } else {
            msg.extend_from_slice(format!("unexpected {}", token_type_name(self.last_token)).as_bytes());
        }
        if expected != Tok::Uninitialized {
            msg.extend_from_slice(format!("; expected {}", token_type_name(expected)).as_bytes());
        }
        msg
    }

    /// `parse_error::create(101, position, exception_message(expected, context))`
    fn error(&self, expected: Tok, context: &str) -> ParseError {
        let p = self.lexer.position;
        let mut w = format!(
            "[json.exception.parse_error.101] parse error at line {}, column {}: ",
            p.lines_read + 1,
            p.chars_read_current_line
        )
        .into_bytes();
        w.extend_from_slice(&self.exception_message(expected, context));
        ParseError::Parse(w)
    }

    fn parse_internal(&mut self, b: &mut Builder) -> Result<(), ParseError> {
        let mut states: Vec<bool> = Vec::new();
        let mut skip_to_state_evaluation = false;
        loop {
            if !skip_to_state_evaluation {
                match self.last_token {
                    Tok::BeginObject => {
                        b.stack.push(Frame::Object(Object::new(), Vec::new()));
                        if self.get_token() == Tok::EndObject {
                            b.end();
                        } else {
                            if self.last_token != Tok::ValueString {
                                return Err(self.error(Tok::ValueString, "object key"));
                            }
                            b.key(&self.lexer.token_buffer);
                            if self.get_token() != Tok::NameSeparator {
                                return Err(self.error(Tok::NameSeparator, "object separator"));
                            }
                            states.push(false);
                            self.get_token();
                            continue;
                        }
                    }
                    Tok::BeginArray => {
                        b.stack.push(Frame::Array(Vec::new()));
                        if self.get_token() == Tok::EndArray {
                            b.end();
                        } else {
                            states.push(true);
                            continue;
                        }
                    }
                    Tok::ValueFloat => {
                        let res = self.lexer.value_float;
                        if !res.is_finite() {
                            let mut w = b"[json.exception.out_of_range.406] number overflow parsing '".to_vec();
                            w.extend_from_slice(&self.lexer.get_token_string());
                            w.push(b'\'');
                            return Err(ParseError::OutOfRange(w));
                        }
                        b.value(Value::Float(res));
                    }
                    Tok::LiteralFalse => b.value(Value::Bool(false)),
                    Tok::LiteralNull => b.value(Value::Null),
                    Tok::LiteralTrue => b.value(Value::Bool(true)),
                    Tok::ValueInteger => b.value(Value::Int(self.lexer.value_integer)),
                    Tok::ValueString => b.value(Value::Str(self.lexer.token_buffer.clone())),
                    Tok::ValueUnsigned => b.value(Value::UInt(self.lexer.value_unsigned)),
                    Tok::ParseError => return Err(self.error(Tok::Uninitialized, "value")),
                    _ => return Err(self.error(Tok::LiteralOrValue, "value")),
                }
            } else {
                skip_to_state_evaluation = false;
            }

            let array = match states.last() {
                None => return Ok(()),
                Some(&a) => a,
            };
            if array {
                if self.get_token() == Tok::ValueSeparator {
                    self.get_token();
                    continue;
                }
                if self.last_token == Tok::EndArray {
                    b.end();
                    states.pop();
                    skip_to_state_evaluation = true;
                    continue;
                }
                return Err(self.error(Tok::EndArray, "array"));
            } else {
                if self.get_token() == Tok::ValueSeparator {
                    if self.get_token() != Tok::ValueString {
                        return Err(self.error(Tok::ValueString, "object key"));
                    }
                    b.key(&self.lexer.token_buffer);
                    if self.get_token() != Tok::NameSeparator {
                        return Err(self.error(Tok::NameSeparator, "object separator"));
                    }
                    self.get_token();
                    continue;
                }
                if self.last_token == Tok::EndObject {
                    b.end();
                    states.pop();
                    skip_to_state_evaluation = true;
                    continue;
                }
                return Err(self.error(Tok::EndObject, "object"));
            }
        }
    }
}

/// `json::parse(input)`。最後まで 1 つの値でなければ `parse_error`。
///
/// 入れ子の深さに上限はない (nlohmann と同じ)。できた値を捨てるときなどは深さの分だけ再帰するので、
/// 信頼できない入力は呼ぶ側で深さを確かめること (`jrpc` は 64 段を超えるものを先に弾く)。
pub fn parse(input: &[u8]) -> Result<Value, ParseError> {
    let mut p = Parser { lexer: Lexer::new(input), last_token: Tok::Uninitialized };
    p.get_token();
    let mut b = Builder { stack: Vec::new(), root: Value::Null };
    p.parse_internal(&mut b)?;
    if p.get_token() != Tok::EndOfInput {
        return Err(p.error(Tok::EndOfInput, "value"));
    }
    Ok(b.root)
}
