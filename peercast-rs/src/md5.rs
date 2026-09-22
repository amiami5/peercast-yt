//! MD5 (RFC 1321)。C++ 版 core/common/md5.cpp (Solar Designer によるパブリックドメイン実装) の
//! 移植。`md5::hexdigest` だけが呼び出し元 (core/common/chanmgr.cpp、リレー可否を決める
//! 認証トークンの生成) から使われているので、それだけを実装する。
//!
//! MD5 は既に暗号学的に破られており、新しい用途には使うべきではない。ここでは
//! 「C++ 版と同じ出力を返す」ことだけを目的にしていて、暗号学的な強度を保証するものではない。

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

// floor(abs(sin(i+1)) * 2^32) の整数部。RFC 1321 の表そのもの。
const K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

fn transform(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_le_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
    }

    let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | (!b & d), i),
            16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(f.rotate_left(S[i]));
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

/// MD5 ダイジェスト (16 バイト)。
pub fn digest(input: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for block in msg.chunks_exact(64) {
        let b: [u8; 64] = block.try_into().unwrap();
        transform(&mut state, &b);
    }

    let mut out = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// 小文字16進数のダイジェスト文字列 (32文字)。C++版の `md5::hexdigest` に相当する。
pub fn hexdigest(input: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let d = digest(input);
    let mut out = Vec::with_capacity(32);
    for b in d {
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 0xf) as usize]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &[u8]) -> String {
        String::from_utf8(hexdigest(s)).unwrap()
    }

    // ---- RFC 1321 のテストベクタ ----
    #[test]
    fn rfc1321_test_vectors() {
        assert_eq!(hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(b"message digest"), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(hex(b"abcdefghijklmnopqrstuvwxyz"), "c3fcd3d76192e4007dfb496cca67e13b");
        assert_eq!(
            hex(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        assert_eq!(
            hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    // ---- C++ 版の単体テスト (tests/md5_unittest.cpp, tests/chanmgr_unittest.cpp) を移したもの ----
    #[test]
    fn cpp_test_cases() {
        assert_eq!(hex(b"hello"), "5d41402abc4b2a76b9719d911017c592");
        assert_eq!(
            hex(b"00151515151515151515151515151515:01234567890123456789012345678901"),
            "44d5299e57ad9274fee7960a9fa60bfd"
        );
    }

    // 64バイト境界をまたぐ入力 (パディングの分岐を確かめる)
    #[test]
    fn boundary_lengths() {
        for len in [55, 56, 57, 63, 64, 65, 119, 120, 121] {
            let input = vec![b'x'; len];
            // 参照実装を持たないので、ここでは panic しないことと、長さが常に32文字になることだけ確認する。
            assert_eq!(hexdigest(&input).len(), 32);
        }
    }

    #[test]
    fn never_panics() {
        let mut x: u64 = 0xa076_1d64_78bd_642f;
        for _ in 0..2000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let len = (x % 300) as usize;
            let input: Vec<u8> = (0..len).map(|i| ((x >> (i % 32)) ^ i as u64) as u8).collect();
            assert_eq!(hexdigest(&input).len(), 32);
        }
    }
}
