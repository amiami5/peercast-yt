//! atom をバイト列に組み立てる (core/common/atom.h の `AtomStream` の write 系)。
//!
//! C++ 版は 1 つずつ `Stream` に書くが、Rust 版はバイト列にまとめ、呼び出し側が一度に書く。
//! 書き先はソケットか 16KB のパケットなので、書き終えたときの中身は同じになる (途中で書けなく
//! なったときに、それまでに書けていた量は違うことがある)。

use super::atom::Id4;

/// NUL の手前まで (C++ 版の `writeString` は `strlen` で長さを決める)
pub fn c_str(s: &[u8]) -> &[u8] {
    s.iter().position(|&b| b == 0).map_or(s, |i| &s[..i])
}

/// 組み立て中の atom の列
#[derive(Default)]
pub struct AtomBuf(pub Vec<u8>);

impl AtomBuf {
    fn head(&mut self, id: Id4, len: i32) {
        self.0.extend_from_slice(&id);
        self.0.extend_from_slice(&len.to_le_bytes());
    }

    pub fn parent(&mut self, id: Id4, nc: i32) {
        self.0.extend_from_slice(&id);
        self.0.extend_from_slice(&((nc as u32) | 0x8000_0000).to_le_bytes());
    }

    pub fn int(&mut self, id: Id4, d: i32) {
        self.head(id, 4);
        self.0.extend_from_slice(&d.to_le_bytes());
    }

    pub fn short(&mut self, id: Id4, d: i16) {
        self.head(id, 2);
        self.0.extend_from_slice(&d.to_le_bytes());
    }

    pub fn char(&mut self, id: Id4, d: u8) {
        self.head(id, 1);
        self.0.push(d);
    }

    pub fn bytes(&mut self, id: Id4, p: &[u8]) {
        self.head(id, p.len() as i32);
        self.0.extend_from_slice(p);
    }

    /// `writeString`: NUL の手前までと、終わりの NUL
    pub fn string(&mut self, id: Id4, s: &[u8]) {
        let s = c_str(s);
        self.head(id, s.len() as i32 + 1);
        self.0.extend_from_slice(s);
        self.0.push(0);
    }

    /// `writeAddress`: IPv4 (`::ffff:a.b.c.d`) なら int、それ以外は 16 バイトを逆順に
    pub fn address(&mut self, id: Id4, ip: &[u8; 16]) {
        if is_ipv4_mapped(ip) {
            self.int(id, u32::from_be_bytes([ip[12], ip[13], ip[14], ip[15]]) as i32);
        } else {
            let mut r = *ip;
            r.reverse();
            self.bytes(id, &r);
        }
    }
}

/// `IP::isIPv4Mapped`
pub fn is_ipv4_mapped(ip: &[u8; 16]) -> bool {
    ip[..10].iter().all(|&b| b == 0) && ip[10] == 0xff && ip[11] == 0xff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcp::atom::id4;

    #[test]
    fn layout() {
        let mut b = AtomBuf::default();
        b.parent(id4(b"host"), 2);
        b.string(id4(b"name"), b"ab\0cd");
        b.short(id4(b"port"), -2);
        let mut v4 = [0u8; 16];
        v4[10] = 0xff;
        v4[11] = 0xff;
        v4[12..].copy_from_slice(&[1, 2, 3, 4]);
        b.address(id4(b"ip"), &v4);
        let mut v6 = [0u8; 16];
        v6[0] = 0x20;
        v6[15] = 1;
        b.address(id4(b"ip"), &v6);
        let want: Vec<u8> = [
            &b"host"[..], &[2, 0, 0, 0x80],
            b"name", &[3, 0, 0, 0], b"ab\0",
            b"port", &[2, 0, 0, 0], &[0xfe, 0xff],
            b"ip\0\0", &[4, 0, 0, 0], &[4, 3, 2, 1],
            b"ip\0\0", &[16, 0, 0, 0], &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20],
        ]
        .concat();
        assert_eq!(b.0, want);
    }
}
