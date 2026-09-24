//! ログ (core/common/peercast.cpp の `ADDLOG` と `LOG_*`、logbuf.cpp の `LogBuffer`)。
//!
//! C++ 版と同じく、ログはプロセスに 1 つで、どのスレッドからも書ける。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use super::sys;

/// `LogBuffer::TYPE`
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// 前の行の続き
    None = 0,
    Trace = 1,
    Debug = 2,
    Info = 3,
    Warn = 4,
    Error = 5,
    Fatal = 6,
    Off = 7,
}

impl Level {
    /// `LogBuffer::getTypeStr`
    pub fn type_str(self) -> &'static str {
        ["", "TRAC", "DBUG", "INFO", "WARN", "EROR", "FATL", " OFF"][self as usize]
    }

    pub fn from_i32(v: i32) -> Level {
        match v {
            1 => Level::Trace,
            2 => Level::Debug,
            3 => Level::Info,
            4 => Level::Warn,
            5 => Level::Error,
            6 => Level::Fatal,
            7 => Level::Off,
            _ => Level::None,
        }
    }
}

/// `LogBuffer`: 最後の 1000 行を、1 行 99 バイトまでに分けて持つ
pub struct LogBuffer {
    lines: Vec<(u32, Level, Vec<u8>)>,
    curr: u32,
    listener_id: u32,
    listeners: BTreeMap<u32, Box<dyn FnMut(u32, Level, &[u8]) + Send>>,
}

const MAX_LINES: u32 = 1000;
const LINE_LEN: usize = 100;

impl Default for LogBuffer {
    fn default() -> Self {
        LogBuffer { lines: Vec::new(), curr: 0, listener_id: 0, listeners: BTreeMap::new() }
    }
}

/// `LogBuffer::copy_utf8`: 文字の途中で切らずに、`buflen` バイトまで写す
fn copy_utf8(src: &[u8], buflen: usize) -> usize {
    let mut i = 0;
    let mut left = buflen;
    while left > 0 && i < src.len() && src[i] != 0 {
        let c = src[i];
        let charlen = if c & 0x80 == 0 {
            1
        } else if c & 0xe0 == 0xc0 {
            2
        } else if c & 0xf0 == 0xe0 {
            3
        } else if c & 0xf8 == 0xf0 {
            4
        } else {
            1
        };
        if left < charlen {
            break;
        }
        i += charlen;
        left -= charlen;
    }
    i.min(src.len())
}

impl LogBuffer {
    /// `write`: 99 バイトずつの行にする (続きの行は時刻 0、種類 `None`)。
    ///
    /// C++ 版は、途中までの UTF-8 で終わる文字列を渡すと進まなくなり、終わらなかった
    /// (docs/cpp-known-issues.md)。Rust 版は、進まなくなったらそこでやめる。
    pub fn write(&mut self, s: &[u8], t: Level) {
        let now = sys::get_time();
        for l in self.listeners.values_mut() {
            l(now, t, s);
        }
        let mut s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
        let mut cnt = 0;
        while !s.is_empty() {
            let rlen = copy_utf8(s, s.len().min(LINE_LEN - 1));
            if rlen == 0 {
                break;
            }
            let i = (self.curr % MAX_LINES) as usize;
            let entry = if cnt == 0 { (now, t, s[..rlen].to_vec()) } else { (0, Level::None, s[..rlen].to_vec()) };
            if i < self.lines.len() {
                self.lines[i] = entry;
            } else {
                self.lines.push(entry);
            }
            self.curr = self.curr.wrapping_add(1);
            s = &s[rlen..];
            cnt += 1;
        }
    }

    /// `eachLine`: 古い順
    pub fn each_line(&self, mut f: impl FnMut(u32, Level, &[u8])) {
        let (n, mut sp) = if self.curr < MAX_LINES { (self.curr, 0) } else { (MAX_LINES, self.curr % MAX_LINES) };
        for _ in 0..n {
            let (t, ty, ref l) = self.lines[sp as usize];
            f(t, ty, l);
            sp = (sp + 1) % MAX_LINES;
        }
    }

    pub fn clear(&mut self) {
        self.curr = 0;
    }

    pub fn add_listener(&mut self, f: Box<dyn FnMut(u32, Level, &[u8]) + Send>) -> u32 {
        let id = self.listener_id;
        self.listener_id = self.listener_id.wrapping_add(1);
        self.listeners.insert(id, f);
        id
    }

    pub fn remove_listener(&mut self, id: u32) {
        self.listeners.remove(&id);
    }

    pub fn listener_ids(&self) -> Vec<u32> {
        self.listeners.keys().copied().collect()
    }

    /// `lineRendererHTML`
    pub fn line_html(time: u32, ty: Level, line: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        if ty != Level::None {
            buf.extend(sys::time_string(time));
            buf.extend_from_slice(b" <b>[");
            buf.extend_from_slice(ty.type_str().as_bytes());
            buf.extend_from_slice(b"]</b> ");
        }
        buf.extend(crate::cgi::escape_html(line));
        buf.extend_from_slice(b"<br>");
        buf
    }

    /// `dumpHTML`
    pub fn dump_html(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.each_line(|t, ty, l| out.extend(Self::line_html(t, ty, l)));
        out
    }
}

/// ログを書く先 (`PeercastApplication::printLog`)
pub type Printer = Box<dyn Fn(Level, &[u8]) + Send + Sync>;

struct Logger {
    level: AtomicI32,
    pause: AtomicBool,
    buffer: Mutex<LogBuffer>,
    printer: Mutex<Option<Printer>>,
}

fn logger() -> &'static Logger {
    static L: OnceLock<Logger> = OnceLock::new();
    L.get_or_init(|| Logger {
        level: AtomicI32::new(Level::Info as i32),
        pause: AtomicBool::new(false),
        buffer: Mutex::new(LogBuffer::default()),
        printer: Mutex::new(None),
    })
}

/// `AUX_LOG_FUNC_VECTOR` の 1 つ
enum AuxSink {
    Collect(Vec<(Level, Vec<u8>)>),
    Func(Box<dyn FnMut(Level, &[u8])>),
}

thread_local! {
    /// `AUX_LOG_FUNC_VECTOR`: ログの水準によらず、このスレッドのログを受け取るもの
    static AUX: RefCell<Vec<AuxSink>> = RefCell::new(Vec::new());
}

/// `ServMgr::logLevel()`
pub fn level() -> i32 {
    logger().level.load(Ordering::Relaxed)
}

/// `ServMgr::logLevel(int)`: 1〜7 の外は無視してエラーを書く
pub fn set_level(v: i32) {
    if (1..=7).contains(&v) {
        logger().level.store(v, Ordering::Relaxed);
    } else {
        crate::log_error!("Trying to set log level outside valid range. Ignored");
    }
}

pub fn paused() -> bool {
    logger().pause.load(Ordering::Relaxed)
}

pub fn set_paused(v: bool) {
    logger().pause.store(v, Ordering::Relaxed);
}

pub fn set_printer(p: Printer) {
    *logger().printer.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
}

/// `sys->logBuf` を使う
pub fn with_buffer<R>(f: impl FnOnce(&mut LogBuffer) -> R) -> R {
    f(&mut logger().buffer.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `peercast::log_escape`: 表示できる ASCII 以外を `[XX]` にする
pub fn log_escape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &c in s {
        if (0x20..=0x7e).contains(&c) {
            out.push(c);
        } else {
            out.extend(format!("[{:02X}]", c).into_bytes());
        }
    }
    out
}

/// `ADDLOG`
pub fn add_log(ty: Level, msg: &[u8]) {
    let l = logger();
    if l.pause.load(Ordering::Relaxed) {
        return;
    }
    const MAX_LINELEN: usize = 1024;
    let tmp = if crate::utf8::validate(msg) { msg.to_vec() } else { log_escape(msg) };
    let tmp = crate::utf8::truncate(&tmp, MAX_LINELEN).unwrap_or(tmp);

    // 受け取る関数の中でログを書いても、ここには戻らない
    let sinks = AUX.with(|a| std::mem::take(&mut *a.borrow_mut()));
    let mut sinks = sinks;
    for v in sinks.iter_mut() {
        match v {
            AuxSink::Collect(v) => v.push((ty, tmp.clone())),
            AuxSink::Func(f) => f(ty, &tmp),
        }
    }
    AUX.with(|a| {
        let mut a = a.borrow_mut();
        let added = std::mem::take(&mut *a);
        *a = sinks;
        a.extend(added);
    });

    if l.level.load(Ordering::Relaxed) > ty as i32 {
        return;
    }
    if ty != Level::None {
        l.buffer.lock().unwrap_or_else(|e| e.into_inner()).write(&tmp, ty);
    }
    if let Some(p) = l.printer.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        p(ty, &tmp);
    }
}

/// `body` を実行する間にこのスレッドで書かれたログを、水準によらず集める
/// (`AUX_LOG_FUNC_VECTOR` に関数を足すのと同じ)
pub fn capture<R>(body: impl FnOnce() -> R) -> (R, Vec<(Level, Vec<u8>)>) {
    AUX.with(|a| a.borrow_mut().push(AuxSink::Collect(Vec::new())));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            AUX.with(|a| {
                a.borrow_mut().pop();
            });
        }
    }
    let guard = Pop;
    let r = body();
    let lines = AUX.with(|a| match a.borrow_mut().last_mut() {
        Some(AuxSink::Collect(v)) => std::mem::take(v),
        _ => Vec::new(),
    });
    drop(guard);
    (r, lines)
}

/// `body` を実行する間にこのスレッドで書かれたログを、水準によらず `f` に渡す
pub fn with_aux<R>(f: Box<dyn FnMut(Level, &[u8])>, body: impl FnOnce() -> R) -> R {
    AUX.with(|a| a.borrow_mut().push(AuxSink::Func(f)));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            AUX.with(|a| {
                a.borrow_mut().pop();
            });
        }
    }
    let _guard = Pop;
    body()
}

#[macro_export]
macro_rules! log_trace { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Trace, format!($($a)*).as_bytes()) } }
#[macro_export]
macro_rules! log_debug { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Debug, format!($($a)*).as_bytes()) } }
#[macro_export]
macro_rules! log_info { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Info, format!($($a)*).as_bytes()) } }
#[macro_export]
macro_rules! log_warn { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Warn, format!($($a)*).as_bytes()) } }
#[macro_export]
macro_rules! log_error { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Error, format!($($a)*).as_bytes()) } }
#[macro_export]
macro_rules! log_fatal { ($($a:tt)*) => { $crate::server::log::add_log($crate::server::log::Level::Fatal, format!($($a)*).as_bytes()) } }

/// バイト列をログの文字列にするときに使う (UTF-8 として正しくなければ、正しくない部分を置き換える)
pub fn b(s: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_lines() {
        let mut b = LogBuffer::default();
        b.write(&[b'a'; 250], Level::Info);
        let mut lines = Vec::new();
        b.each_line(|_, t, l| lines.push((t, l.len())));
        assert_eq!(lines, vec![(Level::Info, 99), (Level::None, 99), (Level::None, 52)]);
        // 文字の途中では切らない
        let mut b = LogBuffer::default();
        let s = "あ".repeat(40);
        b.write(s.as_bytes(), Level::Warn);
        let mut lens = Vec::new();
        b.each_line(|_, _, l| lens.push(l.len()));
        assert_eq!(lens, vec![99, 21]);
        // 途中までの UTF-8 で終わっても終わる
        let mut b = LogBuffer::default();
        b.write(b"a\xe3\x81", Level::Info);
        let mut n = 0;
        b.each_line(|_, _, _| n += 1);
        assert_eq!(n, 1);
        // 1000 行を超えると古いものから消える
        let mut b = LogBuffer::default();
        for i in 0..1005 {
            b.write(format!("{}", i).as_bytes(), Level::Debug);
        }
        let mut first = None;
        let mut count = 0;
        b.each_line(|_, _, l| {
            if first.is_none() {
                first = Some(l.to_vec());
            }
            count += 1;
        });
        assert_eq!((first.unwrap(), count), (b"5".to_vec(), 1000));
    }

    #[test]
    fn escape_and_capture() {
        assert_eq!(log_escape(b"a\xff\n"), b"a[FF][0A]");
        let (r, lines) = capture(|| {
            add_log(Level::Trace, b"hello");
            1
        });
        assert_eq!(r, 1);
        assert_eq!(lines, vec![(Level::Trace, b"hello".to_vec())]);
    }
}
