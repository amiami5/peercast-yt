//! アプリケーションの設定とOSとのやりとり (C++ 版の `PeercastApplication` と ui/linux/main.cpp の
//! `MyPeercastApp`、core/unix/usys.cpp の `openURL` など)

use super::notif;
use super::state::{obj, s, Value};

/// ビルドした日時 (C++ 版は `__DATE__ " " __TIME__`)。ビルドのときに `PEERCAST_BUILD_DATE_TIME` で与える
pub const BUILD_DATE_TIME: &str = match option_env!("PEERCAST_BUILD_DATE_TIME") {
    Some(v) => v,
    None => "unknown",
};

/// `PeercastApplication` の各パス
#[derive(Clone, Debug, Default)]
pub struct App {
    /// `getPath`: html などのファイルのあるディレクトリ (末尾は "/")
    pub html_path: Vec<u8>,
    /// `getIniFilename`
    pub ini_filename: Vec<u8>,
    /// `getSettingsDirPath`
    pub settings_dir: Vec<u8>,
    /// `getTokenListFilename`
    pub token_list_filename: Vec<u8>,
    /// `getCacheDirPath`
    pub cache_dir: Vec<u8>,
    /// `getStateDirPath`
    pub state_dir: Vec<u8>,
    /// notify-send で通知する
    pub enable_notify_send: bool,
}

impl App {
    /// `PeercastApplication::getState`
    pub fn state(&self) -> Value {
        obj(vec![
            ("path", s(&self.html_path)),
            ("settingsDirPath", s(&self.settings_dir)),
            ("iniFilename", s(&self.ini_filename)),
            ("tokenListFilename", s(&self.token_list_filename)),
            ("cacheDirPath", s(&self.cache_dir)),
            ("stateDirPath", s(&self.state_dir)),
        ])
    }

    /// `notifyMessage`。notify-send は同時に 1 つ、`NOTIFY_SEND_INTERVAL` に 1 回までにし、その間の
    /// 通知は (ログと通知の一覧には残して) notify-send を起こさない (C++ 版は通知ごとに起こした。#46)
    pub fn notify_message(&self, ty: u32, message: &[u8]) {
        crate::log_info!("Notification: {}", String::from_utf8_lossy(message));
        if self.enable_notify_send {
            static GATE: NotifyGate = NotifyGate::new();
            let ticket = match GATE.try_start(std::time::Instant::now()) {
                Some(t) => t,
                None => {
                    crate::log_debug!("notifyMessage: skipping notify-send (too frequent)");
                    return;
                }
            };
            let icon = [&self.html_path[..], b"assets/images/small-logo.png"].concat();
            let args: Vec<std::ffi::OsString> =
                vec![b"-i".to_vec(), icon, b"--".to_vec(), notif::type_str(ty).as_bytes().to_vec(), markup_escape(message)].into_iter().map(os_string).collect();
            // 通知デーモンが起動できない環境では数十秒かかるので、終わりを待たない
            match std::process::Command::new("notify-send").args(&args).spawn() {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let r = child.wait();
                        crate::log_debug!("notifyMessage: notify-send = {:?}", r.map(|s| s.code()));
                        drop(ticket);
                    });
                }
                Err(e) => crate::log_debug!("notifyMessage: notify-send: {}", e),
            }
        }
    }
}

/// notify-send を続けて起こさない間隔
pub const NOTIFY_SEND_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// notify-send を同時に 1 つ、`NOTIFY_SEND_INTERVAL` に 1 回までにする
pub struct NotifyGate {
    running: std::sync::atomic::AtomicBool,
    last: std::sync::Mutex<Option<std::time::Instant>>,
}

/// `NotifyGate::try_start` で得たもの。捨てると次を起こせる
pub struct NotifyTicket<'a>(&'a NotifyGate);

impl Drop for NotifyTicket<'_> {
    fn drop(&mut self) {
        self.0.running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl NotifyGate {
    pub const fn new() -> NotifyGate {
        NotifyGate { running: std::sync::atomic::AtomicBool::new(false), last: std::sync::Mutex::new(None) }
    }

    /// 起こしてよければ `Some`。動いているものがあるか、前に起こしてから間がなければ `None`
    pub fn try_start(&self, now: std::time::Instant) -> Option<NotifyTicket<'_>> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*last, Some(t) if now.saturating_duration_since(t) < NOTIFY_SEND_INTERVAL) {
            return None;
        }
        if self.running.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        *last = Some(now);
        Some(NotifyTicket(self))
    }
}

impl Default for NotifyGate {
    fn default() -> Self {
        NotifyGate::new()
    }
}

/// notify-send の本文のエスケープ。本文は多くの通知のデーモンでマークアップとして解釈されるので、
/// チャンネル名やコメントのタグが効かないよう `& < >` を実体参照にする (要約はマークアップにならない)
fn markup_escape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(unix)]
fn os_string(b: Vec<u8>) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(b)
}

#[cfg(not(unix))]
fn os_string(b: Vec<u8>) -> std::ffi::OsString {
    String::from_utf8_lossy(&b).into_owned().into()
}

/// `USys::openURL`: xdg-open で開く (DISPLAY がなければ何もしない)
pub fn open_url(url: &[u8]) {
    match std::env::var_os("DISPLAY") {
        Some(d) if !d.is_empty() => {}
        _ => {
            crate::log_warn!("openURL: Ignoring request (no DISPLAY environment variable): {}", String::from_utf8_lossy(url));
            return;
        }
    }
    crate::log_debug!("openURL: xdg-open {}", String::from_utf8_lossy(&crate::inspect::inspect(url)));
    match std::process::Command::new("xdg-open").arg(os_string(url.to_vec())).status() {
        Ok(st) => match st.code() {
            Some(0) => {}
            Some(c) => crate::log_error!("openURL: Shell exited with error status ({})", c),
            None => crate::log_error!("openURL: Shell terminated abnormally"),
        },
        Err(e) => crate::log_error!("openURL: {}", e),
    }
}

/// `USys::executeFile`
pub fn execute_file(file: &[u8]) {
    open_url(file);
}

/// `USys::callLocalURL`
pub fn call_local_url(path: &[u8], port: u16) {
    open_url(&[format!("http://localhost:{}/", port).as_bytes(), path].concat());
}

/// `USys::exit`
pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_escape_tags() {
        assert_eq!(markup_escape(b"<a href=\"x\">A&B</a> 'q'"), b"&lt;a href=\"x\"&gt;A&amp;B&lt;/a&gt; 'q'".to_vec());
        assert_eq!(markup_escape("日本語".as_bytes()), "日本語".as_bytes().to_vec());
    }

    /// notify-send は同時に 1 つ、間を空けてしか起こさない (security-review #46)
    #[test]
    fn notify_gate() {
        let g = NotifyGate::new();
        let t0 = std::time::Instant::now();
        let t = g.try_start(t0).expect("first");
        // 動いている間と、間を空けないうちは起こさない
        assert!(g.try_start(t0 + NOTIFY_SEND_INTERVAL).is_none());
        drop(t);
        assert!(g.try_start(t0 + NOTIFY_SEND_INTERVAL / 2).is_none());
        let t = g.try_start(t0 + NOTIFY_SEND_INTERVAL).expect("after interval");
        drop(t);
        assert!(g.try_start(t0 + NOTIFY_SEND_INTERVAL * 2).is_some());
    }
}
