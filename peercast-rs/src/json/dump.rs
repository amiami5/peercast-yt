//! nlohmann::json 3.7.3 の `dump()` (serializer) の移植。字下げなし、`ensure_ascii` なし、
//! 不正な UTF-8 は例外 (`error_handler_t::strict`)。浮動小数点数は nlohmann の Grisu2
//! (`dtoa_impl`) をそのまま移したもので書く (Grisu2 は最短の桁とは限らないので、Rust の
//! 標準の書き方とは違うことがある)。

use super::{type_error, Value};

/// `dump()`。文字列に不正な UTF-8 があれば、`type_error` (316) の `what()` を返す。
pub fn dump(v: &Value) -> Result<Vec<u8>, Vec<u8>> {
    let mut out = Vec::new();
    dump_to(v, &mut out)?;
    Ok(out)
}

fn dump_to(v: &Value, o: &mut Vec<u8>) -> Result<(), Vec<u8>> {
    match v {
        Value::Object(m) => {
            if m.is_empty() {
                o.extend_from_slice(b"{}");
                return Ok(());
            }
            o.push(b'{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    o.push(b',');
                }
                o.push(b'"');
                dump_escaped(k, o)?;
                o.extend_from_slice(b"\":");
                dump_to(v, o)?;
            }
            o.push(b'}');
        }
        Value::Array(a) => {
            if a.is_empty() {
                o.extend_from_slice(b"[]");
                return Ok(());
            }
            o.push(b'[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    o.push(b',');
                }
                dump_to(v, o)?;
            }
            o.push(b']');
        }
        Value::Str(s) => {
            o.push(b'"');
            dump_escaped(s, o)?;
            o.push(b'"');
        }
        Value::Bool(b) => o.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Int(i) => o.extend_from_slice(i.to_string().as_bytes()),
        Value::UInt(u) => o.extend_from_slice(u.to_string().as_bytes()),
        Value::Float(f) => dump_float(*f, o),
        Value::Null => o.extend_from_slice(b"null"),
    }
    Ok(())
}

const UTF8_ACCEPT: u8 = 0;
const UTF8_REJECT: u8 = 1;

/// Bjoern Hoehrmann の UTF-8 の DFA (nlohmann の `serializer::decode` と同じ表)
#[rustfmt::skip]
static UTF8D: [u8; 400] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 00..1F
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 20..3F
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 40..5F
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 60..7F
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, // 80..9F
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, // A0..BF
    8, 8, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // C0..DF
    0xA, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3, 0x4, 0x3, 0x3, // E0..EF
    0xB, 0x6, 0x6, 0x6, 0x5, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, 0x8, // F0..FF
    0x0, 0x1, 0x2, 0x3, 0x5, 0x8, 0x7, 0x1, 0x1, 0x1, 0x4, 0x6, 0x1, 0x1, 0x1, 0x1, // s0..s0
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, // s1..s2
    1, 2, 1, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, // s3..s4
    1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 3, 1, 3, 1, 1, 1, 1, 1, 1, // s5..s6
    1, 3, 1, 1, 1, 1, 1, 3, 1, 3, 1, 1, 1, 1, 1, 1, 1, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // s7..s8
];

fn decode(state: &mut u8, codep: &mut u32, byte: u8) -> u8 {
    let t = UTF8D[byte as usize];
    *codep = if *state != UTF8_ACCEPT {
        (byte as u32 & 0x3f) | (*codep << 6)
    } else {
        (0xff >> t) & byte as u32
    };
    *state = UTF8D[256 + *state as usize * 16 + t as usize];
    *state
}

fn dump_escaped(s: &[u8], o: &mut Vec<u8>) -> Result<(), Vec<u8>> {
    let mut codepoint = 0u32;
    let mut state = UTF8_ACCEPT;
    for (i, &byte) in s.iter().enumerate() {
        match decode(&mut state, &mut codepoint, byte) {
            UTF8_ACCEPT => match codepoint {
                0x08 => o.extend_from_slice(b"\\b"),
                0x09 => o.extend_from_slice(b"\\t"),
                0x0a => o.extend_from_slice(b"\\n"),
                0x0c => o.extend_from_slice(b"\\f"),
                0x0d => o.extend_from_slice(b"\\r"),
                0x22 => o.extend_from_slice(b"\\\""),
                0x5c => o.extend_from_slice(b"\\\\"),
                c if c <= 0x1f => o.extend_from_slice(format!("\\u{:04x}", c).as_bytes()),
                _ => o.push(byte),
            },
            UTF8_REJECT => {
                return Err(type_error(316, &format!("invalid UTF-8 byte at index {}: 0x{:02X}", i, byte)));
            }
            // 文字の途中のバイトは、そのまま書く
            _ => o.push(byte),
        }
    }
    if state != UTF8_ACCEPT {
        let last = s.last().copied().unwrap_or(0);
        return Err(type_error(316, &format!("incomplete UTF-8 string; last byte: 0x{:02X}", last)));
    }
    Ok(())
}

fn dump_float(x: f64, o: &mut Vec<u8>) {
    if !x.is_finite() {
        o.extend_from_slice(b"null");
        return;
    }
    to_chars(x, o);
}

// ---------------------------------------------------------------- Grisu2 (nlohmann の dtoa_impl)

#[derive(Clone, Copy)]
struct DiyFp {
    f: u64,
    e: i32,
}

impl DiyFp {
    fn sub(x: DiyFp, y: DiyFp) -> DiyFp {
        DiyFp { f: x.f - y.f, e: x.e }
    }

    fn mul(x: DiyFp, y: DiyFp) -> DiyFp {
        let u_lo = x.f & 0xffff_ffff;
        let u_hi = x.f >> 32;
        let v_lo = y.f & 0xffff_ffff;
        let v_hi = y.f >> 32;
        let p0 = u_lo * v_lo;
        let p1 = u_lo * v_hi;
        let p2 = u_hi * v_lo;
        let p3 = u_hi * v_hi;
        let p0_hi = p0 >> 32;
        let p1_lo = p1 & 0xffff_ffff;
        let p1_hi = p1 >> 32;
        let p2_lo = p2 & 0xffff_ffff;
        let p2_hi = p2 >> 32;
        let mut q = p0_hi + p1_lo + p2_lo;
        q += 1u64 << 31; // 丸め (同点は切り上げ)
        let h = p3 + p2_hi + p1_hi + (q >> 32);
        DiyFp { f: h, e: x.e + y.e + 64 }
    }

    fn normalize(mut x: DiyFp) -> DiyFp {
        while (x.f >> 63) == 0 {
            x.f <<= 1;
            x.e -= 1;
        }
        x
    }

    fn normalize_to(x: DiyFp, target_exponent: i32) -> DiyFp {
        let delta = x.e - target_exponent;
        DiyFp { f: x.f << delta, e: target_exponent }
    }
}

struct Boundaries {
    w: DiyFp,
    minus: DiyFp,
    plus: DiyFp,
}

fn compute_boundaries(value: f64) -> Boundaries {
    const PRECISION: i32 = 53;
    const BIAS: i32 = 1024 - 1 + (PRECISION - 1);
    const MIN_EXP: i32 = 1 - BIAS;
    const HIDDEN_BIT: u64 = 1u64 << (PRECISION - 1);
    let bits = value.to_bits();
    let e = bits >> (PRECISION - 1);
    let f = bits & (HIDDEN_BIT - 1);
    let is_denormal = e == 0;
    let v = if is_denormal {
        DiyFp { f, e: MIN_EXP }
    } else {
        DiyFp { f: f + HIDDEN_BIT, e: e as i32 - BIAS }
    };
    let lower_boundary_is_closer = f == 0 && e > 1;
    let m_plus = DiyFp { f: 2 * v.f + 1, e: v.e - 1 };
    let m_minus = if lower_boundary_is_closer {
        DiyFp { f: 4 * v.f - 1, e: v.e - 2 }
    } else {
        DiyFp { f: 2 * v.f - 1, e: v.e - 1 }
    };
    let w_plus = DiyFp::normalize(m_plus);
    let w_minus = DiyFp::normalize_to(m_minus, w_plus.e);
    Boundaries { w: DiyFp::normalize(v), minus: w_minus, plus: w_plus }
}

const ALPHA: i32 = -60;

#[rustfmt::skip]
static CACHED_POWERS: [(u64, i32, i32); 79] = [
    (0xAB70FE17C79AC6CA, -1060, -300), (0xFF77B1FCBEBCDC4F, -1034, -292), (0xBE5691EF416BD60C, -1007, -284),
    (0x8DD01FAD907FFC3C, -980, -276), (0xD3515C2831559A83, -954, -268), (0x9D71AC8FADA6C9B5, -927, -260),
    (0xEA9C227723EE8BCB, -901, -252), (0xAECC49914078536D, -874, -244), (0x823C12795DB6CE57, -847, -236),
    (0xC21094364DFB5637, -821, -228), (0x9096EA6F3848984F, -794, -220), (0xD77485CB25823AC7, -768, -212),
    (0xA086CFCD97BF97F4, -741, -204), (0xEF340A98172AACE5, -715, -196), (0xB23867FB2A35B28E, -688, -188),
    (0x84C8D4DFD2C63F3B, -661, -180), (0xC5DD44271AD3CDBA, -635, -172), (0x936B9FCEBB25C996, -608, -164),
    (0xDBAC6C247D62A584, -582, -156), (0xA3AB66580D5FDAF6, -555, -148), (0xF3E2F893DEC3F126, -529, -140),
    (0xB5B5ADA8AAFF80B8, -502, -132), (0x87625F056C7C4A8B, -475, -124), (0xC9BCFF6034C13053, -449, -116),
    (0x964E858C91BA2655, -422, -108), (0xDFF9772470297EBD, -396, -100), (0xA6DFBD9FB8E5B88F, -369, -92),
    (0xF8A95FCF88747D94, -343, -84), (0xB94470938FA89BCF, -316, -76), (0x8A08F0F8BF0F156B, -289, -68),
    (0xCDB02555653131B6, -263, -60), (0x993FE2C6D07B7FAC, -236, -52), (0xE45C10C42A2B3B06, -210, -44),
    (0xAA242499697392D3, -183, -36), (0xFD87B5F28300CA0E, -157, -28), (0xBCE5086492111AEB, -130, -20),
    (0x8CBCCC096F5088CC, -103, -12), (0xD1B71758E219652C, -77, -4), (0x9C40000000000000, -50, 4),
    (0xE8D4A51000000000, -24, 12), (0xAD78EBC5AC620000, 3, 20), (0x813F3978F8940984, 30, 28),
    (0xC097CE7BC90715B3, 56, 36), (0x8F7E32CE7BEA5C70, 83, 44), (0xD5D238A4ABE98068, 109, 52),
    (0x9F4F2726179A2245, 136, 60), (0xED63A231D4C4FB27, 162, 68), (0xB0DE65388CC8ADA8, 189, 76),
    (0x83C7088E1AAB65DB, 216, 84), (0xC45D1DF942711D9A, 242, 92), (0x924D692CA61BE758, 269, 100),
    (0xDA01EE641A708DEA, 295, 108), (0xA26DA3999AEF774A, 322, 116), (0xF209787BB47D6B85, 348, 124),
    (0xB454E4A179DD1877, 375, 132), (0x865B86925B9BC5C2, 402, 140), (0xC83553C5C8965D3D, 428, 148),
    (0x952AB45CFA97A0B3, 455, 156), (0xDE469FBD99A05FE3, 481, 164), (0xA59BC234DB398C25, 508, 172),
    (0xF6C69A72A3989F5C, 534, 180), (0xB7DCBF5354E9BECE, 561, 188), (0x88FCF317F22241E2, 588, 196),
    (0xCC20CE9BD35C78A5, 614, 204), (0x98165AF37B2153DF, 641, 212), (0xE2A0B5DC971F303A, 667, 220),
    (0xA8D9D1535CE3B396, 694, 228), (0xFB9B7CD9A4A7443C, 720, 236), (0xBB764C4CA7A44410, 747, 244),
    (0x8BAB8EEFB6409C1A, 774, 252), (0xD01FEF10A657842C, 800, 260), (0x9B10A4E5E9913129, 827, 268),
    (0xE7109BFBA19C0C9D, 853, 276), (0xAC2820D9623BF429, 880, 284), (0x80444B5E7AA7CF85, 907, 292),
    (0xBF21E44003ACDD2D, 933, 300), (0x8E679C2F5E44FF8F, 960, 308), (0xD433179D9C8CB841, 986, 316),
    (0x9E19DB92B4E31BA9, 1013, 324),
];

fn get_cached_power_for_binary_exponent(e: i32) -> (u64, i32, i32) {
    const MIN_DEC_EXP: i32 = -300;
    const DEC_STEP: i32 = 8;
    let f = ALPHA - e - 1;
    let k = (f * 78913) / (1 << 18) + (f > 0) as i32;
    let index = (-MIN_DEC_EXP + k + (DEC_STEP - 1)) / DEC_STEP;
    CACHED_POWERS[index as usize]
}

fn find_largest_pow10(n: u32) -> (i32, u32) {
    let mut pow10 = 1_000_000_000u32;
    let mut k = 10;
    while k > 1 && n < pow10 {
        pow10 /= 10;
        k -= 1;
    }
    (k, pow10)
}

fn grisu2_round(buf: &mut [u8], len: usize, dist: u64, delta: u64, mut rest: u64, ten_k: u64) {
    while rest < dist
        && delta - rest >= ten_k
        && (rest.wrapping_add(ten_k) < dist || dist - rest > rest.wrapping_add(ten_k).wrapping_sub(dist))
    {
        buf[len - 1] -= 1;
        rest = rest.wrapping_add(ten_k);
    }
}

fn grisu2_digit_gen(buffer: &mut [u8], length: &mut usize, decimal_exponent: &mut i32, m_minus: DiyFp, w: DiyFp, m_plus: DiyFp) {
    let mut delta = DiyFp::sub(m_plus, m_minus).f;
    let mut dist = DiyFp::sub(m_plus, w).f;
    let one_e = m_plus.e;
    let one_f = 1u64 << (-one_e);
    let mut p1 = (m_plus.f >> (-one_e)) as u32;
    let mut p2 = m_plus.f & (one_f - 1);

    let (k, mut pow10) = find_largest_pow10(p1);
    let mut n = k;
    while n > 0 {
        let d = p1 / pow10;
        let r = p1 % pow10;
        buffer[*length] = b'0' + d as u8;
        *length += 1;
        p1 = r;
        n -= 1;
        let rest = ((p1 as u64) << (-one_e)) + p2;
        if rest <= delta {
            *decimal_exponent += n;
            let ten_n = (pow10 as u64) << (-one_e);
            grisu2_round(buffer, *length, dist, delta, rest, ten_n);
            return;
        }
        pow10 /= 10;
    }

    let mut m = 0;
    loop {
        p2 = p2.wrapping_mul(10);
        let d = p2 >> (-one_e);
        let r = p2 & (one_f - 1);
        buffer[*length] = b'0' + d as u8;
        *length += 1;
        p2 = r;
        m += 1;
        delta = delta.wrapping_mul(10);
        dist = dist.wrapping_mul(10);
        if p2 <= delta {
            break;
        }
    }
    *decimal_exponent -= m;
    grisu2_round(buffer, *length, dist, delta, p2, one_f);
}

fn grisu2(buf: &mut [u8], len: &mut usize, decimal_exponent: &mut i32, value: f64) {
    let w = compute_boundaries(value);
    let (cf, ce, ck) = get_cached_power_for_binary_exponent(w.plus.e);
    let c_minus_k = DiyFp { f: cf, e: ce };
    let wv = DiyFp::mul(w.w, c_minus_k);
    let w_minus = DiyFp::mul(w.minus, c_minus_k);
    let w_plus = DiyFp::mul(w.plus, c_minus_k);
    let m_minus = DiyFp { f: w_minus.f + 1, e: w_minus.e };
    let m_plus = DiyFp { f: w_plus.f - 1, e: w_plus.e };
    *decimal_exponent = -ck;
    grisu2_digit_gen(buf, len, decimal_exponent, m_minus, wv, m_plus);
}

fn append_exponent(o: &mut Vec<u8>, mut e: i32) {
    if e < 0 {
        e = -e;
        o.push(b'-');
    } else {
        o.push(b'+');
    }
    let k = e as u32;
    if k < 10 {
        o.push(b'0');
        o.push(b'0' + k as u8);
    } else if k < 100 {
        o.push(b'0' + (k / 10) as u8);
        o.push(b'0' + (k % 10) as u8);
    } else {
        o.push(b'0' + (k / 100) as u8);
        o.push(b'0' + (k % 100 / 10) as u8);
        o.push(b'0' + (k % 10) as u8);
    }
}

/// `format_buffer(buf, len, decimal_exponent, -4, 15)`
fn format_buffer(o: &mut Vec<u8>, digits: &[u8], decimal_exponent: i32) {
    const MIN_EXP: i32 = -4;
    const MAX_EXP: i32 = 15; // numeric_limits<double>::digits10
    let k = digits.len() as i32;
    let n = k + decimal_exponent;
    if k <= n && n <= MAX_EXP {
        o.extend_from_slice(digits);
        o.extend(std::iter::repeat(b'0').take((n - k) as usize));
        o.extend_from_slice(b".0");
    } else if 0 < n && n <= MAX_EXP {
        o.extend_from_slice(&digits[..n as usize]);
        o.push(b'.');
        o.extend_from_slice(&digits[n as usize..]);
    } else if MIN_EXP < n && n <= 0 {
        o.extend_from_slice(b"0.");
        o.extend(std::iter::repeat(b'0').take((-n) as usize));
        o.extend_from_slice(digits);
    } else {
        o.push(digits[0]);
        if k > 1 {
            o.push(b'.');
            o.extend_from_slice(&digits[1..]);
        }
        o.push(b'e');
        append_exponent(o, n - 1);
    }
}

fn to_chars(mut value: f64, o: &mut Vec<u8>) {
    if value.is_sign_negative() {
        value = -value;
        o.push(b'-');
    }
    if value == 0.0 {
        o.extend_from_slice(b"0.0");
        return;
    }
    let mut buf = [0u8; 32];
    let mut len = 0usize;
    let mut decimal_exponent = 0;
    grisu2(&mut buf, &mut len, &mut decimal_exponent, value);
    format_buffer(o, &buf[..len], decimal_exponent);
}
