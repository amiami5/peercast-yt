//! TLS (core/common/sslclientsocket.cpp の `SslClientSocket`)。C++ 版と同じく OpenSSL を使う
//! (C の関数を直接呼ぶので、このモジュールは `unsafe` を使う)。
//!
//! OpenSSL 1.1 以降のみ (1.0 の初期化の関数やマクロの実体は使わない)。

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
use std::sync::Mutex;
use std::time::Instant;

use super::error::{Error, Kind, Result};

#[allow(non_camel_case_types)]
type SSL_CTX = c_void;
#[allow(non_camel_case_types)]
type SSL = c_void;
#[allow(non_camel_case_types)]
type SSL_METHOD = c_void;
#[allow(non_camel_case_types)]
type X509_VERIFY_PARAM = c_void;

#[link(name = "ssl")]
#[link(name = "crypto")]
extern "C" {
    fn OPENSSL_init_ssl(opts: u64, settings: *const c_void) -> c_int;
    fn TLS_client_method() -> *const SSL_METHOD;
    fn TLS_server_method() -> *const SSL_METHOD;
    fn SSL_CTX_new(m: *const SSL_METHOD) -> *mut SSL_CTX;
    fn SSL_CTX_free(ctx: *mut SSL_CTX);
    fn SSL_CTX_set_default_verify_paths(ctx: *mut SSL_CTX) -> c_int;
    fn SSL_CTX_set_verify(ctx: *mut SSL_CTX, mode: c_int, cb: *const c_void);
    fn SSL_CTX_use_certificate_file(ctx: *mut SSL_CTX, file: *const c_char, ty: c_int) -> c_int;
    fn SSL_CTX_use_PrivateKey_file(ctx: *mut SSL_CTX, file: *const c_char, ty: c_int) -> c_int;
    fn SSL_new(ctx: *mut SSL_CTX) -> *mut SSL;
    fn SSL_free(ssl: *mut SSL);
    fn SSL_ctrl(ssl: *mut SSL, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
    fn SSL_get0_param(ssl: *mut SSL) -> *mut X509_VERIFY_PARAM;
    fn X509_VERIFY_PARAM_set_hostflags(p: *mut X509_VERIFY_PARAM, flags: u32);
    fn X509_VERIFY_PARAM_set1_ip_asc(p: *mut X509_VERIFY_PARAM, ip: *const c_char) -> c_int;
    fn X509_VERIFY_PARAM_set1_host(p: *mut X509_VERIFY_PARAM, name: *const c_char, len: usize) -> c_int;
    fn SSL_set_fd(ssl: *mut SSL, fd: c_int) -> c_int;
    fn SSL_connect(ssl: *mut SSL) -> c_int;
    fn SSL_accept(ssl: *mut SSL) -> c_int;
    fn SSL_get_verify_result(ssl: *const SSL) -> c_long;
    fn X509_verify_cert_error_string(n: c_long) -> *const c_char;
    fn SSL_read(ssl: *mut SSL, buf: *mut c_void, num: c_int) -> c_int;
    fn SSL_write(ssl: *mut SSL, buf: *const c_void, num: c_int) -> c_int;
    fn SSL_get_error(ssl: *const SSL, ret: c_int) -> c_int;
    fn SSL_shutdown(ssl: *mut SSL) -> c_int;
    fn ERR_get_error() -> c_ulong;
    fn ERR_clear_error();
    fn ERR_error_string_n(e: c_ulong, buf: *mut c_char, len: usize);
}

/// `struct pollfd` (並びは POSIX で決まっている)
#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

/// `nfds_t` (Linux の glibc と musl は unsigned long、macOS や BSD は unsigned int)
#[cfg(target_os = "linux")]
type NfdsT = c_ulong;
#[cfg(not(target_os = "linux"))]
type NfdsT = std::os::raw::c_uint;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: c_int) -> c_int;
}

const POLLIN: i16 = 1;
const POLLOUT: i16 = 4;

const SSL_VERIFY_PEER: c_int = 1;
const SSL_FILETYPE_PEM: c_int = 1;
const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
const TLSEXT_NAMETYPE_HOST_NAME: c_long = 0;
const X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS: u32 = 0x4;
const X509_V_OK: c_long = 0;
const SSL_ERROR_WANT_READ: c_int = 2;
const SSL_ERROR_WANT_WRITE: c_int = 3;
const SSL_ERROR_ZERO_RETURN: c_int = 6;

fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::log_debug!("Initializing OpenSSL ...");
        // SAFETY: 既定の設定で初期化する
        unsafe {
            OPENSSL_init_ssl(0, std::ptr::null());
        }
    });
}

/// TLS の接続 (ソケットの記述子の上)
pub struct Session {
    ctx: *mut SSL_CTX,
    ssl: *mut SSL,
    fd: c_int,
}

// SSL と SSL_CTX は、1 つのスレッドからしか同時に使わない (Session は &mut でしか使わない)
unsafe impl Send for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: new/accept で作ったもので、ここでだけ解放する
        unsafe {
            if !self.ssl.is_null() {
                SSL_shutdown(self.ssl);
                SSL_free(self.ssl);
            }
            if !self.ctx.is_null() {
                SSL_CTX_free(self.ctx);
            }
        }
    }
}

static SERVER_FILES: Mutex<Option<(Vec<u8>, Vec<u8>)>> = Mutex::new(None);

/// `SslClientSocket::configureServer`
pub fn configure_server(certificate: &[u8], private_key: &[u8]) {
    *SERVER_FILES.lock().unwrap_or_else(|e| e.into_inner()) = Some((certificate.to_vec(), private_key.to_vec()));
}

/// `SslClientSocket::getServerConfiguration`
pub fn server_configuration() -> (Vec<u8>, Vec<u8>) {
    SERVER_FILES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| (b"server.crt".to_vec(), b"server.key".to_vec()))
}

fn cstr(b: &[u8]) -> CString {
    CString::new(b.iter().copied().take_while(|&c| c != 0).collect::<Vec<u8>>()).unwrap_or_default()
}

/// OpenSSL のエラーのキューの先頭の理由 (なければ空)。キューは空にする
fn error_reason() -> String {
    // SAFETY: buf は NUL で終わる文字列を書ける長さ
    unsafe {
        let err = ERR_get_error();
        ERR_clear_error();
        if err == 0 {
            return String::new();
        }
        let mut buf = [0 as c_char; 120];
        ERR_error_string_n(err, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// `fd` が読める (`write` なら書ける) ようになるまで、`deadline` まで待つ。過ぎたら `TimeoutException`。
/// 閉じられたときやエラーのときも戻る (続けて呼ぶ SSL の関数が失敗を返す)
fn wait_fd(fd: c_int, write: bool, deadline: Instant) -> Result<()> {
    loop {
        let rest = deadline.saturating_duration_since(Instant::now()).as_millis();
        if rest == 0 {
            return Err(Error::new(Kind::Timeout, "Handshake timeout"));
        }
        let mut p = PollFd { fd, events: if write { POLLOUT } else { POLLIN }, revents: 0 };
        // SAFETY: p は 1 つの pollfd
        let r = unsafe { poll(&mut p, 1, rest.min(c_int::MAX as u128) as c_int) };
        if r > 0 {
            return Ok(());
        }
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                return Err(e.into());
            }
        }
    }
}

impl Session {
    /// `SslClientSocket::open` と `connect` の TLS の部分: 接続したソケットの上でハンドシェイクする。
    /// `hostname` が空でなければ、SNI を送り、証明書とホスト名を検証する。
    pub fn connect(fd: c_int, hostname: &[u8]) -> Result<Session> {
        init();
        // SAFETY: OpenSSL の関数を、ドキュメントどおりの引数で呼ぶ。作ったものは Session が解放する。
        unsafe {
            ERR_clear_error();
            let ctx = SSL_CTX_new(TLS_client_method());
            if ctx.is_null() {
                return Err(Error::sock("SSL_CTX_new failed"));
            }
            let mut s = Session { ctx, ssl: std::ptr::null_mut(), fd };
            if !hostname.is_empty() {
                if SSL_CTX_set_default_verify_paths(ctx) != 1 {
                    return Err(Error::sock("Failed to load CA certificates"));
                }
                SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, std::ptr::null());
            }
            s.ssl = SSL_new(ctx);
            if s.ssl.is_null() {
                return Err(Error::sock("SSL_new failed"));
            }
            if !hostname.is_empty() {
                let name = cstr(hostname);
                let is_ip = super::host::Ip::parse(hostname).is_some() && !hostname.contains(&b'%');
                if !is_ip {
                    SSL_ctrl(s.ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_HOST_NAME, name.as_ptr() as *mut c_void);
                }
                let param = SSL_get0_param(s.ssl);
                X509_VERIFY_PARAM_set_hostflags(param, X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS);
                let ok =
                    if is_ip { X509_VERIFY_PARAM_set1_ip_asc(param, name.as_ptr()) } else { X509_VERIFY_PARAM_set1_host(param, name.as_ptr(), 0) };
                if ok != 1 {
                    return Err(Error::sock("Failed to set the host name to verify"));
                }
            }
            if SSL_set_fd(s.ssl, fd) == 0 {
                return Err(Error::sock("SSL_set_fd failed"));
            }
            if SSL_connect(s.ssl) != 1 {
                let vr = SSL_get_verify_result(s.ssl);
                if vr != X509_V_OK {
                    let msg = std::ffi::CStr::from_ptr(X509_verify_cert_error_string(vr)).to_string_lossy().into_owned();
                    return Err(Error::sock(format!("SSL handshake failed (certificate verification failed: {})", msg)));
                }
                return Err(Error::sock("SSL handshake failed"));
            }
            Ok(s)
        }
    }

    /// SSL の操作 `op` (戻り値が 0 以下なら失敗か待ち) を行う。`deadline` があれば、ソケットは
    /// ノンブロッキングにしておき、読めるか書けるようになるのを期限まで待って繰り返す。
    /// ブロッキングのままだと、中の recv のたびに読む待ち時間をまるごと使えるので、1 バイトずつ
    /// 送られると期限を過ぎても終わらない (Slowloris)。最後の戻り値を返す
    fn run(&mut self, deadline: Option<Instant>, mut op: impl FnMut(*mut SSL) -> c_int) -> Result<c_int> {
        loop {
            // SAFETY: ssl は作ってあり、このスレッドでしか使わない
            let (r, err) = unsafe {
                ERR_clear_error();
                let r = op(self.ssl);
                (r, if r <= 0 { SSL_get_error(self.ssl, r) } else { 0 })
            };
            match (deadline, err) {
                (Some(d), SSL_ERROR_WANT_READ) => wait_fd(self.fd, false, d)?,
                (Some(d), SSL_ERROR_WANT_WRITE) => wait_fd(self.fd, true, d)?,
                _ => return Ok(r),
            }
        }
    }

    /// `SslClientSocket::upgrade`: 受け付けたソケットの上で、サーバーとしてハンドシェイクする。
    /// `deadline` があれば、ソケットはノンブロッキングにしておく (`run`)
    pub fn accept(fd: c_int, deadline: Option<Instant>) -> Result<Session> {
        init();
        let (crt, key) = server_configuration();
        // SAFETY: 同上
        let mut s = unsafe {
            ERR_clear_error();
            let ctx = SSL_CTX_new(TLS_server_method());
            if ctx.is_null() {
                return Err(Error::general("SSL_CTX_new failed"));
            }
            let mut s = Session { ctx, ssl: std::ptr::null_mut(), fd };
            if SSL_CTX_use_certificate_file(ctx, cstr(&crt).as_ptr(), SSL_FILETYPE_PEM) <= 0 {
                return Err(Error::general("Certificate file"));
            }
            if SSL_CTX_use_PrivateKey_file(ctx, cstr(&key).as_ptr(), SSL_FILETYPE_PEM) <= 0 {
                return Err(Error::general("Private key file"));
            }
            s.ssl = SSL_new(ctx);
            if s.ssl.is_null() {
                return Err(Error::general("SSL_new failed"));
            }
            if SSL_set_fd(s.ssl, fd) != 1 {
                return Err(Error::stream("upgrade: SSL_set_fd"));
            }
            s
        };
        // SAFETY: ssl は作ってある
        let ret = s.run(deadline, |ssl| unsafe { SSL_accept(ssl) })?;
        if ret <= 0 {
            // SAFETY: 同上
            let code = unsafe { SSL_get_error(s.ssl, ret) };
            let reason = error_reason();
            let reason = if reason.is_empty() { String::new() } else { format!(" ({})", reason) };
            return Err(Error::stream(format!("upgrade: SSL_accept: ret = {}, code = {}{}", ret, code, reason)));
        }
        Ok(s)
    }

    /// `SSL_read` 1 回。相手が閉じたら `Ok(0)`。`deadline` は `accept` と同じ
    pub fn read_once(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> Result<usize> {
        let n = buf.len().min(i32::MAX as usize) as c_int;
        let p = buf.as_mut_ptr() as *mut c_void;
        // SAFETY: buf は n バイト書ける
        let r = self.run(deadline, |ssl| unsafe { SSL_read(ssl, p, n) })?;
        if r <= 0 {
            // SAFETY: ssl は作ってある
            let err = unsafe { SSL_get_error(self.ssl, r) };
            if err == SSL_ERROR_ZERO_RETURN {
                return Ok(0);
            }
            return Err(Error::sock(format!("SSL_read failed, error = {}", err)));
        }
        Ok(r as usize)
    }

    /// `SSL_write` (全部書く)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let n = data.len().min(i32::MAX as usize) as c_int;
        let p = data.as_ptr() as *const c_void;
        // SAFETY: data は n バイト読める
        let r = self.run(None, |ssl| unsafe { SSL_write(ssl, p, n) })?;
        if r <= 0 {
            return Err(Error::sock(format!("SSL_write failed: {}", error_reason())));
        }
        if (r as usize) < data.len() {
            return self.write(&data[r as usize..]);
        }
        Ok(())
    }
}
