//! glibc の `strtod` / `atof` (C ロケール)。
//!
//! 先頭の空白を読み飛ばし、読めるところまでを 10 進数 (`1.5e3`)、16 進数 (`0x1.8p3`)、
//! `inf` / `infinity`、`nan` / `nan(...)` として読む。10 進数は正しく丸める (Rust の
//! `f64::from_str` も glibc も、最も近い値に丸める)。

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn starts_with_ignore_case(s: &[u8], pat: &[u8]) -> bool {
    s.len() >= pat.len() && s[..pat.len()].eq_ignore_ascii_case(pat)
}

/// `strtod`: 値と、読んだバイト数 (何も読めなければ 0 と 0)
pub fn strtod(s: &[u8]) -> (f64, usize) {
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
    let sign = |v: f64| if neg { -v } else { v };
    let rest = &s[i..];

    if starts_with_ignore_case(rest, b"infinity") {
        return (sign(f64::INFINITY), i + 8);
    }
    if starts_with_ignore_case(rest, b"inf") {
        return (sign(f64::INFINITY), i + 3);
    }
    if starts_with_ignore_case(rest, b"nan") {
        let mut n = i + 3;
        // nan(英数字と _) のかっこは、閉じていれば読む
        if s.get(n) == Some(&b'(') {
            let k = s[n + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
            if s.get(n + 1 + k) == Some(&b')') {
                n += k + 2;
            }
        }
        return (sign(f64::NAN), n);
    }
    if (rest.starts_with(b"0x") || rest.starts_with(b"0X")) && hex_digits_follow(&rest[2..]) {
        let (v, n) = hex(&rest[2..]);
        return (sign(v), i + 2 + n);
    }

    // 10 進数: 数字 ['.' 数字] (数字は少なくとも 1 つ) [e [符号] 数字]
    let int_digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    let mut n = int_digits;
    let mut frac_digits = 0;
    if rest.get(n) == Some(&b'.') {
        frac_digits = rest[n + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if int_digits + frac_digits > 0 {
            n += 1 + frac_digits;
        }
    }
    if int_digits + frac_digits == 0 {
        return (0.0, 0);
    }
    if matches!(rest.get(n), Some(b'e' | b'E')) {
        let mut k = n + 1;
        if matches!(rest.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        let exp_digits = rest[k.min(rest.len())..].iter().take_while(|c| c.is_ascii_digit()).count();
        if exp_digits > 0 {
            n = k + exp_digits;
        }
    }
    let text = std::str::from_utf8(&rest[..n]).unwrap();
    // "1." や ".5" も Rust の from_str で読める。巨大な指数も inf や 0 になる。
    let v: f64 = text.parse().unwrap_or(0.0);
    (sign(v), i + n)
}

fn hex_digits_follow(s: &[u8]) -> bool {
    match s.first() {
        Some(c) if c.is_ascii_hexdigit() => true,
        Some(b'.') => s.get(1).is_some_and(|c| c.is_ascii_hexdigit()),
        _ => false,
    }
}

/// "0x" の後ろ。16 進の仮数 ['.' 小数部] [p [符号] 10 進の指数]
fn hex(s: &[u8]) -> (f64, usize) {
    let mut mant: u64 = 0;
    let mut sticky = false;
    let mut exp: i64 = 0; // 2 の指数
    let mut n = 0;
    let mut seen_point = false;
    loop {
        match s.get(n) {
            Some(&c) if c.is_ascii_hexdigit() => {
                let d = (c as char).to_digit(16).unwrap() as u64;
                if mant >> 60 == 0 {
                    mant = mant << 4 | d;
                    if seen_point {
                        exp -= 4;
                    }
                } else {
                    // 64 ビットに収まらない桁は、丸めのためにあったかどうかだけ覚える
                    sticky |= d != 0;
                    if !seen_point {
                        exp += 4;
                    }
                }
            }
            Some(b'.') if !seen_point => seen_point = true,
            _ => break,
        }
        n += 1;
    }
    if matches!(s.get(n), Some(b'p' | b'P')) {
        let mut k = n + 1;
        let eneg = match s.get(k) {
            Some(b'-') => {
                k += 1;
                true
            }
            Some(b'+') => {
                k += 1;
                false
            }
            _ => false,
        };
        let digits = s[k.min(s.len())..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let mut e: i64 = 0;
            for &c in &s[k..k + digits] {
                e = (e * 10 + (c - b'0') as i64).min(1 << 20);
            }
            exp += if eneg { -e } else { e };
            n = k + digits;
        }
    }
    if mant == 0 {
        return (0.0, n);
    }
    // 下位の桁があれば最下位ビットを立てる (53 ビットへの丸めの結果が正しくなる)
    let m = mant | sticky as u64;
    let mut v = m as f64;
    let mut e = exp.clamp(-2200, 2200) as i32;
    while e > 0 {
        let step = e.min(1000);
        v *= 2f64.powi(step);
        e -= step;
    }
    while e < 0 {
        let step = (-e).min(1000);
        v /= 2f64.powi(step);
        e += step;
    }
    (v, n)
}

/// `atof`: C 文字列として (最初の NUL まで) 読む
pub fn atof(s: &[u8]) -> f64 {
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    strtod(s).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal() {
        assert_eq!(strtod(b"0.5"), (0.5, 3));
        assert_eq!(strtod(b"  -1.5e3x"), (-1500.0, 8));
        assert_eq!(strtod(b"1e"), (1.0, 1));
        assert_eq!(strtod(b"1e+"), (1.0, 1));
        assert_eq!(strtod(b".5"), (0.5, 2));
        assert_eq!(strtod(b"5."), (5.0, 2));
        assert_eq!(strtod(b"."), (0.0, 0));
        assert_eq!(strtod(b"q=1"), (0.0, 0));
        assert_eq!(strtod(b"1e999"), (f64::INFINITY, 5));
        assert_eq!(strtod(b"1e-999").0, 0.0);
        assert_eq!(atof(b"0.1"), 0.1);
        assert_eq!(atof(b"0.8\x000.9"), 0.8);
    }

    #[test]
    fn special() {
        assert_eq!(strtod(b"INF"), (f64::INFINITY, 3));
        assert_eq!(strtod(b"-Infinity"), (f64::NEG_INFINITY, 9));
        let (v, n) = strtod(b"nan(abc)x");
        assert!(v.is_nan());
        assert_eq!(n, 8);
        let (v, n) = strtod(b"nan(a");
        assert!(v.is_nan());
        assert_eq!(n, 3);
        assert_eq!(strtod(b"0x1p3"), (8.0, 5));
        assert_eq!(strtod(b"0x1.8"), (1.5, 5));
        assert_eq!(strtod(b"0x.8p1"), (1.0, 6));
        assert_eq!(strtod(b"0x"), (0.0, 1));
        assert_eq!(strtod(b"0xg"), (0.0, 1));
        assert_eq!(strtod(b"0x1p"), (1.0, 3));
    }
}
