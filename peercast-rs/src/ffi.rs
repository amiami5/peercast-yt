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

// ---------------------------------------------------------------- jis

use crate::jis;

#[no_mangle]
pub extern "C" fn pcrs_jis_sjis_to_unicode(sjis: u16) -> u16 {
    jis::sjis_to_unicode(sjis)
}

#[no_mangle]
pub extern "C" fn pcrs_jis_euc_to_unicode(euc: u16) -> u16 {
    jis::euc_to_unicode(euc)
}

// ---------------------------------------------------------------- String (core/common/_string.cpp)

use crate::pcstring;

macro_rules! bytes_flag_to_buf {
    ($name:ident, $f:path) => {
        /// # Safety
        /// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
        #[no_mangle]
        pub unsafe extern "C" fn $name(s: *const u8, n: usize, flag: bool) -> PcrsBuf {
            // SAFETY: 関数の Safety 節
            into_buf($f(unsafe { input(s, n) }, flag))
        }
    };
}

bytes_flag_to_buf!(pcrs_string_ascii_to_esc, pcstring::ascii_to_esc);
bytes_flag_to_buf!(pcrs_string_ascii_to_meta, pcstring::ascii_to_meta);
bytes_flag_to_buf!(pcrs_string_unknown_to_unicode, pcstring::unknown_to_unicode);
bytes_to_buf!(pcrs_string_esc_to_ascii, pcstring::esc_to_ascii);
bytes_to_buf!(pcrs_string_base64_to_ascii, pcstring::base64_to_ascii);
bytes_to_buf!(pcrs_string_from_string, pcstring::from_string);
bytes_to_buf!(pcrs_string_unquote, pcstring::unquote);

#[no_mangle]
pub extern "C" fn pcrs_string_from_stopwatch(t: u32) -> PcrsBuf {
    into_buf(pcstring::from_stopwatch(t))
}

// ---------------------------------------------------------------- http (core/common/http.cpp)

use crate::http;

/// ステータス行からステータスコードを取り出す。`*cut` には、C++ 版が行を切っていた位置を書く。
///
/// # Safety
/// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。`cut` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_http_parse_status_line(s: *const u8, n: usize, cut: *mut usize) -> i32 {
    // SAFETY: 関数の Safety 節
    let (status, pos) = http::parse_status_line(unsafe { input(s, n) });
    // SAFETY: 関数の Safety 節
    unsafe { *cut = pos };
    status
}

/// ヘッダー行を解析する。`:` があれば真を返し、値の位置・大文字にした名前・値を書く。
///
/// # Safety
/// `s` は `n` バイト読めること。出力引数はすべて書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_http_parse_header_line(
    s: *const u8, n: usize, arg_offset: *mut usize, name: *mut PcrsBuf, value: *mut PcrsBuf,
) -> bool {
    // SAFETY: 関数の Safety 節
    match http::parse_header_line(unsafe { input(s, n) }) {
        Some(h) => {
            // SAFETY: 関数の Safety 節
            unsafe {
                *arg_offset = h.arg_offset;
                *name = into_buf(h.name);
                *value = into_buf(h.value);
            }
            true
        }
        None => false,
    }
}

/// Authorization ヘッダーの値から Basic 認証のユーザー名とパスワードを取り出す。
///
/// # Safety
/// `s` は `n` バイト読めること。`user`, `pass` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_http_parse_basic_auth(
    s: *const u8, n: usize, user: *mut PcrsBuf, pass: *mut PcrsBuf,
) -> bool {
    // SAFETY: 関数の Safety 節
    match http::parse_basic_auth(unsafe { input(s, n) }) {
        Some((u, p)) => {
            // SAFETY: 関数の Safety 節
            unsafe {
                *user = into_buf(u);
                *pass = into_buf(p);
            }
            true
        }
        None => false,
    }
}

/// # Safety
/// 各入力は、それぞれの長さ分読めること (長さ 0 なら NULL でもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_http_is_cross_origin_request(
    site: *const u8, site_n: usize, origin: *const u8, origin_n: usize, host: *const u8, host_n: usize,
) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe { http::is_cross_origin_request(input(site, site_n), input(origin, origin_n), input(host, host_n)) }
}

bytes_to_bool!(pcrs_http_is_loopback_host_header, http::is_loopback_host_header);

/// # Safety
/// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cgi_parse_http_date(s: *const u8, n: usize) -> i64 {
    // SAFETY: 関数の Safety 節
    http::parse_http_date(unsafe { input(s, n) })
}

// ---------------------------------------------------------------- Stream から読む (reader.rs)

use crate::reader::{Abort, Reader};

/// C++ の `Stream` を読むコールバック。どれも成功で 0、C++ の例外で中断したら -1 を返す
/// (例外は C++ 側で保存しておき、Rust の関数から戻ったあとに投げ直す)。
#[repr(C)]
pub struct CReader {
    pub ctx: *mut std::ffi::c_void,
    /// `Stream::readChar`
    pub read_char: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, out: *mut u8) -> i32,
    /// `Stream::read(int)`: ちょうど n バイト
    pub read_exact: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, buf: *mut u8, n: usize) -> i32,
    /// `Stream::read(void*, int)`: 最大 n バイト。読めたバイト数を *got に書く
    pub read_some: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, buf: *mut u8, n: usize, got: *mut usize) -> i32,
    /// `Stream::eof`
    pub eof: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, out: *mut bool) -> i32,
}

/// `CReader` を `Reader` として使う。
struct CReaderRef<'a>(&'a CReader);

impl Reader for CReaderRef<'_> {
    fn read_char(&mut self) -> Result<u8, Abort> {
        let mut c = 0u8;
        // SAFETY: CReader を渡した C++ 側が、関数ポインタと ctx の有効性を保証する
        match unsafe { (self.0.read_char)(self.0.ctx, &mut c) } {
            0 => Ok(c),
            _ => Err(Abort),
        }
    }

    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        let mut v = vec![0u8; n];
        // SAFETY: 同上。v は n バイト書ける
        match unsafe { (self.0.read_exact)(self.0.ctx, v.as_mut_ptr(), n) } {
            0 => Ok(v),
            _ => Err(Abort),
        }
    }

    fn read_some(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        let mut v = vec![0u8; n];
        let mut got = 0usize;
        // SAFETY: 同上。v は n バイト書ける
        match unsafe { (self.0.read_some)(self.0.ctx, v.as_mut_ptr(), n, &mut got) } {
            0 => {
                v.truncate(got.min(n));
                Ok(v)
            }
            _ => Err(Abort),
        }
    }

    fn eof(&mut self) -> Result<bool, Abort> {
        let mut e = false;
        // SAFETY: CReader を渡した C++ 側が、関数ポインタと ctx の有効性を保証する
        match unsafe { (self.0.eof)(self.0.ctx, &mut e) } {
            0 => Ok(e),
            _ => Err(Abort),
        }
    }
}

/// # Safety
/// `r` は有効な `CReader` を指すこと。
unsafe fn reader<'a>(r: *const CReader) -> CReaderRef<'a> {
    // SAFETY: 呼び出し側が保証する
    CReaderRef(unsafe { &*r })
}

// ---------------------------------------------------------------- amf0 (core/common/amf0.cpp)

use crate::amf0;

/// AMF0 の値を組み立てる C++ 側のコールバック (`amf0::Builder` の通知をそのまま渡す)。
#[repr(C)]
pub struct CAmf0Builder {
    pub ctx: *mut std::ffi::c_void,
    pub number: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, v: f64),
    pub string: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, s: *const u8, n: usize),
    pub boolean: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, b: bool),
    pub null: unsafe extern "C" fn(ctx: *mut std::ffi::c_void),
    pub date: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, unix_time: f64, timezone: u16),
    /// kind: 0 = オブジェクト、1 = ECMA 配列
    pub begin_object: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, kind: i32),
    pub key: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, s: *const u8, n: usize),
    pub end_object: unsafe extern "C" fn(ctx: *mut std::ffi::c_void),
    pub begin_strict_array: unsafe extern "C" fn(ctx: *mut std::ffi::c_void),
    pub end_strict_array: unsafe extern "C" fn(ctx: *mut std::ffi::c_void),
}

struct CBuilderRef<'a>(&'a CAmf0Builder);

// SAFETY (この impl のすべての unsafe): CAmf0Builder を渡した C++ 側が、関数ポインタと ctx の
// 有効性を保証する。文字列は呼び出しの間だけ有効なスライスを渡す。
impl amf0::Builder for CBuilderRef<'_> {
    fn number(&mut self, v: f64) { unsafe { (self.0.number)(self.0.ctx, v) } }
    fn string(&mut self, s: &[u8]) { unsafe { (self.0.string)(self.0.ctx, s.as_ptr(), s.len()) } }
    fn boolean(&mut self, b: bool) { unsafe { (self.0.boolean)(self.0.ctx, b) } }
    fn null(&mut self) { unsafe { (self.0.null)(self.0.ctx) } }
    fn date(&mut self, t: f64, tz: u16) { unsafe { (self.0.date)(self.0.ctx, t, tz) } }
    fn begin_object(&mut self, kind: amf0::ObjectKind) {
        let k = match kind {
            amf0::ObjectKind::Object => 0,
            amf0::ObjectKind::Array => 1,
        };
        unsafe { (self.0.begin_object)(self.0.ctx, k) }
    }
    fn key(&mut self, k: &[u8]) { unsafe { (self.0.key)(self.0.ctx, k.as_ptr(), k.len()) } }
    fn end_object(&mut self) { unsafe { (self.0.end_object)(self.0.ctx) } }
    fn begin_strict_array(&mut self) { unsafe { (self.0.begin_strict_array)(self.0.ctx) } }
    fn end_strict_array(&mut self) { unsafe { (self.0.end_strict_array)(self.0.ctx) } }
}

/// 結果を C の値にする: 0 成功、1 読み出しの中断、2 深すぎる、3 値が多すぎる、4 不明な型
/// (型を `*unknown_type` に書く)。
fn amf0_result(res: Result<(), amf0::Error>, unknown_type: *mut i8) -> i32 {
    match res {
        Ok(()) => 0,
        Err(amf0::Error::Abort) => 1,
        Err(amf0::Error::TooDeep) => 2,
        Err(amf0::Error::TooMany) => 3,
        Err(amf0::Error::UnknownType(t)) => {
            // SAFETY: 呼び出し側 (下の関数の Safety 節) が保証する
            unsafe { *unknown_type = t };
            4
        }
    }
}

/// `Deserializer::readValue`
///
/// # Safety
/// `r`, `b` は有効な構造体を指すこと。`unknown_type` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_amf0_read_value(r: *const CReader, b: *const CAmf0Builder, unknown_type: *mut i8) -> i32 {
    // SAFETY: 関数の Safety 節
    let (mut rd, mut bd) = unsafe { (reader(r), CBuilderRef(&*b)) };
    amf0_result(amf0::read_value(&mut rd, &mut bd), unknown_type)
}

/// `Deserializer::readObject` (型のバイトのないオブジェクト。`begin_object` から通知する)
///
/// # Safety
/// `pcrs_amf0_read_value` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_amf0_read_object(r: *const CReader, b: *const CAmf0Builder, unknown_type: *mut i8) -> i32 {
    // SAFETY: 関数の Safety 節
    let (mut rd, mut bd) = unsafe { (reader(r), CBuilderRef(&*b)) };
    amf0_result(amf0::read_object(&mut rd, &mut bd), unknown_type)
}

macro_rules! amf0_primitive {
    ($name:ident, $f:path, $t:ty) => {
        /// 成功で 0、読み出しの中断で -1。
        ///
        /// # Safety
        /// `r` は有効な `CReader` を指し、`out` は書き込めること。
        #[no_mangle]
        pub unsafe extern "C" fn $name(r: *const CReader, out: *mut $t) -> i32 {
            // SAFETY: 関数の Safety 節
            let mut rd = unsafe { reader(r) };
            match $f(&mut rd) {
                // SAFETY: 関数の Safety 節
                Ok(v) => { unsafe { *out = v }; 0 }
                Err(Abort) => -1,
            }
        }
    };
}

amf0_primitive!(pcrs_amf0_read_bool, amf0::read_bool, bool);
amf0_primitive!(pcrs_amf0_read_int32, amf0::read_int32, i32);
amf0_primitive!(pcrs_amf0_read_int16, amf0::read_int16, i16);
amf0_primitive!(pcrs_amf0_read_double, amf0::read_double, f64);

/// `Deserializer::readString`。成功で 0、読み出しの中断で -1。
///
/// # Safety
/// `r` は有効な `CReader` を指し、`out` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_amf0_read_string(r: *const CReader, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let mut rd = unsafe { reader(r) };
    match amf0::read_string(&mut rd) {
        // SAFETY: 関数の Safety 節
        Ok(v) => { unsafe { *out = into_buf(v) }; 0 }
        Err(Abort) => -1,
    }
}

// ---------------------------------------------------------------- xml (core/common/xml.cpp)

use crate::xml;

/// XML の要素を受け取る C++ 側のコールバック。どれも成功で 0、C++ の例外で中断したら -1。
#[repr(C)]
pub struct CXmlBuilder {
    pub ctx: *mut std::ffi::c_void,
    pub content: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, s: *const u8, n: usize) -> i32,
    pub start_tag: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, s: *const u8, n: usize, single: bool) -> i32,
    pub end_tag: unsafe extern "C" fn(ctx: *mut std::ffi::c_void) -> i32,
}

struct CXmlBuilderRef<'a>(&'a CXmlBuilder);

fn ok_if_zero(r: i32) -> Result<(), ()> {
    if r == 0 { Ok(()) } else { Err(()) }
}

// SAFETY (この impl のすべての unsafe): CXmlBuilder を渡した C++ 側が、関数ポインタと ctx の
// 有効性を保証する。文字列は呼び出しの間だけ有効なスライスを渡す。
impl xml::Builder for CXmlBuilderRef<'_> {
    fn content(&mut self, s: &[u8]) -> Result<(), ()> {
        ok_if_zero(unsafe { (self.0.content)(self.0.ctx, s.as_ptr(), s.len()) })
    }
    fn start_tag(&mut self, s: &[u8], single: bool) -> Result<(), ()> {
        ok_if_zero(unsafe { (self.0.start_tag)(self.0.ctx, s.as_ptr(), s.len(), single) })
    }
    fn end_tag(&mut self) -> Result<(), ()> {
        ok_if_zero(unsafe { (self.0.end_tag)(self.0.ctx) })
    }
}

/// `XML::read`。0 成功、1 読み出しの中断、2 通知先の中断、3 "Tag too long"、
/// 4 "Content too big"、5 "Not XML document"、6 "Unexpected end tag"。
///
/// # Safety
/// `r`, `b` は有効な構造体を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_xml_read(r: *const CReader, b: *const CXmlBuilder) -> i32 {
    // SAFETY: 関数の Safety 節
    let (mut rd, mut bd) = unsafe { (reader(r), CXmlBuilderRef(&*b)) };
    match xml::read(&mut rd, &mut bd) {
        Ok(()) => 0,
        Err(xml::Error::Abort) => 1,
        Err(xml::Error::Callback) => 2,
        Err(xml::Error::TagTooLong) => 3,
        Err(xml::Error::ContentTooBig) => 4,
        Err(xml::Error::NotXml) => 5,
        Err(xml::Error::UnexpectedEndTag) => 6,
    }
}

/// `XML::Node::setAttributes`。0 成功、1 "Too many attributes"、2 "Bad tag value"。
///
/// 成功したら、`*data` に C++ 版の `attrData` の中身 (終端の NUL を含まない) を、`positions` に
/// (名前の位置, 値の位置) を属性の数だけ並べて書き、属性の数 (タグ名を含む) を `*count` に書く。
///
/// # Safety
/// `s` は `n` バイト読めること。`positions` は `2 * (n + 1)` 個の `size_t` を書き込めること
/// (属性の数は `n + 1` を超えない)。`data`, `count` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_xml_parse_attributes(
    s: *const u8, n: usize, data: *mut PcrsBuf, positions: *mut usize, count: *mut usize,
) -> i32 {
    // SAFETY: 関数の Safety 節
    match xml::parse_attributes(unsafe { input(s, n) }) {
        Ok(a) => {
            debug_assert!(a.attrs.len() <= n + 1);
            // SAFETY: 関数の Safety 節 (positions は 2 * (n + 1) 個書ける)
            unsafe {
                for (i, &(name, value)) in a.attrs.iter().enumerate() {
                    *positions.add(2 * i) = name;
                    *positions.add(2 * i + 1) = value;
                }
                *count = a.attrs.len();
                *data = into_buf(a.data);
            }
            0
        }
        Err(xml::AttrError::TooMany) => 1,
        Err(xml::AttrError::BadValue) => 2,
    }
}

/// `XML::Node::getBinaryContent`。0 成功 (`*out` に中身)、-1 "Too much binary data"。
///
/// # Safety
/// `s` は `n` バイト読めること。`out` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_xml_binary_content(s: *const u8, n: usize, size: usize, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    store(out, xml::binary_content(unsafe { input(s, n) }, size))
}

// ---------------------------------------------------------------- URL (LUrlParser.cpp, url.cpp)

/// `LUrlParser::clParseURL::ParseURL`。成功で 0 を返し、`*out` に 8 つの部分
/// (scheme, host, port, path, query, fragment, user_name, password の順) を書く。
/// 失敗なら `LUrlParserError` の値 (2〜5) を返し、`*out` には何も書かない。
///
/// # Safety
/// `s` は `n` バイト読めること。`out` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_url_parse(s: *const u8, n: usize, out: *mut PcrsVec) -> i32 {
    // SAFETY: 関数の Safety 節
    match url::parse_url(unsafe { input(s, n) }) {
        Ok(u) => {
            let parts = vec![u.scheme, u.host, u.port, u.path, u.query, u.fragment, u.user_name, u.password];
            // SAFETY: 関数の Safety 節
            unsafe { *out = into_vec(parts) };
            0
        }
        Err(e) => e as i32,
    }
}

/// `clParseURL::GetPort` の数値化。1〜65535 ならその値、そうでなければ 0。
///
/// # Safety
/// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_url_port_number(s: *const u8, n: usize) -> i32 {
    // SAFETY: 関数の Safety 節
    url::port_number(unsafe { input(s, n) }).map_or(0, i32::from)
}

/// `URLSource::getSourceProtocol`。`ChanInfo::PROTOCOL` の値を返し、読み飛ばす長さを `*skip` に書く。
///
/// # Safety
/// `s` は `n` バイト読めること。`skip` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_url_source_protocol(s: *const u8, n: usize, skip: *mut usize) -> i32 {
    // SAFETY: 関数の Safety 節
    let (proto, len) = url::source_protocol(unsafe { input(s, n) });
    // SAFETY: 関数の Safety 節
    unsafe { *skip = len };
    proto as i32
}

// ---------------------------------------------------------------- dechunker (core/common/dechunker.cpp)

use crate::dechunk;

/// `Dechunker::getNextChunk`。チャンクの中身を `*data` に書き (空のこともある)、その後に起きた
/// エラーを返す: 0 なし、1 読み出しの中断、2 "Protocol error"、3 "Chunk size too large"、
/// 4 最後のチャンク ("Closed on read")、5 "Premature end"。
///
/// # Safety
/// `r` は有効な `CReader` を指し、`data` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_dechunk_next(r: *const CReader, max_chunk_size: usize, data: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let mut rd = unsafe { reader(r) };
    let (d, err) = dechunk::next_chunk(&mut rd, max_chunk_size);
    // SAFETY: 関数の Safety 節
    unsafe { *data = into_buf(d) };
    match err {
        None => 0,
        Some(dechunk::Error::Abort) => 1,
        Some(dechunk::Error::Protocol) => 2,
        Some(dechunk::Error::TooLarge) => 3,
        Some(dechunk::Error::Closed) => 4,
        Some(dechunk::Error::Premature) => 5,
    }
}

/// base64 の 4 文字を 3 バイトに復号して `out` に書き、書いたバイト数 (3、不正なら 0) を返す。
///
/// # Safety
/// `word` は 4 バイト読めること。`out` は 3 バイト書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_base64_word_to_chars(word: *const u8, out: *mut u8) -> i32 {
    // SAFETY: 関数の Safety 節
    let w: [u8; 4] = unsafe { std::slice::from_raw_parts(word, 4) }.try_into().unwrap();
    match pcstring::base64_word_to_chars(&w) {
        Some(bytes) => {
            // SAFETY: 関数の Safety 節
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, 3) };
            3
        }
        None => 0,
    }
}
