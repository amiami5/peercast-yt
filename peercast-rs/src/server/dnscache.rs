//! フィルターの判定で使う名前解決 (正引き・逆引き) の結果を覚えておく。
//!
//! 接続の処理が DNS の応答を長く待たないよう、覚えたものは古くてもすぐ返して別のスレッドで引き直し、
//! まだ覚えていないものだけ決まった時間まで待つ。間に合わなければ「わからない」(`None`) を返す。
//! 逆引きの応答は相手の側の DNS サーバーが返すので、相手がわざと遅くできる。

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::host::{get_ip, Ip};

/// 覚えたものを引き直すまでの時間
const TTL: Duration = Duration::from_secs(60);
/// まだ覚えていないものを待つ時間
const WAIT: Duration = Duration::from_secs(2);
/// 覚えておく数の上限
const MAX_ENTRIES: usize = 1024;
/// 同時に走らせる問い合わせの数の上限
const MAX_RESOLVING: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    /// 名前の IPv4 アドレス (正引き)
    Name(Vec<u8>),
    /// アドレスの名前 (逆引き)
    Addr(Ip),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// 引けなければ 0
    Ip(u32),
    Name(Option<Vec<u8>>),
}

struct Entry {
    answer: Option<Answer>,
    fetched: Instant,
    resolving: bool,
}

#[derive(Default)]
struct State {
    m: HashMap<Query, Entry>,
    resolving: usize,
}

pub struct DnsCache {
    st: Mutex<State>,
    cv: Condvar,
    resolve: fn(&Query) -> Answer,
    ttl: Duration,
    wait: Duration,
}

impl DnsCache {
    pub fn new(resolve: fn(&Query) -> Answer, ttl: Duration, wait: Duration) -> DnsCache {
        DnsCache { st: Mutex::new(State::default()), cv: Condvar::new(), resolve, ttl, wait }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 覚えている答えを (古くても) 返す。覚えていなければ `wait` まで待ち、間に合わなければ `None`
    pub fn lookup(&'static self, q: &Query) -> Option<Answer> {
        let mut st = self.state();
        self.start(&mut st, q);
        let deadline = Instant::now() + self.wait;
        loop {
            match st.m.get(q) {
                Some(Entry { answer: Some(a), .. }) => return Some(a.clone()),
                Some(e) if e.resolving => {}
                _ => return None,
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            st = self.cv.wait_timeout(st, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// 答えがないか古ければ、待たずに別のスレッドで引き始める
    pub fn prefetch(&'static self, q: &Query) {
        let mut st = self.state();
        self.start(&mut st, q);
    }

    fn start(&'static self, st: &mut State, q: &Query) {
        let now = Instant::now();
        if let Some(e) = st.m.get(q) {
            if e.resolving || (e.answer.is_some() && now.duration_since(e.fetched) < self.ttl) {
                return;
            }
        }
        if st.resolving >= MAX_RESOLVING {
            return;
        }
        if !st.m.contains_key(q) && st.m.len() >= MAX_ENTRIES {
            let ttl = self.ttl;
            st.m.retain(|_, e| e.resolving || now.duration_since(e.fetched) < ttl);
            if st.m.len() >= MAX_ENTRIES {
                return;
            }
        }
        let q2 = q.clone();
        // 引き終えたスレッドは、呼び出し元がロックを放すまで結果を書けない
        let spawned = std::thread::Builder::new().name("DNS".into()).spawn(move || {
            let a = (self.resolve)(&q2);
            let mut st = self.state();
            st.resolving -= 1;
            if let Some(e) = st.m.get_mut(&q2) {
                e.answer = Some(a);
                e.fetched = Instant::now();
                e.resolving = false;
            }
            drop(st);
            self.cv.notify_all();
        });
        if spawned.is_ok() {
            st.resolving += 1;
            st.m.entry(q.clone()).or_insert(Entry { answer: None, fetched: now, resolving: false }).resolving = true;
        }
    }
}

fn resolve(q: &Query) -> Answer {
    match q {
        Query::Name(n) => Answer::Ip(get_ip(n)),
        Query::Addr(ip) => Answer::Name(super::sys::hostname_by_address(ip)),
    }
}

fn global() -> &'static DnsCache {
    static C: OnceLock<DnsCache> = OnceLock::new();
    C.get_or_init(|| DnsCache::new(resolve, TTL, WAIT))
}

/// 名前の IPv4 アドレス。引けないか間に合わなければ 0
pub fn ip_of(name: &[u8]) -> u32 {
    match global().lookup(&Query::Name(name.to_vec())) {
        Some(Answer::Ip(ip)) => ip,
        _ => 0,
    }
}

/// アドレスの名前 (逆引き)。引けないか間に合わなければ `None`
pub fn name_of(ip: &Ip) -> Option<Vec<u8>> {
    match global().lookup(&Query::Addr(*ip)) {
        Some(Answer::Name(n)) => n,
        _ => None,
    }
}

/// `ip_of` の答えを前もって引き始める
pub fn prefetch_name(name: &[u8]) {
    global().prefetch(&Query::Name(name.to_vec()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn leak(resolve: fn(&Query) -> Answer, ttl_ms: u64, wait_ms: u64) -> &'static DnsCache {
        Box::leak(Box::new(DnsCache::new(resolve, Duration::from_millis(ttl_ms), Duration::from_millis(wait_ms))))
    }

    fn name(s: &str) -> Query {
        Query::Name(s.as_bytes().to_vec())
    }

    #[test]
    fn remembers_answers() {
        static N: AtomicUsize = AtomicUsize::new(0);
        fn r(q: &Query) -> Answer {
            N.fetch_add(1, Ordering::SeqCst);
            match q {
                Query::Name(n) => Answer::Ip(n.len() as u32),
                Query::Addr(_) => Answer::Name(None),
            }
        }
        let c = leak(r, 60_000, 2_000);
        assert_eq!(c.lookup(&name("abc")), Some(Answer::Ip(3)));
        assert_eq!(c.lookup(&name("abc")), Some(Answer::Ip(3)));
        assert_eq!(N.load(Ordering::SeqCst), 1);
        // 引けなかったことも覚える
        let a = Query::Addr(Ip::from_v4(0x0a00_0001));
        assert_eq!(c.lookup(&a), Some(Answer::Name(None)));
        assert_eq!(c.lookup(&a), Some(Answer::Name(None)));
        assert_eq!(N.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn slow_dns_does_not_block() {
        static N: AtomicUsize = AtomicUsize::new(0);
        fn r(_: &Query) -> Answer {
            N.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(300));
            Answer::Name(Some(b"slow.example".to_vec()))
        }
        let c = leak(r, 60_000, 50);
        let q = Query::Addr(Ip::from_v4(0x0a00_0002));
        let t = Instant::now();
        assert_eq!(c.lookup(&q), None);
        // 待っている間にもう一度来ても、問い合わせは増やさない
        assert_eq!(c.lookup(&q), None);
        assert!(t.elapsed() < Duration::from_millis(250));
        std::thread::sleep(Duration::from_millis(400));
        let t = Instant::now();
        assert_eq!(c.lookup(&q), Some(Answer::Name(Some(b"slow.example".to_vec()))));
        assert!(t.elapsed() < Duration::from_millis(50));
        assert_eq!(N.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_answer_is_returned_while_refreshing() {
        static N: AtomicUsize = AtomicUsize::new(0);
        fn r(_: &Query) -> Answer {
            let n = N.fetch_add(1, Ordering::SeqCst);
            if n > 0 {
                std::thread::sleep(Duration::from_millis(300));
            }
            Answer::Ip(n as u32 + 1)
        }
        let c = leak(r, 20, 2_000);
        assert_eq!(c.lookup(&name("x")), Some(Answer::Ip(1)));
        std::thread::sleep(Duration::from_millis(40));
        let t = Instant::now();
        assert_eq!(c.lookup(&name("x")), Some(Answer::Ip(1)));
        assert!(t.elapsed() < Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(c.lookup(&name("x")), Some(Answer::Ip(2)));
    }
}
