//! IP アドレスとホスト (core/common/ip.h の `IP`、host.h / host.cpp の `Host`)。
//!
//! IP アドレスは C++ 版と同じく 16 バイトで持ち、IPv4 は `::ffff:a.b.c.d` (IPv4 射影アドレス) にする。

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use super::regex::Regex;
use crate::http::atoi;

/// `IP`
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ip(pub [u8; 16]);

const V4_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];

impl Ip {
    /// `IP(unsigned int)`
    pub fn from_v4(v: u32) -> Ip {
        let mut a = [0u8; 16];
        a[..12].copy_from_slice(&V4_PREFIX);
        a[12..].copy_from_slice(&v.to_be_bytes());
        Ip(a)
    }

    pub fn is_ipv4_mapped(&self) -> bool {
        self.0[..12] == V4_PREFIX
    }

    /// `ipv4()` (IPv4 でなければ `None`。C++ 版は例外)
    pub fn ipv4(&self) -> Option<u32> {
        if self.is_ipv4_mapped() {
            Some(u32::from_be_bytes([self.0[12], self.0[13], self.0[14], self.0[15]]))
        } else {
            None
        }
    }

    /// `ip3()` から `ip0()` (IPv4 の各バイト。3 が先頭)
    pub fn octet(&self, i: usize) -> u8 {
        self.0[15 - i]
    }

    pub fn is_ipv6_link_local(&self) -> bool {
        self.0[0] == 0xfe && (self.0[1] & 0xc0) == 0x80
    }

    pub fn is_ipv6_unique_local(&self) -> bool {
        self.0[0] == 0xfc || self.0[0] == 0xfd
    }

    pub fn is_ipv4_private(&self) -> bool {
        if !self.is_ipv4_mapped() {
            return false;
        }
        let (a, b) = (self.octet(3), self.octet(2));
        a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
    }

    pub fn is_ipv4_loopback(&self) -> bool {
        self.is_ipv4_mapped() && self.octet(3) == 127
    }

    pub fn is_ipv6_loopback(&self) -> bool {
        let mut l = [0u8; 16];
        l[15] = 1;
        self.0 == l
    }

    pub fn is_ipv6_any(&self) -> bool {
        self.0 == [0; 16]
    }

    pub fn is_ipv4_any(&self) -> bool {
        self.is_ipv4_mapped() && self.0[12..] == [0, 0, 0, 0]
    }

    /// `isAny` (`operator bool` の否定)
    pub fn is_any(&self) -> bool {
        self.is_ipv6_any() || self.is_ipv4_any()
    }

    /// `operator bool`
    pub fn is_set(&self) -> bool {
        !self.is_any()
    }

    pub fn is_global(&self) -> bool {
        if self.is_ipv4_mapped() {
            !(self.is_ipv4_loopback() || self.is_ipv4_private())
        } else {
            !(self.is_ipv6_loopback() || self.is_ipv6_link_local() || self.is_ipv6_unique_local())
        }
    }

    /// `str()`。IPv4 は `a.b.c.d`、IPv6 は glibc の `inet_ntop` と同じ形。
    pub fn str(&self) -> String {
        if self.is_ipv4_mapped() {
            format!("{}.{}.{}.{}", self.0[12], self.0[13], self.0[14], self.0[15])
        } else {
            inet_ntop6(&self.0)
        }
    }

    /// `tryParse`: `:` があれば IPv6、なければ IPv4 (`inet_pton`)
    pub fn parse(s: &[u8]) -> Option<Ip> {
        // C++ 版は c_str() を渡すので NUL で切れる
        let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
        if s.contains(&b':') {
            inet_pton6(s).map(Ip)
        } else {
            let mut t = b"::FFFF:".to_vec();
            t.extend_from_slice(s);
            inet_pton6(&t).map(Ip)
        }
    }

    pub fn to_std(&self) -> IpAddr {
        match self.ipv4() {
            Some(v) => IpAddr::V4(Ipv4Addr::from(v)),
            None => IpAddr::V6(Ipv6Addr::from(self.0)),
        }
    }

    pub fn from_std(a: IpAddr) -> Ip {
        match a {
            IpAddr::V4(v) => Ip::from_v4(u32::from(v)),
            IpAddr::V6(v) => Ip(v.octets()),
        }
    }
}

impl fmt::Debug for Ip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.str())
    }
}

/// glibc の `inet_ntop6`
fn inet_ntop6(a: &[u8; 16]) -> String {
    let words: Vec<u16> = (0..8).map(|i| u16::from_be_bytes([a[2 * i], a[2 * i + 1]])).collect();
    // 最も長い 0 の並び (2 語以上)
    let (mut best_base, mut best_len) = (None, 0);
    let (mut cur_base, mut cur_len) = (None, 0);
    for (i, &w) in words.iter().enumerate() {
        if w == 0 {
            if cur_base.is_none() {
                cur_base = Some(i);
                cur_len = 1;
            } else {
                cur_len += 1;
            }
        } else if let Some(b) = cur_base {
            if best_base.is_none() || cur_len > best_len {
                best_base = Some(b);
                best_len = cur_len;
            }
            cur_base = None;
        }
    }
    if let Some(b) = cur_base {
        if best_base.is_none() || cur_len > best_len {
            best_base = Some(b);
            best_len = cur_len;
        }
    }
    if best_base.is_some() && best_len < 2 {
        best_base = None;
    }
    let mut out = String::new();
    let mut i = 0;
    while i < 8 {
        if let Some(b) = best_base {
            if i >= b && i < b + best_len {
                if i == b {
                    out.push(':');
                }
                i += 1;
                continue;
            }
        }
        if i != 0 {
            out.push(':');
        }
        // 埋め込まれた IPv4
        if i == 6 && best_base == Some(0) && (best_len == 6 || (best_len == 5 && words[5] == 0xffff)) {
            out.push_str(&format!("{}.{}.{}.{}", a[12], a[13], a[14], a[15]));
            return out;
        }
        out.push_str(&format!("{:x}", words[i]));
        i += 1;
    }
    if best_base.map_or(false, |b| b + best_len == 8) {
        out.push(':');
    }
    out
}

/// glibc の `inet_pton4` (`src` の全体が a.b.c.d の形)
fn inet_pton4(src: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut octets = 0;
    let mut saw_digit = false;
    let mut cur: u32 = 0;
    for &c in src {
        if c.is_ascii_digit() {
            if saw_digit && cur == 0 {
                return None;
            }
            cur = cur * 10 + (c - b'0') as u32;
            if cur > 255 {
                return None;
            }
            if !saw_digit {
                octets += 1;
                if octets > 4 {
                    return None;
                }
                saw_digit = true;
            }
            out[octets - 1] = cur as u8;
        } else if c == b'.' && saw_digit {
            if octets == 4 {
                return None;
            }
            cur = 0;
            saw_digit = false;
        } else {
            return None;
        }
    }
    if octets < 4 {
        return None;
    }
    Some(out)
}

/// glibc の `inet_pton6`
fn inet_pton6(src: &[u8]) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    let mut tp = 0usize;
    let mut colonp: Option<usize> = None;
    let mut i = 0;
    if src.first() == Some(&b':') {
        if src.get(1) != Some(&b':') {
            return None;
        }
        i = 1;
    }
    let mut curtok = i;
    let mut saw_xdigit = false;
    let mut val: u32 = 0;
    let mut ndigits = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        if let Some(d) = (c as char).to_digit(16) {
            if ndigits == 4 {
                return None;
            }
            val = (val << 4) | d;
            ndigits += 1;
            saw_xdigit = true;
            continue;
        }
        if c == b':' {
            curtok = i;
            if !saw_xdigit {
                if colonp.is_some() {
                    return None;
                }
                colonp = Some(tp);
                continue;
            } else if i >= src.len() {
                return None;
            }
            if tp + 2 > 16 {
                return None;
            }
            out[tp] = (val >> 8) as u8;
            out[tp + 1] = val as u8;
            tp += 2;
            saw_xdigit = false;
            ndigits = 0;
            val = 0;
            continue;
        }
        if c == b'.' && tp + 4 <= 16 {
            let v4 = inet_pton4(&src[curtok..])?;
            out[tp..tp + 4].copy_from_slice(&v4);
            tp += 4;
            saw_xdigit = false;
            break;
        }
        return None;
    }
    if saw_xdigit {
        if tp + 2 > 16 {
            return None;
        }
        out[tp] = (val >> 8) as u8;
        out[tp + 1] = val as u8;
        tp += 2;
    }
    if let Some(cp) = colonp {
        if tp == 16 {
            return None;
        }
        let n = tp - cp;
        for k in 1..=n {
            out[16 - k] = out[cp + n - k];
            out[cp + n - k] = 0;
        }
        tp = 16;
    }
    if tp != 16 {
        return None;
    }
    Some(out)
}

/// `Host`
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Host {
    pub ip: Ip,
    pub port: u16,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.str())
    }
}

impl Host {
    pub fn new(ip: Ip, port: u16) -> Host {
        Host { ip, port }
    }

    /// `Host(unsigned int, unsigned short)`
    pub fn v4(ip: u32, port: u16) -> Host {
        Host { ip: Ip::from_v4(ip), port }
    }

    /// `Host()` (`::ffff:0.0.0.0`、ポート 0)
    pub fn none() -> Host {
        Host { ip: Ip::from_v4(0), port: 0 }
    }

    pub fn global_ip(&self) -> bool {
        self.ip.is_global()
    }

    pub fn local_ip(&self) -> bool {
        !self.global_ip()
    }

    pub fn loopback_ip(&self) -> bool {
        self.ip.is_ipv4_loopback() || self.ip.is_ipv6_loopback()
    }

    /// `isValid`
    pub fn is_valid(&self) -> bool {
        !self.ip.is_any()
    }

    /// `isSameType`
    pub fn is_same_type(&self, h: &Host) -> bool {
        self.global_ip() == h.global_ip()
    }

    /// `str(true)`: `a.b.c.d:port` か `[v6]:port`
    pub fn str(&self) -> String {
        if self.ip.is_ipv4_mapped() {
            format!("{}:{}", self.ip.str(), self.port)
        } else {
            format!("[{}]:{}", self.ip.str(), self.port)
        }
    }

    /// `str(false)`
    pub fn ip_str(&self) -> String {
        self.ip.str()
    }

    /// `isMemberOf`: IPv4 の 255 の欄はどれにでも一致する。パターンが IPv4 でなければ `None`
    /// (C++ 版は例外)。
    pub fn is_member_of(&self, pattern: &Host) -> Option<bool> {
        let p = pattern.ip;
        if !p.is_ipv4_mapped() {
            return None;
        }
        if !self.ip.is_ipv4_mapped() || !p.is_set() {
            return Some(false);
        }
        Some((0..4).all(|i| p.octet(i) == 255 || self.ip.octet(i) == p.octet(i)))
    }

    pub fn to_socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip.to_std(), self.port)
    }

    pub fn from_socket_addr(a: SocketAddr) -> Host {
        // IPv6 のソケットで受けた IPv4 射影アドレスも、C++ 版と同じく 16 バイトの形で持つ
        Host { ip: Ip::from_std(a.ip()), port: a.port() }
    }

    /// `fromStrIP`: `sscanf("%03d.%03d.%03d.%03d:%d")`。読めなければ IP は 0。
    pub fn from_str_ip(s: &[u8], default_port: u16) -> Host {
        let s = until_nul(s);
        if s.contains(&b':') {
            let mut sc = Scanf { s, i: 0 };
            let v = (|| {
                let a = sc.int(3)?;
                sc.lit(b'.')?;
                let b = sc.int(3)?;
                sc.lit(b'.')?;
                let c = sc.int(3)?;
                sc.lit(b'.')?;
                let d = sc.int(3)?;
                sc.lit(b':')?;
                let p = sc.int(usize::MAX)?;
                Some((a, b, c, d, p))
            })();
            match v {
                Some((a, b, c, d, p)) => Host::v4(ipb(a, b, c, d), p as u16),
                None => Host::v4(0, 0),
            }
        } else {
            let mut sc = Scanf { s, i: 0 };
            let v = (|| {
                let a = sc.int(3)?;
                sc.lit(b'.')?;
                let b = sc.int(3)?;
                sc.lit(b'.')?;
                let c = sc.int(3)?;
                sc.lit(b'.')?;
                let d = sc.int(3)?;
                Some((a, b, c, d))
            })();
            match v {
                Some((a, b, c, d)) => Host::v4(ipb(a, b, c, d), default_port),
                None => Host::v4(0, default_port),
            }
        }
    }

    /// `fromStrName`: IPv4 か IPv6 の文字列 (ポート付きでも)、なければホスト名を引く。
    pub fn from_str_name(s: &[u8], default_port: u16) -> Host {
        let s = until_nul(s);
        if s.is_empty() {
            return Host::v4(0, 0);
        }
        let p = patterns();
        if p.ipv4.is_match(s) {
            return Host::new(Ip::parse(s).unwrap_or_else(|| Ip::from_v4(0)), default_port);
        }
        if p.ipv4_port.is_match(s) {
            let v = crate::strutil::split(s, b":");
            return Host::new(Ip::parse(&v[0]).unwrap_or_else(|| Ip::from_v4(0)), atoi(&v[1]) as u16);
        }
        if p.ipv6.is_match(s) {
            return Host::new(Ip::parse(s).unwrap_or_default(), default_port);
        }
        if p.ipv6_bracketed.is_match(s) {
            return Host::new(Ip::parse(&s[1..s.len() - 1]).unwrap_or_default(), default_port);
        }
        if p.ipv6_bracketed_port.is_match(s) {
            let q = s.iter().position(|&c| c == b']').unwrap_or(0);
            return Host::new(Ip::parse(&s[1..q]).unwrap_or_default(), atoi(&s[q + 2..]) as u16);
        }
        let v = crate::strutil::split(s, b":");
        if v.len() <= 2 {
            let ip = Ip::parse(&v[0]).unwrap_or_else(|| resolve_first(&v[0]).unwrap_or_default());
            let port = if v.len() == 2 { atoi(&v[1]) as u16 } else { default_port };
            Host::new(ip, port)
        } else {
            crate::log_error!("fromStrName: Parse error: {}", String::from_utf8_lossy(s));
            Host::v4(0, 0)
        }
    }

    /// `Host::fromString`: `[v6]:port` か `名前:port` (名前は IPv4 で引く)
    pub fn from_string(s: &[u8], default_port: u16) -> Host {
        let s = until_nul(s);
        if s.first() == Some(&b'[') {
            let mut ip = Ip::default();
            let mut port = default_port;
            if let Some(q) = s.iter().position(|&c| c == b']') {
                ip = Ip::parse(&s[1..q]).unwrap_or_default();
                if s.get(q + 1) == Some(&b':') {
                    port = atoi(&s[q + 2..]) as u16;
                }
            }
            Host::new(ip, port)
        } else {
            match s.iter().position(|&c| c == b':') {
                Some(c) => Host::new(Ip::from_v4(get_ip(&s[..c])), atoi(&s[c + 1..]) as u16),
                None => Host::new(Ip::from_v4(get_ip(s)), default_port),
            }
        }
    }
}

fn ipb(a: i64, b: i64, c: i64, d: i64) -> u32 {
    (((a as u32) & 0xff) << 24) | (((b as u32) & 0xff) << 16) | (((c as u32) & 0xff) << 8) | ((d as u32) & 0xff)
}

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `sscanf` の `%d` (幅つき) と文字の読み取り
struct Scanf<'a> {
    s: &'a [u8],
    i: usize,
}

impl Scanf<'_> {
    fn int(&mut self, width: usize) -> Option<i64> {
        while self.s.get(self.i).map_or(false, |c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
        let start = self.i;
        let mut w = 0;
        let mut neg = false;
        if w < width && matches!(self.s.get(self.i), Some(b'+' | b'-')) {
            neg = self.s[self.i] == b'-';
            self.i += 1;
            w += 1;
        }
        let digits_start = self.i;
        let mut v: i64 = 0;
        while w < width && self.s.get(self.i).map_or(false, |c| c.is_ascii_digit()) {
            v = v.saturating_mul(10).saturating_add((self.s[self.i] - b'0') as i64);
            self.i += 1;
            w += 1;
        }
        if self.i == digits_start {
            self.i = start;
            return None;
        }
        // %d は int に入る (桁あふれは未定義。glibc は下位の 32 ビット)
        Some(if neg { (-v) as i32 as i64 } else { v as i32 as i64 })
    }

    fn lit(&mut self, c: u8) -> Option<()> {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }
}

struct Patterns {
    ipv4: Regex,
    ipv4_port: Regex,
    ipv6: Regex,
    ipv6_bracketed: Regex,
    ipv6_bracketed_port: Regex,
}

/// host.cpp と servfilter.cpp の IPv6 の正規表現
pub const IPV6_PATTERN: &str = "(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|[fF][eE]80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::([fF][fF][fF][fF](:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))";

fn patterns() -> &'static Patterns {
    static P: std::sync::OnceLock<Patterns> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let re = |s: String| Regex::new(s.as_bytes()).expect("正規表現");
        Patterns {
            ipv4: re("^\\d+\\.\\d+\\.\\d+\\.\\d+$".into()),
            ipv4_port: re("^\\d+\\.\\d+\\.\\d+\\.\\d+:\\d+$".into()),
            ipv6: re(format!("^{}$", IPV6_PATTERN)),
            ipv6_bracketed: re(format!("^\\[{}\\]$", IPV6_PATTERN)),
            ipv6_bracketed_port: re(format!("^\\[{}\\]:\\d+$", IPV6_PATTERN)),
        }
    })
}

/// 名前を引いた最初のアドレス (`Sys::getIPAddresses` の先頭。IPv4 と IPv6 の両方)
pub fn resolve_first(name: &[u8]) -> Option<Ip> {
    resolve_all(name).into_iter().next()
}

/// `USys::getIPAddresses`
pub fn resolve_all(name: &[u8]) -> Vec<Ip> {
    let name = match std::str::from_utf8(until_nul(name)) {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };
    match (name, 0u16).to_socket_addrs() {
        Ok(it) => {
            let mut v: Vec<Ip> = Vec::new();
            for a in it {
                let ip = Ip::from_std(a.ip());
                if !v.contains(&ip) {
                    v.push(ip);
                }
            }
            v
        }
        Err(_) => Vec::new(),
    }
}

/// `ClientSocket::getIP`: 名前の IPv4 アドレス (`gethostbyname` の先頭)。引けなければ 0。
pub fn get_ip(name: &[u8]) -> u32 {
    resolve_all(name).into_iter().find_map(|ip| ip.ipv4()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Option<String> {
        Ip::parse(s.as_bytes()).map(|i| i.str())
    }

    #[test]
    fn ip_strings() {
        assert_eq!(p("127.0.0.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(p("::1").as_deref(), Some("::1"));
        assert_eq!(p("::").as_deref(), Some("::"));
        assert_eq!(p("2001:db8:0:0:1:0:0:1").as_deref(), Some("2001:db8::1:0:0:1"));
        assert_eq!(p("2001:DB8::").as_deref(), Some("2001:db8::"));
        assert_eq!(p("::ffff:1.2.3.4").as_deref(), Some("1.2.3.4"));
        assert_eq!(p("::1.2.3.4").as_deref(), Some("::1.2.3.4"));
        assert_eq!(p("1:0:1:0:1:0:1:0").as_deref(), Some("1:0:1:0:1:0:1:0"));
        assert_eq!(p("01.2.3.4"), None);
        assert_eq!(p("256.2.3.4"), None);
        assert_eq!(p("1.2.3"), None);
        assert_eq!(p(":::"), None);
        assert_eq!(p("1::2::3"), None);
        assert_eq!(p("12345::"), None);
    }

    #[test]
    fn hosts() {
        let h = Host::from_str_ip(b"192.168.0.1:7144", 0);
        assert_eq!(h.str(), "192.168.0.1:7144");
        assert!(h.local_ip());
        assert_eq!(Host::from_str_ip(b"1.2.3.4", 80).str(), "1.2.3.4:80");
        assert_eq!(Host::from_str_ip(b"1234.2.3.4", 80).str(), "0.0.0.0:80");
        assert_eq!(Host::from_str_ip(b"x", 80).str(), "0.0.0.0:80");
        assert_eq!(Host::from_str_name(b"[::1]:8144", 7144).str(), "[::1]:8144");
        assert_eq!(Host::from_str_name(b"::1", 7144).str(), "[::1]:7144");
        assert_eq!(Host::from_str_name(b"10.0.0.1", 7144).str(), "10.0.0.1:7144");
        assert_eq!(Host::from_str_name(b"10.0.0.1:1", 7144).str(), "10.0.0.1:1");
        assert_eq!(Host::from_str_name(b"", 7144).str(), "0.0.0.0:0");
        assert_eq!(Host::from_string(b"[::1]:80", 7144).str(), "[::1]:80");
        assert_eq!(Host::from_string(b"1.2.3.4:5", 7144).str(), "1.2.3.4:5");
        let pat = Host::from_str_ip(b"192.168.255.255", 0);
        assert_eq!(Host::from_str_ip(b"192.168.3.4", 0).is_member_of(&pat), Some(true));
        assert_eq!(Host::from_str_ip(b"192.169.3.4", 0).is_member_of(&pat), Some(false));
    }
}
