//! フィルターの判定で使う名前解決 (正引き・逆引き) の結果を覚えておく。
//!
//! 接続の処理が DNS の応答を長く待たないよう、覚えたものは古くてもすぐ返して別のスレッドで引き直し、
//! まだ覚えていないものだけ決まった時間まで待つ。間に合わなければ「わからない」(`None`) を返す。
//! 逆引きの応答は相手の側の DNS サーバーが返すので、相手がわざと遅くできる。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::host::{get_ip, Ip};

/// 覚えたものを引き直すまでの秒数の既定 (ini の `dnsCacheSeconds`)
pub const DEFAULT_TTL_SECONDS: u32 = 60;
/// まだ覚えていないものを待つミリ秒数の既定 (ini の `dnsWaitMillis`)
pub const DEFAULT_WAIT_MILLIS: u32 = 2000;
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
    ttl_ms: AtomicU64,
    wait_ms: AtomicU64,
}

impl DnsCache {
    pub fn new(resolve: fn(&Query) -> Answer, ttl: Duration, wait: Duration) -> DnsCache {
        let c = DnsCache { st: Mutex::new(State::default()), cv: Condvar::new(), resolve, ttl_ms: AtomicU64::new(0), wait_ms: AtomicU64::new(0) };
        c.set_limits(ttl, wait);
        c
    }

    /// 覚えたものを引き直すまでの時間と、まだ覚えていないものを待つ時間を変える
    pub fn set_limits(&self, ttl: Duration, wait: Duration) {
        self.ttl_ms.store(ttl.as_millis() as u64, Ordering::Relaxed);
        self.wait_ms.store(wait.as_millis() as u64, Ordering::Relaxed);
    }

    fn ttl(&self) -> Duration {
        Duration::from_millis(self.ttl_ms.load(Ordering::Relaxed))
    }

    fn wait(&self) -> Duration {
        Duration::from_millis(self.wait_ms.load(Ordering::Relaxed))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 覚えている答えを (古くても) 返す。覚えていなければ `wait` まで待ち、間に合わなければ `None`
    pub fn lookup(&'static self, q: &Query) -> Option<Answer> {
        let mut st = self.state();
        self.start(&mut st, q);
        let deadline = Instant::now() + self.wait();
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
        let ttl = self.ttl();
        if let Some(e) = st.m.get(q) {
            if e.resolving || (e.answer.is_some() && now.duration_since(e.fetched) < ttl) {
                return;
            }
        }
        if st.resolving >= MAX_RESOLVING {
            return;
        }
        if !st.m.contains_key(q) && st.m.len() >= MAX_ENTRIES {
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
        Query::Addr(ip) => {
            Answer::Name(super::sys::hostname_by_address(ip).and_then(|n| forward_confirmed(ip, n, &super::host::resolve_all)))
        }
    }
}

/// 逆引きで得た名前 `name` を正引きし、そのアドレスに `ip` が含まれるときだけ名前を返す
/// (forward-confirmed reverse DNS)。PTR はアドレスの持ち主が好きな名前にできるので、そのままでは
/// フィルターの `.` で始まる名前に一致させられる (C++ 版は逆引きだけで判定した。security-review #47)
fn forward_confirmed(ip: &Ip, name: Vec<u8>, resolve_all: &dyn Fn(&[u8]) -> Vec<Ip>) -> Option<Vec<u8>> {
    if resolve_all(&name).contains(ip) {
        Some(name)
    } else {
        crate::log_debug!("Reverse DNS name {} of {} does not resolve back to it", String::from_utf8_lossy(&name), ip.str());
        None
    }
}

fn global() -> &'static DnsCache {
    static C: OnceLock<DnsCache> = OnceLock::new();
    C.get_or_init(|| {
        DnsCache::new(resolve, Duration::from_secs(DEFAULT_TTL_SECONDS.into()), Duration::from_millis(DEFAULT_WAIT_MILLIS.into()))
    })
}

/// ini の `dnsCacheSeconds` と `dnsWaitMillis` を反映する
pub fn configure(ttl_seconds: u32, wait_millis: u32) {
    global().set_limits(Duration::from_secs(ttl_seconds.into()), Duration::from_millis(wait_millis.into()));
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
    fn limits_can_be_changed() {
        fn r(_: &Query) -> Answer {
            std::thread::sleep(Duration::from_millis(100));
            Answer::Ip(7)
        }
        let c = leak(r, 60_000, 2_000);
        c.set_limits(Duration::from_secs(60), Duration::ZERO);
        let t = Instant::now();
        assert_eq!(c.lookup(&name("a")), None);
        assert!(t.elapsed() < Duration::from_millis(50));
        c.set_limits(Duration::from_secs(60), Duration::from_secs(2));
        assert_eq!(c.lookup(&name("b")), Some(Answer::Ip(7)));
    }

    /// 逆引きの名前は、正引きして元のアドレスに戻るときだけ使う (security-review #47)
    #[test]
    fn reverse_name_is_forward_confirmed() {
        let ip = Ip::from_v4(0xcb00_7105); // 203.0.113.5
        let other = Ip::from_v4(0xcb00_7106);
        let v6 = Ip::parse(b"2001:db8::5").unwrap();
        let fake = |n: &[u8]| -> Vec<Ip> {
            match n {
                b"host.example.jp" => vec![v6, Ip::from_v4(0xcb00_7105)],
                b"lies.example.jp" => vec![Ip::from_v4(0xcb00_7106)],
                _ => Vec::new(),
            }
        };
        assert_eq!(forward_confirmed(&ip, b"host.example.jp".to_vec(), &fake), Some(b"host.example.jp".to_vec()));
        assert_eq!(forward_confirmed(&v6, b"host.example.jp".to_vec(), &fake), Some(b"host.example.jp".to_vec()));
        assert_eq!(forward_confirmed(&ip, b"lies.example.jp".to_vec(), &fake), None);
        assert_eq!(forward_confirmed(&other, b"host.example.jp".to_vec(), &fake), None);
        assert_eq!(forward_confirmed(&ip, b"nx.example.jp".to_vec(), &fake), None);
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
