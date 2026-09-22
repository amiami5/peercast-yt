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

// ---------------------------------------------------------------- strutil (str.cpp 残り)

use crate::strutil;

macro_rules! two_bytes_to_buf {
    ($name:ident, $f:path) => {
        /// # Safety
        /// `s`/`t` はそれぞれ `sn`/`tn` バイト読めること (0 なら NULL でもよい)。
        #[no_mangle]
        pub unsafe extern "C" fn $name(s: *const u8, sn: usize, t: *const u8, tn: usize) -> PcrsBuf {
            // SAFETY: 関数の Safety 節
            into_buf($f(unsafe { input(s, sn) }, unsafe { input(t, tn) }))
        }
    };
}

bytes_to_buf!(pcrs_str_hexdump, strutil::hexdump);
bytes_to_buf!(pcrs_str_upcase, strutil::upcase);
bytes_to_buf!(pcrs_str_downcase, strutil::downcase);
bytes_to_buf!(pcrs_str_capitalize, strutil::capitalize);
two_bytes_to_buf!(pcrs_str_group_digits, strutil::group_digits);

/// # Safety
/// `s`/`t` はそれぞれ `sn`/`tn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_contains(s: *const u8, sn: usize, t: *const u8, tn: usize) -> bool {
    // SAFETY: 関数の Safety 節
    strutil::contains(unsafe { input(s, sn) }, unsafe { input(t, tn) })
}

/// # Safety
/// `s`/`t` はそれぞれ `sn`/`tn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_has_prefix(s: *const u8, sn: usize, t: *const u8, tn: usize) -> bool {
    // SAFETY: 関数の Safety 節
    strutil::has_prefix(unsafe { input(s, sn) }, unsafe { input(t, tn) })
}

/// # Safety
/// `s`/`t` はそれぞれ `sn`/`tn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_has_suffix(s: *const u8, sn: usize, t: *const u8, tn: usize) -> bool {
    // SAFETY: 関数の Safety 節
    strutil::has_suffix(unsafe { input(s, sn) }, unsafe { input(t, tn) })
}

/// # Safety
/// `s`/`prefix`/`repl` はそれぞれの長さ分読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_replace_prefix(
    s: *const u8, sn: usize, prefix: *const u8, pn: usize, repl: *const u8, rn: usize,
) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    into_buf(strutil::replace_prefix(unsafe { input(s, sn) }, unsafe { input(prefix, pn) }, unsafe { input(repl, rn) }))
}

/// # Safety
/// `s`/`suffix`/`repl` はそれぞれの長さ分読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_replace_suffix(
    s: *const u8, sn: usize, suffix: *const u8, fn_: usize, repl: *const u8, rn: usize,
) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    into_buf(strutil::replace_suffix(unsafe { input(s, sn) }, unsafe { input(suffix, fn_) }, unsafe { input(repl, rn) }))
}

/// # Safety
/// `s` は `n` バイト読めること。`repl` は `rn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_ascii_dump(s: *const u8, n: usize, repl: *const u8, rn: usize) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    into_buf(strutil::ascii_dump(unsafe { input(s, n) }, unsafe { input(repl, rn) }))
}

bytes_to_buf!(pcrs_str_extension_without_dot, strutil::extension_without_dot);
bytes_to_buf!(pcrs_str_rstrip, strutil::rstrip);
bytes_to_buf!(pcrs_str_strip, strutil::strip);
bytes_to_buf!(pcrs_str_escapeshellarg_unix, strutil::escapeshellarg_unix);

/// 長さの一覧と、それらをつなげたバイト列。C++ 側で `size_t` の配列に沿って切り分ける。
#[repr(C)]
pub struct PcrsVec {
    pub joined: PcrsBuf,
    pub lens: *mut usize,
    pub count: usize,
}

fn into_vec(parts: Vec<Vec<u8>>) -> PcrsVec {
    let mut joined = Vec::new();
    let mut lens: Vec<usize> = Vec::with_capacity(parts.len());
    for p in &parts {
        lens.push(p.len());
        joined.extend_from_slice(p);
    }
    let count = lens.len();
    let lens_box = lens.into_boxed_slice();
    let lens_ptr = if count == 0 { std::ptr::null_mut() } else { Box::into_raw(lens_box) as *mut usize };
    PcrsVec { joined: into_buf(joined), lens: lens_ptr, count }
}

/// # Safety
/// `v` は `pcrs_str_*` の関数が返した `PcrsVec` で、まだ解放されていないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_vec_free(v: PcrsVec) {
    // SAFETY: into_buf/into_vec で作ったポインタと長さの組
    unsafe { pcrs_buf_free(v.joined) };
    if !v.lens.is_null() {
        // SAFETY: into_vec で Box<[usize]> として確保したのと同じポインタと長さ
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(v.lens, v.count)) });
    }
}

/// # Safety
/// `s`/`sep` はそれぞれ `sn`/`sepn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_split(s: *const u8, sn: usize, sep: *const u8, sepn: usize) -> PcrsVec {
    // SAFETY: 関数の Safety 節
    into_vec(strutil::split(unsafe { input(s, sn) }, unsafe { input(sep, sepn) }))
}

/// 成功で 0、`limit <= 0` なら -1。
///
/// # Safety
/// `s`/`sep` はそれぞれの長さ分読めること。`out` は書き込める `PcrsVec` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_split_limit(
    s: *const u8, sn: usize, sep: *const u8, sepn: usize, limit: i32, out: *mut PcrsVec,
) -> i32 {
    // SAFETY: 関数の Safety 節
    match strutil::split_limit(unsafe { input(s, sn) }, unsafe { input(sep, sepn) }, limit) {
        Some(parts) => {
            #[allow(unsafe_code)]
            unsafe {
                out.write(into_vec(parts));
            }
            0
        }
        None => -1,
    }
}

/// # Safety
/// `delim` は `dn` バイト読めること。`parts_joined`/`parts_lens` は、それぞれ `parts_count` 個の
/// 部分文字列を、長さの合計が `parts_joined_len` になるよう連結して表す。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_join(
    delim: *const u8, dn: usize, parts_joined: *const u8, parts_joined_len: usize,
    parts_lens: *const usize, parts_count: usize,
) -> PcrsBuf {
    // SAFETY: 呼び出し側が約束する (関数の Safety 節)
    let joined = unsafe { input(parts_joined, parts_joined_len) };
    // SAFETY: 呼び出し側が約束する
    let lens = unsafe { input_usize(parts_lens, parts_count) };
    let mut parts = Vec::with_capacity(parts_count);
    let mut p = 0usize;
    for &len in lens {
        parts.push(joined[p..p + len].to_vec());
        p += len;
    }
    into_buf(strutil::join(unsafe { input(delim, dn) }, &parts))
}

/// # Safety
/// `ptr` は、長さが 0 でなければ、`len` 個の `usize` が読める有効なポインタであること。
unsafe fn input_usize<'a>(ptr: *const usize, len: usize) -> &'a [usize] {
    if len == 0 || ptr.is_null() {
        &[]
    } else {
        // SAFETY: 呼び出し側が保証する (関数の Safety 節)
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

/// # Safety
/// `text` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_to_lines(text: *const u8, n: usize) -> PcrsVec {
    // SAFETY: 関数の Safety 節
    into_vec(strutil::to_lines(unsafe { input(text, n) }))
}

/// 成功で 0、`n < 0` なら -1。
///
/// # Safety
/// `text` は `tn` バイト読めること。`out` は書き込める `PcrsBuf` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_indent_tab(text: *const u8, tn: usize, n: i32, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    store(out, unsafe { input(text, tn) }.pipe_indent(n))
}

trait PipeIndent {
    fn pipe_indent(&self, n: i32) -> Option<Vec<u8>>;
}
impl PipeIndent for [u8] {
    fn pipe_indent(&self, n: i32) -> Option<Vec<u8>> {
        strutil::indent_tab(self, n)
    }
}

/// 成功で 0、`haystack`/`needle` が空文字列なら -1。
///
/// # Safety
/// `h`/`nd` はそれぞれの長さ分読めること。`out` は書き込める `i32` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_count(h: *const u8, hn: usize, nd: *const u8, ndn: usize, out: *mut i32) -> i32 {
    // SAFETY: 関数の Safety 節
    match strutil::count(unsafe { input(h, hn) }, unsafe { input(nd, ndn) }) {
        Some(n) => {
            #[allow(unsafe_code)]
            unsafe {
                out.write(n);
            }
            0
        }
        None => -1,
    }
}

/// 成功 (=単語数) は 0 以上、失敗は -1 で、`*error_kind` にどの種類の構文エラーかを書く
/// (0: 閉じていないシングルクォート, 1: 閉じていないダブルクォート, 2: 末尾のバックスラッシュ)。
///
/// # Safety
/// `s` は `n` バイト読めること。`out`/`error_kind` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_str_shellwords(s: *const u8, n: usize, out: *mut PcrsVec, error_kind: *mut i32) -> i32 {
    // SAFETY: 関数の Safety 節
    match strutil::shellwords(unsafe { input(s, n) }) {
        Ok(words) => {
            #[allow(unsafe_code)]
            unsafe {
                out.write(into_vec(words));
            }
            0
        }
        Err(msg) => {
            let kind = match msg {
                "Unterminated single-quoted string" => 0,
                "Unterminated double-quoted string" => 1,
                _ => 2,
            };
            #[allow(unsafe_code)]
            unsafe {
                error_kind.write(kind);
            }
            -1
        }
    }
}

// ---------------------------------------------------------------- md5

use crate::md5;

bytes_to_buf!(pcrs_md5_hexdigest, md5::hexdigest);

// ---------------------------------------------------------------- gnuid

use crate::gnuid;

/// `id_out` に大文字16進数32文字 (NUL終端なし) を書く。呼び出し側は33バイト以上用意すること。
///
/// # Safety
/// `id` は16バイト、`out` は32バイト書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_gnuid_to_str(id: *const u8, out: *mut u8) {
    // SAFETY: 呼び出し側が約束する (関数の Safety 節)
    let id_arr: [u8; 16] = unsafe { std::slice::from_raw_parts(id, 16) }.try_into().unwrap();
    let s = gnuid::to_str(&id_arr);
    // SAFETY: 呼び出し側が約束する
    unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), out, 32) };
}

/// # Safety
/// `s` は `n` バイト読めること。`out` は16バイト書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_gnuid_from_str(s: *const u8, n: usize, out: *mut u8) {
    // SAFETY: 関数の Safety 節
    let id = gnuid::from_str(unsafe { input(s, n) });
    // SAFETY: 呼び出し側が約束する
    unsafe { std::ptr::copy_nonoverlapping(id.as_ptr(), out, 16) };
}

/// `id` (16バイト、入出力共用) を、IPアドレス (`has_ip` が真なら `ip` の下位4バイトを使う) と
/// ソルトで撹拌する。C++ 版 `GnuID::encode` に相当。
///
/// # Safety
/// `id` は16バイト読み書きできること。`ip` は非NULLなら4バイト読めること。
/// `salt1`/`salt2` はそれぞれの長さ分読めること (長さ0ならNULLでもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_gnuid_encode(
    id: *mut u8, ip: *const u8, has_ip: bool,
    salt1: *const u8, salt1n: usize, salt2: *const u8, salt2n: usize, salt3: u8,
) {
    // SAFETY: 呼び出し側が約束する (関数の Safety 節)
    let mut id_arr: [u8; 16] = unsafe { std::slice::from_raw_parts(id, 16) }.try_into().unwrap();
    let ip_bytes = if has_ip {
        // SAFETY: has_ip が真なら ip は4バイト読める (関数の Safety 節)
        Some(unsafe { std::slice::from_raw_parts(ip, 4) }.try_into().unwrap())
    } else {
        None
    };
    // SAFETY: 関数の Safety 節
    gnuid::encode(&mut id_arr, ip_bytes, unsafe { input(salt1, salt1n) }, unsafe { input(salt2, salt2n) }, salt3);
    // SAFETY: 呼び出し側が約束する
    unsafe { std::ptr::copy_nonoverlapping(id_arr.as_ptr(), id, 16) };
}
