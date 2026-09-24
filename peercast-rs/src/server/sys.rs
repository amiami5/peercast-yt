//! 時刻、乱数、スレッド、パス (core/common/sys.cpp の `Sys`、core/unix/usys.cpp の `USys`)。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::os;

/// `Sys::getTime`: UNIX 時間の秒 (C++ 版と同じく `unsigned int`)
pub fn get_time() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

/// `USys::getDTime`: 秒 (小数つき)
pub fn get_dtime() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `Sys::sleep` (ミリ秒)
pub fn sleep(ms: u32) {
    std::thread::sleep(Duration::from_millis(ms as u64));
}

static IDLE_SLEEP_TIME: AtomicU32 = AtomicU32::new(10);

/// `Sys::idleSleepTime`
pub fn idle_sleep_time() -> u32 {
    IDLE_SLEEP_TIME.load(Ordering::Relaxed)
}

pub fn set_idle_sleep_time(v: u32) {
    IDLE_SLEEP_TIME.store(v, Ordering::Relaxed);
}

/// `Sys::sleepIdle`
pub fn sleep_idle() {
    sleep(idle_sleep_time());
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `String::setFromTime`: `"Sun Nov  6 08:49:37 1994\n"` (地方時。最後に改行がある)
pub fn time_string(t: u32) -> Vec<u8> {
    match os::localtime(t as i64) {
        Some(tm) => format!(
            "{} {} {:2} {:02}:{:02}:{:02} {}\n",
            DAYS[tm.wday.rem_euclid(7) as usize],
            MONTHS[tm.mon.rem_euclid(12) as usize],
            tm.mday,
            tm.hour,
            tm.min,
            tm.sec,
            tm.year + 1900
        )
        .into_bytes(),
        None => b"-".to_vec(),
    }
}

/// `peercast::Random` (C++ 版と同じ数列)
#[derive(Clone, Copy, Debug)]
pub struct Random {
    a: [u64; 2],
}

impl Random {
    pub fn new(seed: i32) -> Random {
        let mut r = Random { a: [0; 2] };
        r.set_seed(seed);
        r
    }

    /// `setSeed`: `unsigned long` に `int` を入れる (負の数は符号拡張。x86-64 の 64 ビットの long)
    pub fn set_seed(&mut self, s: i32) {
        self.a = [s as i64 as u64; 2];
    }

    /// `next`: `RAND(a, b)` を `unsigned long` で計算して `unsigned int` にする
    pub fn next(&mut self) -> u32 {
        let [a, b] = &mut self.a;
        *a = 36969u64.wrapping_mul(*a & 65535).wrapping_add(*a >> 16);
        *b = 18000u64.wrapping_mul(*b & 65535).wrapping_add(*b >> 16);
        ((*a << 16).wrapping_add(*b)) as u32
    }
}

impl Default for Random {
    fn default() -> Self {
        Random::new(0x14235465)
    }
}

fn rnd_gen() -> &'static Mutex<Random> {
    static R: OnceLock<Mutex<Random>> = OnceLock::new();
    R.get_or_init(|| {
        // USys のコンストラクター: /dev/urandom から種を作る。なければ PID から
        let mut r = Random::default();
        use std::io::Read;
        let mut b = [0u8; 4];
        match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)) {
            Ok(()) => r.set_seed(i32::from_ne_bytes(b)),
            Err(_) => {
                let v = r.next().wrapping_add(std::process::id());
                r.set_seed(v as i32);
            }
        }
        Mutex::new(r)
    })
}

/// `Sys::rnd`
pub fn rnd() -> u32 {
    rnd_gen().lock().unwrap_or_else(|e| e.into_inner()).next()
}

/// `ThreadInfo` の、スレッドを止めるための旗
#[derive(Clone, Debug, Default)]
pub struct ThreadFlag(Arc<AtomicBool>);

impl ThreadFlag {
    pub fn new() -> ThreadFlag {
        ThreadFlag(Arc::new(AtomicBool::new(false)))
    }

    /// `ThreadInfo::active`
    pub fn active(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn set_active(&self, v: bool) {
        self.0.store(v, Ordering::SeqCst);
    }

    /// `ThreadInfo::shutdown`
    pub fn shutdown(&self) {
        self.set_active(false);
    }
}

/// `f` を呼び、panic したら捕まえてログに書く (C++ 版のスレッドが例外を捕まえてログに書いていたのと
/// 同じく、そのスレッドの処理だけを終わらせてサーバーは続ける)。panic したら `None`。
pub fn catch_panic<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown".to_string()
            };
            crate::log_error!("{}: internal error (panic): {}", what, msg);
            None
        }
    }
}

/// `Sys::startThread`: 旗を立ててスレッドを始める。始められなければ false。
pub fn start_thread<F>(flag: &ThreadFlag, name: &str, f: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    flag.set_active(true);
    let name: String = name.chars().take(15).collect();
    let what = name.clone();
    match std::thread::Builder::new().name(name).spawn(move || {
        catch_panic(&what, f);
    }) {
        Ok(_) => true,
        Err(_) => {
            crate::log_error!("Error creating thread");
            false
        }
    }
}

/// `USys::getHostname`
pub fn hostname() -> Vec<u8> {
    os::hostname().unwrap_or_else(|| b"localhost".to_vec())
}

/// `USys::getAllIPAddresses`: `hostname -I` の出力
pub fn all_ip_addresses() -> Vec<Vec<u8>> {
    let out = match std::process::Command::new("hostname").arg("-I").stdin(std::process::Stdio::null()).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    if !out.status.success() {
        match out.status.code() {
            Some(c) => crate::log_error!("hostname exited with status code {}", c),
            None => crate::log_error!("hostname didn't exit normally"),
        }
    }
    let mut buf = out.stdout;
    while buf.last().map_or(false, |c| c.is_ascii_whitespace()) {
        buf.pop();
    }
    crate::strutil::split(&buf, b" ")
}

/// `USys::getHostnameByAddress`
pub fn hostname_by_address(ip: &super::host::Ip) -> Option<Vec<u8>> {
    let r = os::reverse_lookup(&ip.0);
    match &r {
        Some(h) => crate::log_trace!("getnameinfo: {} ({})", String::from_utf8_lossy(h), ip.str()),
        None => crate::log_trace!("getnameinfo: failed ({})", ip.str()),
    }
    r
}

/// `USys::getExecutablePath`
pub fn executable_path() -> Vec<u8> {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned().into_bytes()).unwrap_or_default()
}

/// `USys::dirname`
pub fn dirname(path: &[u8]) -> Vec<u8> {
    let mut normal = Vec::new();
    for (i, &c) in path.iter().enumerate() {
        if c == b'/' {
            if path.get(i + 1) != Some(&b'/') {
                normal.push(c);
            }
        } else {
            normal.push(c);
        }
    }
    if normal.len() > 1 && normal.last() == Some(&b'/') {
        normal.pop();
    }
    while normal.last().map_or(false, |&c| c != b'/') {
        normal.pop();
    }
    if normal.is_empty() {
        return b".".to_vec();
    }
    if normal != b"/" && normal.last() == Some(&b'/') {
        normal.pop();
    }
    normal
}

/// `USys::joinPath`
pub fn join_path(parts: &[&[u8]]) -> Vec<u8> {
    let mut result = Vec::new();
    for (i, frag) in parts.iter().enumerate() {
        if frag.is_empty() {
            continue;
        }
        let mut frag: &[u8] = frag;
        if i != 0 {
            while frag.first() == Some(&b'/') {
                frag = &frag[1..];
            }
            if frag.is_empty() {
                continue;
            }
            result.push(b'/');
        }
        while frag.last() == Some(&b'/') {
            frag = &frag[..frag.len() - 1];
        }
        result.extend_from_slice(frag);
    }
    result
}

/// `USys::realPath` (ファイルがなければ `None`)
pub fn real_path(path: &[u8]) -> Option<Vec<u8>> {
    let p = bytes_to_path(path)?;
    std::fs::canonicalize(p).ok().map(|p| path_to_bytes(&p))
}

#[cfg(unix)]
pub fn bytes_to_path(b: &[u8]) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
    Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b)))
}

#[cfg(not(unix))]
pub fn bytes_to_path(b: &[u8]) -> Option<std::path::PathBuf> {
    let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
    std::str::from_utf8(b).ok().map(std::path::PathBuf::from)
}

#[cfg(unix)]
pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    p.to_string_lossy().into_owned().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_sequence() {
        // C++ 版の peercast::Random(0x14235465) の最初の値
        let mut r = Random::default();
        let a = 36969u64 * (0x14235465 & 65535) + (0x14235465 >> 16);
        let b = 18000u64 * (0x14235465 & 65535) + (0x14235465 >> 16);
        assert_eq!(r.next(), ((a << 16) + b) as u32);
        let mut r = Random::new(-1);
        r.next();
    }

    #[test]
    fn paths() {
        assert_eq!(dirname(b"/usr/bin/peercast"), b"/usr/bin");
        assert_eq!(dirname(b"/peercast"), b"/");
        assert_eq!(dirname(b"peercast"), b".");
        assert_eq!(dirname(b"//a//b//"), b"/a");
        assert_eq!(join_path(&[b"/a/", b"/b", b"", b"c/"]), b"/a/b/c");
        assert_eq!(join_path(&[b"a", b"/"]), b"a");
    }
}
