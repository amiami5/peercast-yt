//! TLS (core/common/sslclientsocket.cpp の `SslClientSocket`)。C++ 版と同じく OpenSSL を使う
//! (C の関数を直接呼ぶので、このモジュールは `unsafe` を使う)。
//!
//! OpenSSL 1.1 以降のみ (1.0 の初期化の関数やマクロの実体は使わない)。

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
use std::sync::Mutex;

use super::error::{Error, Result};

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
    fn ERR_error_string_n(e: c_ulong, buf: *mut c_char, len: usize);
}

const SSL_VERIFY_PEER: c_int = 1;
const SSL_FILETYPE_PEM: c_int = 1;
const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
const TLSEXT_NAMETYPE_HOST_NAME: c_long = 0;
const X509_CHECK_FLAG_NO_PARTIAL_WILDCARDS: u32 = 0x4;
const X509_V_OK: c_long = 0;
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

impl Session {
    /// `SslClientSocket::open` と `connect` の TLS の部分: 接続したソケットの上でハンドシェイクする。
    /// `hostname` が空でなければ、SNI を送り、証明書とホスト名を検証する。
    pub fn connect(fd: c_int, hostname: &[u8]) -> Result<Session> {
        init();
        // SAFETY: OpenSSL の関数を、ドキュメントどおりの引数で呼ぶ。作ったものは Session が解放する。
        unsafe {
            let ctx = SSL_CTX_new(TLS_client_method());
            if ctx.is_null() {
                return Err(Error::sock("SSL_CTX_new failed"));
            }
            let mut s = Session { ctx, ssl: std::ptr::null_mut() };
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

    /// `SslClientSocket::upgrade`: 受け付けたソケットの上で、サーバーとしてハンドシェイクする
    pub fn accept(fd: c_int) -> Result<Session> {
        init();
        let (crt, key) = server_configuration();
        // SAFETY: 同上
        unsafe {
            let ctx = SSL_CTX_new(TLS_server_method());
            if ctx.is_null() {
                return Err(Error::general("SSL_CTX_new failed"));
            }
            let mut s = Session { ctx, ssl: std::ptr::null_mut() };
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
            let ret = SSL_accept(s.ssl);
            if ret <= 0 {
                let code = SSL_get_error(s.ssl, ret);
                return Err(Error::stream(format!("upgrade: SSL_accept: ret = {}, code = {}", ret, code)));
            }
            Ok(s)
        }
    }

    /// `SSL_read` 1 回。相手が閉じたら `Ok(0)`。
    pub fn read_once(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(i32::MAX as usize) as c_int;
        // SAFETY: buf は n バイト書ける
        unsafe {
            let r = SSL_read(self.ssl, buf.as_mut_ptr() as *mut c_void, n);
            if r <= 0 {
                let err = SSL_get_error(self.ssl, r);
                if err == SSL_ERROR_ZERO_RETURN {
                    return Ok(0);
                }
                return Err(Error::sock(format!("SSL_read failed, error = {}", err)));
            }
            Ok(r as usize)
        }
    }

    /// `SSL_write` (全部書く)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let n = data.len().min(i32::MAX as usize) as c_int;
        // SAFETY: data は n バイト読める
        unsafe {
            let r = SSL_write(self.ssl, data.as_ptr() as *const c_void, n);
            if r <= 0 {
                let err = ERR_get_error();
                let mut buf = [0 as c_char; 120];
                ERR_error_string_n(err, buf.as_mut_ptr(), buf.len());
                let msg = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                return Err(Error::sock(format!("SSL_write failed: {}", msg)));
            }
            if (r as usize) < data.len() {
                return self.write(&data[r as usize..]);
            }
        }
        Ok(())
    }
}
