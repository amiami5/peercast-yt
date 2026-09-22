//! C++ から呼ぶための C ABI。この crate で `unsafe` を使うのはここだけ。
//!
//! 約束事 (詳しくは include/peercast_rs.h):
//!  - 入力は「先頭ポインタ + 長さ」のバイト列。長さが 0 ならポインタは NULL でもよい。
//!  - 出力は `PcrsBuf` (Rust が確保したバイト列)。受け取った側が `pcrs_buf_free` で必ず返す。
//!  - 失敗しうる関数は、成功で 0、失敗で -1 を返し、結果は出力引数に書く。
//!  - panic は起こさない設計だが、起きた場合は abort する (Cargo.toml の `panic = "abort"`)。

use crate::{cgi, inspect, url, utf8};

/// Rust が確保したバイト列。`pcrs_buf_free` で解放する。
#[repr(C)]
pub struct PcrsBuf {
    pub ptr: *mut u8,
    pub len: usize,
}

fn into_buf(v: Vec<u8>) -> PcrsBuf {
    let boxed = v.into_boxed_slice();
    let len = boxed.len();
    let ptr = Box::into_raw(boxed) as *mut u8;
    PcrsBuf { ptr, len }
}

/// # Safety
/// `ptr` は、長さが 0 でなければ、`len` バイト読める有効なポインタであること。
unsafe fn input<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 || ptr.is_null() {
        &[]
    } else {
        // SAFETY: 呼び出し側が保証する (関数の Safety 節)
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

/// # Safety
/// `buf` は、この crate の関数が返した `PcrsBuf` で、まだ解放されていないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_buf_free(buf: PcrsBuf) {
    if buf.ptr.is_null() {
        return;
    }
    // SAFETY: into_buf で作った Box<[u8]> と同じポインタと長さ
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buf.ptr, buf.len)) });
}

macro_rules! bytes_to_buf {
    ($(#[$doc:meta])* $name:ident, $f:path) => {
        $(#[$doc])*
        /// # Safety
        /// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
        #[no_mangle]
        pub unsafe extern "C" fn $name(s: *const u8, n: usize) -> PcrsBuf {
            // SAFETY: 関数の Safety 節
            into_buf($f(unsafe { input(s, n) }))
        }
    };
}

macro_rules! bytes_to_bool {
    ($name:ident, $f:path) => {
        /// # Safety
        /// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
        #[no_mangle]
        pub unsafe extern "C" fn $name(s: *const u8, n: usize) -> bool {
            // SAFETY: 関数の Safety 節
            $f(unsafe { input(s, n) })
        }
    };
}

bytes_to_buf!(pcrs_cgi_escape, cgi::escape);
bytes_to_buf!(pcrs_cgi_unescape, cgi::unescape);
bytes_to_buf!(pcrs_cgi_escape_html, cgi::escape_html);
bytes_to_buf!(pcrs_cgi_unescape_html, cgi::unescape_html);
bytes_to_buf!(pcrs_cgi_escape_javascript, cgi::escape_javascript);
bytes_to_buf!(pcrs_str_valid_utf8, utf8::valid);
bytes_to_buf!(pcrs_str_inspect, inspect::inspect);
bytes_to_bool!(pcrs_cgi_is_safe_local_path, cgi::is_safe_local_path);
bytes_to_bool!(pcrs_str_validate_utf8, utf8::validate);
bytes_to_bool!(pcrs_str_is_http_url, url::is_http_url);

fn store(out: *mut PcrsBuf, r: Option<Vec<u8>>) -> i32 {
    match r {
        Some(v) => {
            if !out.is_null() {
                // SAFETY: 呼び出し側が有効な書き込み先を渡す (各関数の Safety 節)
                #[allow(unsafe_code)]
                unsafe {
                    out.write(into_buf(v));
                }
            }
            0
        }
        None => -1,
    }
}

/// 成功で 0、UTF-8 として不正なら -1。
///
/// # Safety
/// `s` は `n` バイト読めること。`out` は書き込める `PcrsBuf` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_json_inspect(s: *const u8, n: usize, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    store(out, inspect::json_inspect(unsafe { input(s, n) }))
}

/// 成功で 0、切り出す範囲に不正な UTF-8 があれば -1。
///
/// # Safety
/// `s` は `n` バイト読めること。`out` は書き込める `PcrsBuf` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_truncate_utf8(s: *const u8, n: usize, limit: usize, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    store(out, utf8::truncate(unsafe { input(s, n) }, limit))
}

/// 成功で 0、Unicode のスカラー値でなければ -1。
///
/// # Safety
/// `out` は書き込める `PcrsBuf` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_codepoint_to_utf8(codepoint: u32, out: *mut PcrsBuf) -> i32 {
    store(out, utf8::codepoint_to_utf8(codepoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_roundtrip_through_the_c_abi() {
        // SAFETY: このテストが作ったバッファを、その場で返している
        unsafe {
            let b = pcrs_cgi_escape(b"a b".as_ptr(), 3);
            assert_eq!(std::slice::from_raw_parts(b.ptr, b.len), b"a+b");
            pcrs_buf_free(b);

            // 空の入力 (NULL) と、空の出力
            let b = pcrs_cgi_escape(std::ptr::null(), 0);
            assert_eq!(b.len, 0);
            pcrs_buf_free(b);
            pcrs_buf_free(PcrsBuf { ptr: std::ptr::null_mut(), len: 0 });
        }
    }

    #[test]
    fn fallible_functions_report_errors() {
        // SAFETY: 有効なポインタと長さだけを渡している
        unsafe {
            let mut out = PcrsBuf { ptr: std::ptr::null_mut(), len: 0 };
            assert_eq!(pcrs_str_json_inspect(b"\xff".as_ptr(), 1, &mut out), -1);
            assert!(out.ptr.is_null());
            assert_eq!(pcrs_str_json_inspect(b"a".as_ptr(), 1, &mut out), 0);
            assert_eq!(std::slice::from_raw_parts(out.ptr, out.len), b"\"a\"");
            pcrs_buf_free(out);

            let mut out = PcrsBuf { ptr: std::ptr::null_mut(), len: 0 };
            assert_eq!(pcrs_str_codepoint_to_utf8(0xd800, &mut out), -1);
            assert_eq!(pcrs_str_codepoint_to_utf8(0xa9, &mut out), 0);
            assert_eq!(std::slice::from_raw_parts(out.ptr, out.len), &[0xc2, 0xa9]);
            pcrs_buf_free(out);
        }
    }
}
