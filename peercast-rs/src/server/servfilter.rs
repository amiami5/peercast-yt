//! 接続を許す・禁じるホストの決まり (core/common/servfilter.cpp の `ServFilter`)。
//!
//! パターンは IPv4 アドレス (255 の欄はどれにでも一致する)、IPv6 アドレス、ネットマスク付きの
//! アドレス、ホスト名、`.` で始まる名前の終わり (逆引きした名前と比べる)。

use std::sync::OnceLock;

use super::host::{get_ip, Host, Ip, IPV6_PATTERN};
use super::regex::Regex;
use super::state::{flag, obj, s, Value};

pub const F_PRIVATE: u32 = 0x01;
pub const F_BAN: u32 = 0x02;
pub const F_NETWORK: u32 = 0x04;
pub const F_DIRECT: u32 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Ip,
    Hostname,
    Suffix,
    Ipv6,
    Ipv4WithNetmask,
    Ipv6WithNetmask,
}

/// `ServFilter`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServFilter {
    pub ty: Type,
    pub flags: u32,
    host: Host,
    pattern: Vec<u8>,
    netmask: i32,
}

impl Default for ServFilter {
    fn default() -> Self {
        ServFilter { ty: Type::Ip, flags: 0, host: Host::none(), pattern: Vec::new(), netmask: -1 }
    }
}

struct Patterns {
    ipv4: Regex,
    ipv6: Regex,
    ipv4_mask: Regex,
    ipv6_mask: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let re = |s: String| Regex::new(s.as_bytes()).expect("正規表現");
        Patterns {
            ipv4: re("^\\d+\\.\\d+\\.\\d+\\.\\d+$".into()),
            ipv6: re(format!("^{}$", IPV6_PATTERN)),
            ipv4_mask: re("^\\d+\\.\\d+\\.\\d+\\.\\d+/\\d+$".into()),
            ipv6_mask: re(format!("^{}/\\d+$", IPV6_PATTERN)),
        }
    })
}

impl ServFilter {
    /// `init`
    pub fn init(&mut self) {
        *self = ServFilter::default();
    }

    /// `setPattern`
    pub fn set_pattern(&mut self, p: &[u8]) {
        let p = &p[..p.iter().position(|&c| c == 0).unwrap_or(p.len())];
        let pat = patterns();
        if p.is_empty() {
            self.init();
        } else if p[0] == b'.' {
            self.ty = Type::Suffix;
            self.pattern = p.to_vec();
        } else if pat.ipv4.is_match(p) {
            self.ty = Type::Ip;
            self.host = Host::from_str_ip(p, 0);
        } else if pat.ipv6.is_match(p) {
            self.ty = Type::Ipv6;
            self.host = Host::new(Ip::parse(p).unwrap_or_default(), 0);
        } else if pat.ipv4_mask.is_match(p) {
            self.ty = Type::Ipv4WithNetmask;
            let v = crate::strutil::split_limit(p, b"/", 2).unwrap_or_default();
            self.host = Host::from_str_ip(&v[0], 0);
            self.netmask = crate::http::atoi(&v[1]).min(32);
        } else if pat.ipv6_mask.is_match(p) {
            self.ty = Type::Ipv6WithNetmask;
            let v = crate::strutil::split_limit(p, b"/", 2).unwrap_or_default();
            self.host = Host::new(Ip::parse(&v[0]).unwrap_or_default(), 0);
            self.netmask = crate::http::atoi(&v[1]).min(128);
        } else {
            self.ty = Type::Hostname;
            self.pattern = p.to_vec();
        }
    }

    /// `getPattern`
    pub fn pattern(&self) -> Vec<u8> {
        match self.ty {
            Type::Ip => self.host.ip_str().into_bytes(),
            Type::Ipv6 => self.host.ip.str().into_bytes(),
            Type::Hostname | Type::Suffix => self.pattern.clone(),
            Type::Ipv4WithNetmask | Type::Ipv6WithNetmask => format!("{}/{}", self.host.ip.str(), self.netmask).into_bytes(),
        }
    }

    /// `matches`
    pub fn matches(&self, fl: u32, h: &Host) -> bool {
        if self.flags & fl == 0 {
            return false;
        }
        match self.ty {
            Type::Ip => h.is_member_of(&self.host).unwrap_or(false),
            Type::Ipv6 => self.host.ip == h.ip,
            Type::Hostname => h.ip == Ip::from_v4(get_ip(&self.pattern)),
            Type::Suffix => match super::sys::hostname_by_address(&h.ip) {
                Some(name) => name.ends_with(&self.pattern),
                None => false,
            },
            Type::Ipv4WithNetmask => {
                let (a, b) = match (self.host.ip.ipv4(), h.ip.ipv4()) {
                    (Some(a), Some(b)) => (a, b),
                    _ => return false,
                };
                // C++ 版は 32 - netmask だけずらすので、/0 は 32 ビットずらす未定義の動作になり、
                // x86 ではずらさない (0.0.0.0/0 が 0.0.0.0 にしか一致しない)。Rust 版は /0 を
                // すべてのアドレスに一致させる (docs/cpp-known-issues.md)
                let mask = if self.netmask <= 0 { 0 } else { u32::MAX << (32 - self.netmask) as u32 };
                (a & mask) == (b & mask)
            }
            Type::Ipv6WithNetmask => {
                if h.ip.is_ipv4_mapped() {
                    return false;
                }
                let (a1, a2) = (&self.host.ip.0, &h.ip.0);
                let mut n = self.netmask;
                for i in 0..16 {
                    if n >= 8 {
                        if a1[i] != a2[i] {
                            return false;
                        }
                        n -= 8;
                    } else {
                        let mask = (0xffu32 << (8 - n)) as u8;
                        if (a1[i] & mask) != (a2[i] & mask) {
                            return false;
                        }
                        n = 0;
                    }
                    if n <= 0 {
                        break;
                    }
                }
                true
            }
        }
    }

    /// `isGlobal`
    pub fn is_global(&self) -> bool {
        let p = self.pattern();
        p == b"255.255.255.255" || p == b"0.0.0.0/0" || p == b"::/0"
    }

    /// `isSet`
    pub fn is_set(&self) -> bool {
        !(self.ty == Type::Ip && !self.host.ip.is_set())
    }

    /// `getState`
    pub fn state(&self) -> Value {
        obj(vec![
            ("network", flag(self.flags & F_NETWORK != 0)),
            ("private", flag(self.flags & F_PRIVATE != 0)),
            ("direct", flag(self.flags & F_DIRECT != 0)),
            ("banned", flag(self.flags & F_BAN != 0)),
            ("ip", s(self.pattern())),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, flags: u32) -> ServFilter {
        let mut x = ServFilter::default();
        x.set_pattern(p.as_bytes());
        x.flags = flags;
        x
    }

    #[test]
    fn patterns_and_matches() {
        let h = |s: &str| Host::from_str_name(s.as_bytes(), 0);
        let a = f("192.168.255.255", F_BAN);
        assert_eq!(a.pattern(), b"192.168.255.255");
        assert!(a.matches(F_BAN, &h("192.168.1.2")));
        assert!(!a.matches(F_NETWORK, &h("192.168.1.2")));
        let m = f("10.0.0.0/8", F_DIRECT);
        assert_eq!(m.pattern(), b"10.0.0.0/8");
        assert!(m.matches(F_DIRECT, &h("10.2.3.4")));
        assert!(!m.matches(F_DIRECT, &h("11.2.3.4")));
        assert!(f("0.0.0.0/0", F_DIRECT).matches(F_DIRECT, &h("11.2.3.4")));
        let v6 = f("::/0", F_NETWORK);
        assert!(v6.is_global());
        assert!(v6.matches(F_NETWORK, &h("::1")));
        assert!(!v6.matches(F_NETWORK, &h("1.2.3.4")));
        let v6m = f("2001:db8::/32", F_NETWORK);
        assert!(v6m.matches(F_NETWORK, &h("2001:db8:1::1")));
        assert!(!v6m.matches(F_NETWORK, &h("2001:db9::1")));
        assert_eq!(f(".example.jp", 0).ty, Type::Suffix);
        assert_eq!(f("localhost", 0).ty, Type::Hostname);
        let mut e = f("1.2.3.4", F_BAN);
        e.set_pattern(b"");
        assert!(!e.is_set());
    }
}
