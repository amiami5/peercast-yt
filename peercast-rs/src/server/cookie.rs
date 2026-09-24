//! 管理画面のログインのトークン (core/common/cookie.cpp の `Cookie` と `CookieList`)。

use std::collections::VecDeque;

use super::host::Ip;
use super::state::{arr, obj, s, Value};

/// `Cookie`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cookie {
    pub ip: Ip,
    /// 63 バイトまで
    pub id: Vec<u8>,
}

impl Cookie {
    /// `set`
    pub fn new(id: &[u8], ip: Ip) -> Cookie {
        let id = &id[..id.iter().position(|&c| c == 0).unwrap_or(id.len())];
        Cookie { ip, id: id[..id.len().min(63)].to_vec() }
    }

    /// `Cookie()` (`clear`): IP は `IP(0)`
    pub fn empty() -> Cookie {
        Cookie { ip: Ip::from_v4(0), id: Vec::new() }
    }
}

/// `CookieList`: 最大 32 個
#[derive(Clone, Debug, Default)]
pub struct CookieList {
    pub list: VecDeque<Cookie>,
    pub never_expire: bool,
}

const MAX_COOKIES: usize = 32;

impl CookieList {
    /// `init`
    pub fn init(&mut self) {
        self.list.clear();
        self.never_expire = false;
    }

    /// `contains`: ID と IP が空でなく、同じものがあるか
    pub fn contains(&self, c: &Cookie) -> bool {
        !c.id.is_empty() && c.ip.is_set() && self.list.contains(c)
    }

    /// `add`: 同じものがなければ加える (古いものから消す)
    pub fn add(&mut self, c: Cookie) -> bool {
        if self.contains(&c) {
            return false;
        }
        self.list.push_back(c);
        while self.list.len() > MAX_COOKIES {
            self.list.pop_front();
        }
        true
    }

    /// `remove`
    pub fn remove(&mut self, c: &Cookie) {
        self.list.retain(|x| x != c);
    }

    /// `getState`
    pub fn state(&self) -> Value {
        arr(self.list.iter().map(|c| obj(vec![("ip", s(c.ip.str())), ("id", s(&c.id))])).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list() {
        let mut l = CookieList::default();
        let ip = Ip::from_v4(0x7f000001);
        assert!(l.add(Cookie::new(b"abc", ip)));
        assert!(!l.add(Cookie::new(b"abc", ip)));
        assert!(l.contains(&Cookie::new(b"abc", ip)));
        assert!(!l.contains(&Cookie::new(b"", ip)));
        for i in 0..40 {
            l.add(Cookie::new(format!("{}", i).as_bytes(), ip));
        }
        assert_eq!(l.list.len(), 32);
        assert!(!l.contains(&Cookie::new(b"abc", ip)));
        assert_eq!(Cookie::new(&[b'x'; 100], ip).id.len(), 63);
    }
}
