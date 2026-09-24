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

    /// `notifyMessage`
    pub fn notify_message(&self, ty: u32, message: &[u8]) {
        crate::log_info!("Notification: {}", String::from_utf8_lossy(message));
        if self.enable_notify_send {
            let icon = [&self.html_path[..], b"assets/images/small-logo.png"].concat();
            let args: Vec<std::ffi::OsString> =
                vec![b"-i".to_vec(), icon, b"--".to_vec(), notif::type_str(ty).as_bytes().to_vec(), message.to_vec()].into_iter().map(os_string).collect();
            // 通知デーモンが起動できない環境では数十秒かかるので、終わりを待たない
            match std::process::Command::new("notify-send").args(&args).spawn() {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let r = child.wait();
                        crate::log_debug!("notifyMessage: notify-send = {:?}", r.map(|s| s.code()));
                    });
                }
                Err(e) => crate::log_debug!("notifyMessage: notify-send: {}", e),
            }
        }
    }
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
