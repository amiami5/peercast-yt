//! GnuID (16バイトのチャンネル/セッション識別子) の純粋な部分。
//! C++ 版 core/common/gnuid.cpp のうち、`sys->rnd()` や `Host` の実体に依存しない
//! `toStr`/`fromStr`/`encode` に相当する。乱数生成 (`generate`/`random`) と、
//! `GnuIDList` (直近使用時刻つきのキャッシュ) は状態を持つクラスなので、この段階では
//! 移植の対象にしない。

/// 大文字16進数32文字にする。
pub fn to_str(id: &[u8; 16]) -> [u8; 32] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 32];
    for (i, &b) in id.iter().enumerate() {
        out[i * 2] = HEX[(b >> 4) as usize];
        out[i * 2 + 1] = HEX[(b & 0xf) as usize];
    }
    out
}

/// 16進数文字列から16バイトを読む。32文字未満なら全ゼロ (C++版の `fromStr` と同じ:
/// `clear()` した後、長さが足りなければ何もしない)。32文字以上あれば、33文字目以降は無視する。
/// 各バイトは `strtoul(buf, nullptr, 16)` 相当 (2文字ずつ、16進数として不正な文字は
/// その桁までで打ち切り、非16進文字が続く場合は0になる。C の `strtoul` の緩い構文に合わせている)。
pub fn from_str(s: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    if s.len() < 32 {
        return out;
    }
    for i in 0..16 {
        out[i] = parse_hex_byte_loose(s[i * 2], s[i * 2 + 1]);
    }
    out
}

/// `strtoul("XY", NULL, 16)` を1バイトに切り詰めたもの相当。C++ 版は2文字を渡すだけなので、
/// 1文字目が16進数でなければ0、2文字目が16進数でなければ1文字目だけの値になる。
fn parse_hex_byte_loose(c1: u8, c2: u8) -> u8 {
    let d1 = (c1 as char).to_digit(16);
    let d2 = (c2 as char).to_digit(16);
    match (d1, d2) {
        (Some(a), Some(b)) => ((a << 4) | b) as u8,
        (Some(a), None) => a as u8,
        (None, _) => 0,
    }
}

/// IP アドレスとソルトで撹拌する。C++ 版 `GnuID::encode` と同じ。
/// `ip_bytes` は4バイト (ホストのIPアドレスを表す32ビット整数のバイト表現)。`None` なら
/// IPアドレスによる撹拌をしない (C++版で `Host* h` が `nullptr` の場合に相当)。
pub fn encode(id: &mut [u8; 16], ip_bytes: Option<[u8; 4]>, salt1: &[u8], salt2: &[u8], salt3: u8) {
    for (i, b) in id.iter_mut().enumerate() {
        let mut v = *b;
        if let Some(ip) = ip_bytes {
            v ^= ip[i & 3];
        }
        v ^= salt3;
        *b = v;
    }

    let n = (salt1.len().max(salt2.len()) / 16 + 1) * 16;
    let mut s1 = 0usize;
    let mut s2 = 0usize;
    for i in 0..n {
        let mut v = id[i % 16];
        if !salt1.is_empty() {
            if s1 < salt1.len() && salt1[s1] != 0 {
                v ^= salt1[s1];
                s1 += 1;
            } else {
                s1 = 0;
            }
        }
        if !salt2.is_empty() {
            if s2 < salt2.len() && salt2[s2] != 0 {
                v ^= salt2[s2];
                s2 += 1;
            } else {
                s2 = 0;
            }
        }
        id[i % 16] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_str_from_str_roundtrip() {
        let id: [u8; 16] = [0x00, 0x01, 0x0a, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0];
        let s = to_str(&id);
        assert_eq!(&s, b"00010AFF102030405060708090A0B0C0".as_slice()); // 16進数32文字
        assert_eq!(from_str(&s), id);
    }

    #[test]
    fn from_str_short_input_is_all_zero() {
        assert_eq!(from_str(b""), [0u8; 16]);
        assert_eq!(from_str(b"0123456789ABCDEF"), [0u8; 16]); // 16文字 (足りない)
    }

    #[test]
    fn from_str_lowercase_and_extra_chars() {
        let s = b"0123456789abcdef0123456789ABCDEFxxxx"; // 37文字、末尾は無視される
        let want: [u8; 16] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        assert_eq!(from_str(s), want);
    }

    #[test]
    fn encode_without_ip() {
        let mut id = [0u8; 16];
        encode(&mut id, None, b"", b"", 0);
        assert_eq!(id, [0u8; 16]); // ソルトなし・IPなしなら変化しない
    }

    #[test]
    fn encode_with_salt3_only() {
        let mut id = [0u8; 16];
        encode(&mut id, None, b"", b"", 0x42);
        assert_eq!(id, [0x42u8; 16]);
    }

    #[test]
    fn encode_with_ip() {
        let mut id = [0u8; 16];
        encode(&mut id, Some([1, 2, 3, 4]), b"", b"", 0);
        // 16バイトに [1,2,3,4] を4回繰り返して XOR
        assert_eq!(id, [1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
    }

    #[test]
    fn encode_never_panics() {
        let mut x: u64 = 0x1234_5678_9abc_def0;
        for _ in 0..5000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let mut id = [0u8; 16];
            for b in id.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = x as u8;
            }
            let ip = if x % 2 == 0 { Some([x as u8, (x >> 8) as u8, (x >> 16) as u8, (x >> 24) as u8]) } else { None };
            let salt1: Vec<u8> = (0..(x % 40)).map(|i| (x >> (i % 32)) as u8).collect();
            let salt2: Vec<u8> = (0..(x % 40)).map(|i| (x.wrapping_mul(3) >> (i % 32)) as u8).collect();
            encode(&mut id, ip, &salt1, &salt2, x as u8);
        }
    }
}
