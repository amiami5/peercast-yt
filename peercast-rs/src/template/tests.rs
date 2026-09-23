//! テンプレートエンジンの単体テスト (C++ 版の tests/template_unittest.cpp の内容を含む)

use super::*;
use crate::reader::Abort;

/// 入力は StringStream と同じく末尾で読むと中断する。スコープは先頭から順に探す。
#[derive(Default)]
struct TestHost {
    input: Vec<u8>,
    pos: usize,
    out: Vec<u8>,
    /// (GenericScope か, 変数)
    scopes: Vec<(bool, BTreeMap<Vec<u8>, Value>)>,
    selected: Vec<u8>,
    current: Vec<u8>,
}

impl TestHost {
    fn new(input: &str) -> TestHost {
        let mut page = BTreeMap::new();
        page.insert(b"page".to_vec(), Value::Object(BTreeMap::new()));
        let mut root = BTreeMap::new();
        root.insert(b"servMgr.version".to_vec(), Value::str(b"v0.1218"));
        TestHost { input: input.as_bytes().to_vec(), scopes: vec![(true, page), (false, root)], ..Default::default() }
    }
}

impl Host for TestHost {
    fn read_char(&mut self) -> std::result::Result<u8, Abort> {
        let c = *self.input.get(self.pos).ok_or(Abort)?;
        self.pos += 1;
        Ok(c)
    }
    fn eof(&mut self) -> std::result::Result<bool, Abort> {
        Ok(self.pos >= self.input.len())
    }
    fn position(&mut self) -> std::result::Result<i32, Abort> {
        Ok(self.pos as i32)
    }
    fn seek(&mut self, pos: i32) -> std::result::Result<(), Abort> {
        self.pos = pos as usize;
        Ok(())
    }
    fn write(&mut self, data: &[u8]) -> std::result::Result<(), Abort> {
        self.out.extend_from_slice(data);
        Ok(())
    }
    fn lookup(&mut self, name: &[u8]) -> std::result::Result<Value, Abort> {
        for (_, vars) in &self.scopes {
            if let Some(v) = vars.get(name) {
                return Ok(v.clone());
            }
        }
        Ok(match name {
            b"TRUE" => Value::str(b"1"),
            b"FALSE" => Value::str(b"0"),
            b"true" => Value::Bool(true),
            b"false" => Value::Bool(false),
            _ => Value::Null,
        })
    }
    fn push_scope(&mut self) {
        self.scopes.insert(0, (true, BTreeMap::new()));
    }
    fn pop_scope(&mut self) {
        self.scopes.remove(0);
    }
    fn front_is_generic(&mut self) -> bool {
        self.scopes[0].0
    }
    fn set_front(&mut self, name: &[u8], value: &Value) -> std::result::Result<(), Abort> {
        self.scopes[0].1.insert(name.to_vec(), value.clone());
        Ok(())
    }
    fn regex_check(&mut self, pattern: &[u8]) -> std::result::Result<(), Abort> {
        if pattern == b"(" {
            Err(Abort)
        } else {
            Ok(())
        }
    }
    // テスト用の簡単なもの: ^ と $ だけを解釈する
    fn regex_match(&mut self, pattern: &[u8], subject: &[u8]) -> std::result::Result<bool, Abort> {
        let (start, p) = match pattern.strip_prefix(b"^") {
            Some(p) => (true, p),
            None => (false, pattern),
        };
        let (end, p) = match p.strip_suffix(b"$") {
            Some(p) => (true, p),
            None => (false, p),
        };
        Ok(match (start, end) {
            (true, true) => subject == p,
            (true, false) => subject.starts_with(p),
            (false, true) => subject.ends_with(p),
            (false, false) => subject.windows(p.len().max(1)).any(|w| w == p) || p.is_empty(),
        })
    }
    fn selected_fragment(&mut self) -> Vec<u8> {
        self.selected.clone()
    }
    fn current_fragment(&mut self) -> Vec<u8> {
        self.current.clone()
    }
    fn set_current_fragment(&mut self, f: &[u8]) {
        self.current = f.to_vec();
    }
    fn log_error(&mut self, _msg: &str) {}
}

fn render(input: &str) -> Result<String> {
    render_with(TestHost::new(input))
}

fn render_with(mut h: TestHost) -> Result<String> {
    let e = Engine::new(&mut h);
    let r = e.read_template_top(true);
    r.map(|t| {
        assert_eq!(t, TMPL_END);
        String::from_utf8(h.out.clone()).unwrap()
    })
}

impl Engine<'_> {
    fn read_template_top(mut self, out: bool) -> Result<i32> {
        let r = self.read_template(out);
        self.finish(r)
    }
}

fn eval(s: &str) -> Result<Value> {
    let mut h = TestHost::new("");
    let mut e = Engine::new(&mut h);
    e.eval_str(s.as_bytes())
}

fn toks(s: &str) -> Vec<String> {
    tokenize(s.as_bytes()).unwrap().into_iter().map(|t| String::from_utf8(t).unwrap()).collect()
}

fn parsed(s: &str) -> String {
    let mut t: VecDeque<_> = tokenize(s.as_bytes()).unwrap().into();
    String::from_utf8(parse(&mut t).unwrap().inspect().unwrap()).unwrap()
}

fn general(s: &str) -> Error {
    Error::General(s.as_bytes().to_vec())
}

#[test]
fn read_template_basics() {
    assert_eq!(render("hoge").unwrap(), "hoge");
    assert_eq!(render("{$servMgr.version}").unwrap(), "v0.1218");
    assert_eq!(render("a{b}c{").unwrap_err(), Error::Abort); // '{' の次を読もうとする
    assert_eq!(render("a{b}c").unwrap(), "a{b}c");
    // バックスラッシュでエスケープ
    assert_eq!(render("{$\"\\}\"}").unwrap(), "}");
    assert_eq!(render("{\\ \"<a>'\" }").unwrap(), "\\x3Ca\\x3E\\'");
    assert_eq!(render("{! \"<b>\" }").unwrap(), "<b>");
    assert_eq!(render("{$ \"<b>\" }").unwrap(), "&lt;b&gt;");
}

#[test]
fn string_literals() {
    assert_eq!(eval_string_literal(b"\"abc\"").unwrap(), b"abc");
    assert_eq!(eval_string_literal(b"\"\"").unwrap(), b"");
    assert_eq!(read_string_literal(b"\"abc\"def").unwrap(), (b"\"abc\"".to_vec(), b"def".to_vec()));
    assert!(matches!(eval_string_literal(b"abc"), Err(Error::Stream(_))));
    assert!(matches!(read_string_literal(b"\"abc"), Err(Error::Stream(_))));
}

#[test]
fn tokens() {
    assert_eq!(toks("a==b"), ["a", "==", "b"]);
    assert_eq!(toks("a"), ["a"]);
    assert_eq!(toks("!a"), ["!", "a"]);
    assert_eq!(toks("inspect(x)"), ["inspect", "(", "x", ")"]);
    assert_eq!(toks(" \"a b\" ,'c' "), ["\"a b\"", ",", "'c'"]);
    assert_eq!(tokenize(b"a + b").unwrap_err(), Error::Stream(b"Unrecognized token. Error at \"+\"".to_vec()));
}

#[test]
fn parse_forms() {
    assert_eq!(parsed("inspect(x)"), r#"["inspect","x"]"#);
    assert_eq!(parsed("!x.y"), r#"["!","x.y"]"#);
    assert_eq!(parsed("!!x"), r#"["!",["!","x"]]"#);
    assert_eq!(parsed("x.y"), r#""x.y""#);
    assert_eq!(parsed("a == b"), r#"["==","a","b"]"#);
    assert_eq!(parsed("f(a) != g(b)"), r#"["!=",["f","a"],["g","b"]]"#);
    assert_eq!(parsed("\"a\""), r#"["quote","a"]"#);
    assert_eq!(parsed("'a'"), r#"["quote","a"]"#);
    assert_eq!(parsed("\"\\\"\""), r#"["quote","\""]"#);
    assert_eq!(parsed("1"), r#""1""#);
    assert_eq!(parsed("{}"), r#"["object"]"#);
    assert_eq!(parsed("{\"a\":1,\"b\":2}"), r#"["object",["quote","a"],"1",["quote","b"],"2"]"#);
    assert_eq!(parsed("[]"), r#"["array"]"#);
    assert_eq!(parsed("x[1]"), r#"["prop","x","1"]"#);
    assert_eq!(parsed("(f)(1)"), r#"["f","1"]"#);
    // 文字列の中の改行は、C++ 版の正規表現の `.` に一致しない
    let mut t: VecDeque<_> = tokenize(b"\"a\nb\"").unwrap().into();
    assert_eq!(parse(&mut t).unwrap_err(), general(" something weird "));
    let mut t: VecDeque<_> = tokenize(b"f(").unwrap().into();
    assert_eq!(parse(&mut t).unwrap_err(), general(" something weird "));
    let mut t: VecDeque<_> = tokenize(b"x[").unwrap().into();
    assert_eq!(parse(&mut t).unwrap_err(), general("exp expected"));
    let mut t: VecDeque<_> = tokenize(b"{a 1}").unwrap().into();
    assert_eq!(parse(&mut t).unwrap_err(), general("Got 1 while expecting ^:$"));
}

#[test]
fn conditions() {
    let c = |s: &str| is_truish(&eval(s).unwrap());
    assert!(c("TRUE"));
    assert!(!c("FALSE"));
    assert!(c("TRUE==TRUE"));
    assert!(!c("TRUE==FALSE"));
    assert!(!c("TRUE!=TRUE"));
    assert!(c("TRUE!=FALSE"));
    assert!(c("\"A\"==\"A\""));
    assert!(!c("\"A\"==\"B\""));
    assert!(c("\"A\"=~\"A\""));
    assert!(!c("\"A\"=~\"B\""));
    assert!(!c("\"A\"!~\"A\""));
    assert!(c("\"A\"!~\"B\""));
    assert!(c("\"ABC\"=~\"^A\""));
    assert!(c("\"ABC\"=~\"C$\""));
}

#[test]
fn if_elsif_else() {
    assert_eq!(render("{@if TRUE}yes{@else}no{@end}").unwrap(), "yes");
    assert_eq!(render("{@if FALSE}yes{@else}no{@end}").unwrap(), "no");
    assert_eq!(render("{@if TRUE}yes{@end}").unwrap(), "yes");
    assert_eq!(render("{@if FALSE}yes{@end}").unwrap(), "");
    let t = "{@if a}A{@elsif b}B{@elsif c}C{@else}D{@end}";
    let with = |var: &str| {
        let mut h = TestHost::new(t);
        if !var.is_empty() {
            h.scopes[0].1.insert(var.as_bytes().to_vec(), Value::Bool(true));
        }
        render_with(h).unwrap()
    };
    assert_eq!(with("a"), "A");
    assert_eq!(with("b"), "B");
    assert_eq!(with("c"), "C");
    assert_eq!(with(""), "D");
}

#[test]
fn fragments() {
    let t = "{@fragment a}A{$ 1 }{@end}{@fragment b}B{@end}C";
    assert_eq!(render(t).unwrap(), "A1BC");
    let sel = |f: &str| {
        let mut h = TestHost::new(t);
        h.selected = f.as_bytes().to_vec();
        render_with(h).unwrap()
    };
    assert_eq!(sel("a"), "A1");
    assert_eq!(sel("b"), "B");
}

#[test]
fn foreach_let_loop() {
    assert_eq!(render("{@foreach [1,2,3]}{$loop.index}:{$this},{@end}").unwrap(), "0:1,1:2,2:3,");
    assert_eq!(render("{@foreach []}x{@end}y").unwrap(), "y");
    assert_eq!(render("{@let a = 1, b = [a]}{$ inspect(b) }{@end}").unwrap(), "[1]");
    let mut h = TestHost::new("{@loop n}x{@end}");
    h.scopes[0].1.insert(b"n".to_vec(), Value::str(b" 3abc"));
    assert_eq!(render_with(h).unwrap(), "xxx");
    assert_eq!(render("{@loop 3}x{@end}").unwrap_err(), general("3 is not a Number. Value: null"));
    let mut h = TestHost::new("{@loop n}x{@end}");
    h.scopes[0].1.insert(b"n".to_vec(), Value::str(b"99999999999"));
    assert_eq!(render_with(h).unwrap_err(), general("stoi"));
    // int に収まらない数は -2^31 (1 回も回らない)。ループの中身は読み飛ばされずに外側で読まれ、
    // {@end} で外側が終わる (C++ 版と同じ)。
    for n in [1e10, -1e10, f64::NAN, f64::INFINITY] {
        let mut h = TestHost::new("{@loop n}x{@end}y");
        h.scopes[0].1.insert(b"n".to_vec(), Value::Number(n));
        assert_eq!(render_with(h).unwrap(), "x");
    }
}

#[test]
fn forms() {
    let s = |e: &str| String::from_utf8(eval(e).unwrap().inspect().unwrap()).unwrap();
    assert_eq!(s("merge({\"a\":1}, {\"b\":2})"), r#"{"a":1,"b":2}"#);
    assert_eq!(s("keys({\"b\":1,\"a\":2})"), r#"["a","b"]"#);
    assert_eq!(s("removeKey({\"a\":1,\"b\":2}, \"a\")"), r#"{"b":2}"#);
    assert_eq!(s("toQueryString({\"a b\":\"c&d\", \"n\":1})"), r#""a+b=c%26d&n=1""#);
    assert_eq!(s("nth(1, [5,6])"), "6");
    assert_eq!(s("prop({\"a\":1}, \"a\")"), "1");
    assert_eq!(s("prop({\"a\":1}, \"b\")"), "null");
    assert_eq!(s("{\"a\":{\"b\":2}}[\"a\"]"), r#"{"b":2}"#);
    assert_eq!(s("str(\"a\", 1, null, [1])"), r#""a1[1]""#);
    assert_eq!(s("replacePrefix(\"abc\", \"a\", \"x\")"), r#""xbc""#);
    assert_eq!(s("replaceSuffix(\"abc\", \"c\", \"x\")"), r#""abx""#);
    assert_eq!(s("and(1, 0, 2)"), "0");
    assert_eq!(s("or(0, \"\", 2)"), "2");
    assert_eq!(s("if(0, 1, 2)"), "2");
    assert_eq!(s("cond(0, 1, 1, 2)"), "2");
    assert_eq!(s("length([1,2])"), "2");
    assert_eq!(s("evalString(\"[1]\")"), "[1]");
    assert_eq!(s("(lambda([x], x))(7)"), "7");
    assert_eq!(s("99999999999"), "1215752191");
    assert_eq!(s("1.5"), "null"); // 識別子 (変数) として読む
    // 右辺から評価する
    assert_eq!(eval("length(1) == prop(1,2)").unwrap_err(), general("prop: 1 is not an object"));
    assert_eq!(eval("replacePrefix(length(1), prop(1,2), keys(1))").unwrap_err(), Error::runtime("not an object or an array"));
    assert_eq!(eval("nth(\"a\", 1)").unwrap_err(), Error::runtime("not a number"));
    assert_eq!(eval("length(1) =~ \"(\"").unwrap_err(), Error::Abort);
    assert_eq!(
        eval("define()").unwrap_err(),
        Error::OutOfRange(b"vector::_M_range_check: __n (which is 2) >= this->size() (which is 1)".to_vec())
    );
    assert_eq!(eval("nth(5, [1])").unwrap_err(), general("nth: index out of range"));
    assert_eq!(eval("foo(1)").unwrap_err(), general("\"foo\" is not a function"));
    assert_eq!(eval("a b").unwrap_err(), general("Unexpected token b"));
}

#[test]
fn let_sees_outer_variable() {
    // 右辺を先に評価するので、外側の page が見える
    assert_eq!(render("{@let page = page}{$ inspect(page) }{@end}").unwrap(), "{}");
    assert_eq!(render("{@let z = 0}{$ inspect(define(page, 1)) }{$ page }{@end}{$ inspect(page) }").unwrap(), "null1{}");
}

#[test]
fn deep_nesting_is_an_error() {
    let deep = "[".repeat(1000) + &"]".repeat(1000);
    assert_eq!(eval(&deep).unwrap_err(), general("Template: nesting too deep"));
    let deep = "{@if 1}".repeat(1000);
    assert_eq!(render(&deep).unwrap_err(), general("Template: nesting too deep"));
}

#[test]
fn fuzz() {
    use crate::media::testhost::Rng;
    let samples = [
        "{@if a == 'x'}A{@elsif !b}B{@else}C{@end}",
        "{@foreach [1,{\"k\":[2]},'s']}{$ inspect(this) }{$loop.indexBaseOne}{@end}",
        "{@let f = lambda([x], str(x, x))}{$ f('a') }{$ f() }{@end}",
        "{$ toQueryString(merge({'a':1}, {'b':[1,2]})) }{\\ keys({'q':1}) }{! nth(0, ['<']) }",
        "{@fragment x}{@loop 2}z{@end}{@end}{$ evalString('prop({\"a\":1}, \"a\")') }",
        "{$ define(v, 1) }{$ v }{$ (lambda([a,b], a))(1,2) }{$ cond(0,1,1,2) }",
    ];
    let mut rng = Rng(1234);
    for _ in 0..20000 {
        let sample = samples[rng.below(samples.len())];
        let t = rng.mutate(sample.as_bytes());
        let mut h = TestHost::new("");
        h.input = t;
        if rng.below(3) == 0 {
            h.selected = b"x".to_vec();
        }
        let e = Engine::new(&mut h);
        let _ = e.read_template_top(true);
    }
}
