use super::*;

fn d(v: &Value) -> String {
    String::from_utf8(dump(v).unwrap()).unwrap()
}

fn p(s: &str) -> Value {
    parse(s.as_bytes()).unwrap()
}

fn perr(s: &[u8]) -> String {
    String::from_utf8_lossy(parse(s).unwrap_err().what()).into_owned()
}

#[test]
fn dump_floats_like_nlohmann() {
    for (x, want) in [
        (1.0, "1.0"),
        (0.1, "0.1"),
        (-0.0, "-0.0"),
        (0.0, "0.0"),
        (1e100, "1e+100"),
        (1e15, "1e+15"),
        (1e14, "100000000000000.0"),
        (123.456, "123.456"),
        (0.0001, "0.0001"),
        (0.00001, "1e-05"),
        (1.5e-300, "1.5e-300"),
        (5e-324, "5e-324"),
        (1.7976931348623157e308, "1.7976931348623157e+308"),
        (123456789012345680.0, "1.2345678901234568e+17"),
        (f64::NAN, "null"),
        (f64::INFINITY, "null"),
    ] {
        assert_eq!(d(&Value::Float(x)), want, "{}", x);
    }
}

#[test]
fn dump_floats_round_trip() {
    // Grisu2 は最短とは限らないが、読み戻すと同じ値になる
    let mut x: u64 = 0x1234_5678_9abc_def1;
    for _ in 0..100_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let f = f64::from_bits(x);
        if !f.is_finite() {
            continue;
        }
        let s = d(&Value::Float(f));
        assert_eq!(s.parse::<f64>().unwrap().to_bits(), f.to_bits(), "{}", s);
    }
}

#[test]
fn dump_strings() {
    assert_eq!(d(&Value::str(b"a\"\\/\x08\x0c\n\r\t\x01\x1f\x7f")), "\"a\\\"\\\\/\\b\\f\\n\\r\\t\\u0001\\u001f\x7f\"");
    assert_eq!(d(&Value::str("日本語".as_bytes())), "\"日本語\"");
    assert_eq!(
        dump(&Value::str(b"ab\xff")).unwrap_err(),
        b"[json.exception.type_error.316] invalid UTF-8 byte at index 2: 0xFF"
    );
    // 過長表現は 2 バイト目で弾かれる
    assert_eq!(
        dump(&Value::str(b"\xe0\x80\x80")).unwrap_err(),
        b"[json.exception.type_error.316] invalid UTF-8 byte at index 1: 0x80"
    );
    assert_eq!(
        dump(&Value::str(b"a\xe3\x81")).unwrap_err(),
        b"[json.exception.type_error.316] incomplete UTF-8 string; last byte: 0x81"
    );
    // オブジェクトのキーも同じ
    let mut o = Object::new();
    o.insert(b"\xc0".to_vec(), Value::Null);
    assert!(dump(&Value::Object(o)).is_err());
}

#[test]
fn dump_containers() {
    let v = p(r#" {"b":[1,-2,3.5,true,false,null,{}],"a":"x","c":[]} "#);
    assert_eq!(d(&v), r#"{"a":"x","b":[1,-2,3.5,true,false,null,{}],"c":[]}"#);
}

#[test]
fn parse_numbers() {
    assert_eq!(p("0"), Value::UInt(0));
    assert_eq!(p("-0"), Value::Int(0));
    assert_eq!(p("-0.0"), Value::Float(-0.0));
    assert_eq!(p("18446744073709551615"), Value::UInt(u64::MAX));
    assert_eq!(p("18446744073709551616"), Value::Float(18446744073709551616.0));
    assert_eq!(p("-9223372036854775808"), Value::Int(i64::MIN));
    assert_eq!(p("-9223372036854775809"), Value::Float(-9223372036854775809.0));
    assert_eq!(p("1E+2"), Value::Float(100.0));
    assert_eq!(p("1e-400"), Value::Float(0.0));
    assert_eq!(
        parse(b"1e400"),
        Err(ParseError::OutOfRange(b"[json.exception.out_of_range.406] number overflow parsing '1e400'".to_vec()))
    );
}

#[test]
fn parse_strings() {
    assert_eq!(p(r#""\u00e9\ud83d\ude00\/""#), Value::str("é😀/".as_bytes()));
    assert_eq!(p(r#""\u0000""#), Value::str(b"\0"));
    // 同じキーは後のもの
    assert_eq!(p(r#"{"a":1,"a":{"b":2}}"#), p(r#"{"a":{"b":2}}"#));
    // BOM は読み飛ばす。NUL は入力の終わり
    assert_eq!(parse(b"\xef\xbb\xbf[1]"), Ok(Value::Array(vec![Value::UInt(1)])));
    assert_eq!(parse(b"{}\0garbage"), Ok(Value::Object(Object::new())));
}

#[test]
fn parse_errors() {
    assert_eq!(
        perr(b""),
        "[json.exception.parse_error.101] parse error at line 1, column 1: syntax error while parsing value - \
         unexpected end of input; expected '[', '{', or a literal"
    );
    assert_eq!(
        perr(b"[1,]"),
        "[json.exception.parse_error.101] parse error at line 1, column 4: syntax error while parsing value - \
         unexpected ']'; expected '[', '{', or a literal"
    );
    assert_eq!(
        perr(b"{\"a\" 1}"),
        "[json.exception.parse_error.101] parse error at line 1, column 6: syntax error while parsing object separator - \
         unexpected number literal; expected ':'"
    );
    assert_eq!(
        perr(b"\"a\nb\""),
        "[json.exception.parse_error.101] parse error at line 2, column 0: syntax error while parsing value - \
         invalid string: control character U+000A (LF) must be escaped to \\u000A or \\n; last read: '\"a<U+000A>'"
    );
    assert_eq!(
        perr(b"[1] 2"),
        "[json.exception.parse_error.101] parse error at line 1, column 5: syntax error while parsing value - \
         unexpected number literal; expected end of input"
    );
    assert!(perr(b"nan").contains("invalid literal"));
    assert!(perr(b"\"\\ud800\"").contains("surrogate"));
    assert!(perr(b"\"\xc0\x80\"").contains("ill-formed UTF-8 byte"));
    assert!(perr(b"\xef\xbb").contains("invalid BOM"));
}

#[test]
fn conversions() {
    assert_eq!(Value::Bool(true).as_int(), Ok(1));
    assert_eq!(Value::Float(-1.9).as_int(), Ok(-1));
    assert_eq!(Value::Float(1e10).as_int(), Ok(i32::MIN));
    assert_eq!(Value::UInt(4294967297).as_int(), Ok(1));
    assert_eq!(
        Value::Bool(true).as_size(),
        Err(b"[json.exception.type_error.302] type must be number, but is boolean".to_vec())
    );
    assert_eq!(Value::Int(-1).as_size(), Ok(u64::MAX));
    assert_eq!(Value::Float(1e19).as_size(), Ok(10_000_000_000_000_000_000));
    assert_eq!(Value::Float(1e20).as_size(), Ok(0));
    assert_eq!(Value::Float(-5.0).as_size(), Ok(-5i64 as u64));
    assert_eq!(
        Value::Null.as_string(),
        Err(b"[json.exception.type_error.302] type must be string, but is null".to_vec())
    );
    let mut v = Value::UInt(1);
    assert_eq!(
        v.index_mut(b"k").unwrap_err(),
        b"[json.exception.type_error.305] cannot use operator[] with a string argument with number".to_vec()
    );
    let mut n = Value::Null;
    assert_eq!(*n.index_mut(b"k").unwrap(), Value::Null);
    assert_eq!(d(&n), r#"{"k":null}"#);
}

/// 変異させた入力でパニックしない (深さは 64 段までにしておく)
#[test]
fn fuzz_parse_dump() {
    let seeds: [&[u8]; 6] = [
        br#"{"jsonrpc":"2.0","id":1,"method":"getLog","params":{"from":0,"maxLines":-1.5e3}}"#,
        br#"[1,-0,0.5e-3,"\u00e9\ud83d\ude00",true,false,null,{"a":[]}]"#,
        b"\xef\xbb\xbf \t\r\n\"\xe3\x81\x82\\n\"",
        b"18446744073709551616",
        b"-9223372036854775809e2",
        br#"{"a":{"b":{"c":"\ud800\udc00"}}}"#,
    ];
    let mut x: u32 = 12345;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for i in 0..300_000 {
        let mut s = seeds[i % seeds.len()].to_vec();
        for _ in 0..1 + rnd() % 4 {
            let pos = (rnd() as usize) % (s.len() + 1);
            match rnd() % 3 {
                0 if pos < s.len() => s[pos] = rnd() as u8,
                1 => s.insert(pos, rnd() as u8),
                _ if pos < s.len() => {
                    s.remove(pos);
                }
                _ => {}
            }
        }
        if let Ok(v) = parse(&s) {
            // 読めたものは必ず書き出せ、読み戻して書き出すと同じになる (-0 は整数の 0 になる)
            let out = dump(&v).unwrap();
            assert_eq!(dump(&parse(&out).unwrap()).unwrap(), out);
        }
    }
}
