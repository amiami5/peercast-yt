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

// ---------------------------------------------------------------- メディアコンテナ (src/media)

use crate::media::{self, HeadKind, Host, LogLevel, Parser, Track};
use std::ffi::c_void;

/// `Track` の 1 項目 (`present` が false なら値なし)
#[repr(C)]
pub struct CTrackField {
    pub ptr: *const u8,
    pub len: usize,
    pub present: bool,
}

#[repr(C)]
pub struct CTrack {
    pub artist: CTrackField,
    pub title: CTrackField,
    pub genre: CTrackField,
    pub contact: CTrackField,
    pub album: CTrackField,
}

/// 解析器から見たチャンネルと入力 (C の `pcrs_media_host`)。int を返すものは、成功で 0、
/// C++ の例外で中断したら -1 (例外は C++ 側で保存しておき、戻ったあとで投げ直す)。
#[repr(C)]
pub struct CMediaHost {
    pub ctx: *mut c_void,
    pub reader: *const CReader,
    pub ready: unsafe extern "C" fn(ctx: *mut c_void, out: *mut bool) -> i32,
    pub raise_bitrate: unsafe extern "C" fn(ctx: *mut c_void) -> i32,
    pub packet: unsafe extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize, cont: bool, read_delay: bool) -> i32,
    pub head: unsafe extern "C" fn(ctx: *mut c_void, kind: i32, data: *const u8, len: usize) -> i32,
    pub head_len: unsafe extern "C" fn(ctx: *mut c_void) -> u32,
    pub head_clear: unsafe extern "C" fn(ctx: *mut c_void),
    pub head_append: unsafe extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize) -> i32,
    pub set_bitrate: unsafe extern "C" fn(ctx: *mut c_void, bitrate: i32) -> i32,
    pub ogg_set_info: unsafe extern "C" fn(ctx: *mut c_void, bitrate: i32, ogm: bool),
    pub set_track: unsafe extern "C" fn(ctx: *mut c_void, track: *const CTrack) -> i32,
    pub mp3_metadata: unsafe extern "C" fn(ctx: *mut c_void, buf: *const u8, len: usize) -> i32,
    pub icy_meta_interval: unsafe extern "C" fn(ctx: *mut c_void) -> i32,
    pub read_delay: unsafe extern "C" fn(ctx: *mut c_void) -> bool,
    pub dtime: unsafe extern "C" fn(ctx: *mut c_void) -> f64,
    pub time: unsafe extern "C" fn(ctx: *mut c_void) -> u32,
    pub sleep: unsafe extern "C" fn(ctx: *mut c_void, ms: i32),
    pub sleep_until: unsafe extern "C" fn(ctx: *mut c_void, t: f64),
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: i32, msg: *const u8, len: usize),
}

/// `CMediaHost` を `Host` として使う。
struct CMediaHostRef<'a> {
    h: &'a CMediaHost,
    r: CReaderRef<'a>,
}

fn status(code: i32) -> Result<(), Abort> {
    if code == 0 {
        Ok(())
    } else {
        Err(Abort)
    }
}

// 以下の unsafe ブロックはどれも、CMediaHost を渡した C++ 側が関数ポインタと ctx の有効性を
// 保証していることに頼る。渡すスライスは呼び出しの間だけ有効。
impl Reader for CMediaHostRef<'_> {
    fn read_char(&mut self) -> Result<u8, Abort> {
        self.r.read_char()
    }
    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        self.r.read_exact(n)
    }
    fn read_some(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        self.r.read_some(n)
    }
    fn eof(&mut self) -> Result<bool, Abort> {
        self.r.eof()
    }
}

impl Host for CMediaHostRef<'_> {
    fn ready(&mut self) -> Result<bool, Abort> {
        let mut out = false;
        // SAFETY: 上記
        status(unsafe { (self.h.ready)(self.h.ctx, &mut out) })?;
        Ok(out)
    }
    fn raise_bitrate(&mut self) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.raise_bitrate)(self.h.ctx) })
    }
    fn packet(&mut self, data: &[u8], cont: bool, read_delay: bool) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.packet)(self.h.ctx, data.as_ptr(), data.len(), cont, read_delay) })
    }
    fn head(&mut self, kind: HeadKind, data: &[u8]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.head)(self.h.ctx, kind as i32, data.as_ptr(), data.len()) })
    }
    fn head_len(&mut self) -> u32 {
        // SAFETY: 上記
        unsafe { (self.h.head_len)(self.h.ctx) }
    }
    fn head_clear(&mut self) {
        // SAFETY: 上記
        unsafe { (self.h.head_clear)(self.h.ctx) }
    }
    fn head_append(&mut self, data: &[u8]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.head_append)(self.h.ctx, data.as_ptr(), data.len()) })
    }
    fn set_bitrate(&mut self, bitrate: i32) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.set_bitrate)(self.h.ctx, bitrate) })
    }
    fn ogg_set_info(&mut self, bitrate: i32, ogm: bool) {
        // SAFETY: 上記
        unsafe { (self.h.ogg_set_info)(self.h.ctx, bitrate, ogm) }
    }
    fn set_track(&mut self, t: &Track) -> Result<(), Abort> {
        let field = |f: &Option<Vec<u8>>| match f {
            Some(v) => CTrackField { ptr: v.as_ptr(), len: v.len(), present: true },
            None => CTrackField { ptr: std::ptr::null(), len: 0, present: false },
        };
        let c = CTrack { artist: field(&t.artist), title: field(&t.title), genre: field(&t.genre), contact: field(&t.contact), album: field(&t.album) };
        // SAFETY: 上記 (c と t はこの呼び出しの間有効)
        status(unsafe { (self.h.set_track)(self.h.ctx, &c) })
    }
    fn mp3_metadata(&mut self, buf: &[u8]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.mp3_metadata)(self.h.ctx, buf.as_ptr(), buf.len()) })
    }
    fn icy_meta_interval(&mut self) -> i32 {
        // SAFETY: 上記
        unsafe { (self.h.icy_meta_interval)(self.h.ctx) }
    }
    fn read_delay(&mut self) -> bool {
        // SAFETY: 上記
        unsafe { (self.h.read_delay)(self.h.ctx) }
    }
    fn dtime(&mut self) -> f64 {
        // SAFETY: 上記
        unsafe { (self.h.dtime)(self.h.ctx) }
    }
    fn time(&mut self) -> u32 {
        // SAFETY: 上記
        unsafe { (self.h.time)(self.h.ctx) }
    }
    fn sleep(&mut self, ms: i32) {
        // SAFETY: 上記
        unsafe { (self.h.sleep)(self.h.ctx, ms) }
    }
    fn sleep_until(&mut self, t: f64) {
        // SAFETY: 上記
        unsafe { (self.h.sleep_until)(self.h.ctx, t) }
    }
    fn log(&mut self, level: LogLevel, msg: &str) {
        // SAFETY: 上記
        unsafe { (self.h.log)(self.h.ctx, level as i32, msg.as_ptr(), msg.len()) }
    }
}

/// 解析器を作る。`kind` は `PCRS_MEDIA_*` (1: MP3、2: FLV、3: OGG、4: MKV、5: MP4)。
/// 知らない値なら NULL。`pcrs_media_free` で解放する。
#[no_mangle]
pub extern "C" fn pcrs_media_new(kind: i32) -> *mut Parser {
    match media::Kind::from_i32(kind) {
        Some(k) => Box::into_raw(Box::new(Parser::new(k))),
        None => std::ptr::null_mut(),
    }
}

/// # Safety
/// `p` は `pcrs_media_new` が返した値 (または NULL) で、まだ解放されていないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_media_free(p: *mut Parser) {
    if !p.is_null() {
        // SAFETY: 関数の Safety 節
        drop(unsafe { Box::from_raw(p) });
    }
}

/// 0 成功、1 コールバックの中断、2 エラー (メッセージを `*err` に書く)
fn media_result(r: media::Result<()>, err: *mut PcrsBuf) -> i32 {
    match r {
        Ok(()) => 0,
        Err(media::Error::Abort) => 1,
        Err(media::Error::Stream(msg)) => {
            // SAFETY: 呼び出し元 (pcrs_media_read_*) の Safety 節
            unsafe { *err = into_buf(msg.into_bytes()) };
            2
        }
    }
}

/// `readHeader`。返り値は 0 成功、1 コールバックの中断、2 エラー (`StreamException` のメッセージを
/// `*err` に書く)。
///
/// # Safety
/// `p` は `pcrs_media_new` が返した有効な値、`host` は有効な `CMediaHost` (その `reader` も有効)
/// を指し、`err` は書き込めること。同じ `p` を同時に複数のスレッドから使わないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_media_read_header(p: *mut Parser, host: *const CMediaHost, err: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let (parser, h) = unsafe { (&mut *p, &*host) };
    // SAFETY: 同上
    let mut hr = CMediaHostRef { h, r: unsafe { reader(h.reader) } };
    media_result(parser.read_header(&mut hr), err)
}

/// `readPacket`。返り値は `pcrs_media_read_header` と同じ。
///
/// # Safety
/// `pcrs_media_read_header` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_media_read_packet(p: *mut Parser, host: *const CMediaHost, err: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let (parser, h) = unsafe { (&mut *p, &*host) };
    // SAFETY: 同上
    let mut hr = CMediaHostRef { h, r: unsafe { reader(h.reader) } };
    media_result(parser.read_packet(&mut hr), err)
}

/// `FLVStream::readMetaData`。onMetaData でビットレートがあれば 1 (値を `*bitrate` に書く)、
/// なければ 0、形式が壊れていれば 2 (理由を `*err` に書く)。
///
/// # Safety
/// `data` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。`bitrate` と `err` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_flv_read_meta_data(data: *const u8, n: usize, bitrate: *mut i32, err: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    match media::flv::read_meta_data(unsafe { input(data, n) }) {
        Ok(Some(b)) => {
            // SAFETY: 関数の Safety 節
            unsafe { *bitrate = b };
            1
        }
        Ok(None) => 0,
        Err(msg) => {
            // SAFETY: 関数の Safety 節
            unsafe { *err = into_buf(msg.into_bytes()) };
            2
        }
    }
}

// ---------------------------------------------------------------- テンプレート (src/template)

use crate::template::{self, value as tvalue, Engine, Value as TValue};
use std::collections::VecDeque;

/// テンプレートから見た C++ 側 (C の `pcrs_template_host`)。int を返すものは、成功で 0、
/// C++ の例外で中断したら -1。
#[repr(C)]
pub struct CTemplateHost {
    pub ctx: *mut c_void,
    /// テンプレートの Stream (read_char と eof を使う)。ディレクティブを読まない呼び出しでは NULL
    pub reader: *const CReader,
    pub position: unsafe extern "C" fn(ctx: *mut c_void, out: *mut i32) -> i32,
    pub seek: unsafe extern "C" fn(ctx: *mut c_void, pos: i32) -> i32,
    pub write: unsafe extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize) -> i32,
    /// 値は、次のコールバックまで有効な C++ 側のバッファを指す
    pub lookup: unsafe extern "C" fn(ctx: *mut c_void, name: *const u8, n: usize, value: *mut *const u8, value_len: *mut usize) -> i32,
    pub push_scope: unsafe extern "C" fn(ctx: *mut c_void),
    pub pop_scope: unsafe extern "C" fn(ctx: *mut c_void),
    pub front_is_generic: unsafe extern "C" fn(ctx: *mut c_void) -> bool,
    pub set_front: unsafe extern "C" fn(ctx: *mut c_void, name: *const u8, n: usize, value: *const u8, value_len: usize) -> i32,
    pub regex_check: unsafe extern "C" fn(ctx: *mut c_void, pattern: *const u8, n: usize) -> i32,
    pub regex_match: unsafe extern "C" fn(ctx: *mut c_void, pattern: *const u8, n: usize, subject: *const u8, m: usize, out: *mut bool) -> i32,
    pub selected_fragment: unsafe extern "C" fn(ctx: *mut c_void, out: *mut *const u8, n: *mut usize),
    pub current_fragment: unsafe extern "C" fn(ctx: *mut c_void, out: *mut *const u8, n: *mut usize),
    pub set_current_fragment: unsafe extern "C" fn(ctx: *mut c_void, f: *const u8, n: usize),
    pub log_error: unsafe extern "C" fn(ctx: *mut c_void, msg: *const u8, n: usize),
}

struct CTemplateHostRef<'a> {
    h: &'a CTemplateHost,
    r: Option<CReaderRef<'a>>,
}

impl CTemplateHostRef<'_> {
    fn fragment(&self, f: unsafe extern "C" fn(*mut c_void, *mut *const u8, *mut usize)) -> Vec<u8> {
        let (mut p, mut n) = (std::ptr::null(), 0usize);
        // SAFETY: C++ 側が有効なポインタと長さを書く (次のコールバックまで有効)
        unsafe {
            f(self.h.ctx, &mut p, &mut n);
            input(p, n).to_vec()
        }
    }
}

// 以下の unsafe ブロックはどれも、CTemplateHost を渡した C++ 側が関数ポインタと ctx の有効性を
// 保証していることに頼る。渡すスライスは呼び出しの間だけ有効。
impl template::Host for CTemplateHostRef<'_> {
    fn read_char(&mut self) -> Result<u8, Abort> {
        self.r.as_mut().ok_or(Abort)?.read_char()
    }
    fn eof(&mut self) -> Result<bool, Abort> {
        self.r.as_mut().ok_or(Abort)?.eof()
    }
    fn position(&mut self) -> Result<i32, Abort> {
        let mut out = 0;
        // SAFETY: 上記
        status(unsafe { (self.h.position)(self.h.ctx, &mut out) })?;
        Ok(out)
    }
    fn seek(&mut self, pos: i32) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.seek)(self.h.ctx, pos) })
    }
    fn write(&mut self, data: &[u8]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.write)(self.h.ctx, data.as_ptr(), data.len()) })
    }
    fn lookup(&mut self, name: &[u8]) -> Result<TValue, Abort> {
        let (mut p, mut n) = (std::ptr::null(), 0usize);
        // SAFETY: 上記。値は次のコールバックまで有効なので、すぐに読む
        status(unsafe { (self.h.lookup)(self.h.ctx, name.as_ptr(), name.len(), &mut p, &mut n) })?;
        // SAFETY: 同上
        tvalue::decode(unsafe { input(p, n) }).ok_or(Abort)
    }
    fn push_scope(&mut self) {
        // SAFETY: 上記
        unsafe { (self.h.push_scope)(self.h.ctx) }
    }
    fn pop_scope(&mut self) {
        // SAFETY: 上記
        unsafe { (self.h.pop_scope)(self.h.ctx) }
    }
    fn front_is_generic(&mut self) -> bool {
        // SAFETY: 上記
        unsafe { (self.h.front_is_generic)(self.h.ctx) }
    }
    fn set_front(&mut self, name: &[u8], value: &TValue) -> Result<(), Abort> {
        let v = tvalue::encoded(value);
        // SAFETY: 上記
        status(unsafe { (self.h.set_front)(self.h.ctx, name.as_ptr(), name.len(), v.as_ptr(), v.len()) })
    }
    fn regex_check(&mut self, pattern: &[u8]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.h.regex_check)(self.h.ctx, pattern.as_ptr(), pattern.len()) })
    }
    fn regex_match(&mut self, pattern: &[u8], subject: &[u8]) -> Result<bool, Abort> {
        let mut out = false;
        // SAFETY: 上記
        status(unsafe {
            (self.h.regex_match)(self.h.ctx, pattern.as_ptr(), pattern.len(), subject.as_ptr(), subject.len(), &mut out)
        })?;
        Ok(out)
    }
    fn selected_fragment(&mut self) -> Vec<u8> {
        self.fragment(self.h.selected_fragment)
    }
    fn current_fragment(&mut self) -> Vec<u8> {
        self.fragment(self.h.current_fragment)
    }
    fn set_current_fragment(&mut self, f: &[u8]) {
        // SAFETY: 上記
        unsafe { (self.h.set_current_fragment)(self.h.ctx, f.as_ptr(), f.len()) }
    }
    fn log_error(&mut self, msg: &str) {
        // SAFETY: 上記
        unsafe { (self.h.log_error)(self.h.ctx, msg.as_ptr(), msg.len()) }
    }
}

fn tokens_value(tokens: VecDeque<Vec<u8>>) -> TValue {
    TValue::StrictArray(tokens.into_iter().map(TValue::String).collect())
}

fn value_tokens(v: &TValue) -> template::Result<VecDeque<Vec<u8>>> {
    v.strict_array()?.iter().map(|t| Ok(t.string()?.to_vec())).collect()
}

/// ホストを使わない処理 (字句解析、構文解析、文字列リテラル)
fn template_pure(op: i32, arg: &TValue) -> Option<template::Result<TValue>> {
    fn tokenize(arg: &TValue) -> template::Result<TValue> {
        Ok(TValue::StrictArray(template::tokenize(arg.string()?)?.into_iter().map(TValue::String).collect()))
    }
    fn parse(arg: &TValue) -> template::Result<TValue> {
        let mut t = value_tokens(arg)?;
        let v = template::parse(&mut t)?;
        Ok(TValue::StrictArray(vec![v, tokens_value(t)]))
    }
    fn parse_let_spec(arg: &TValue) -> template::Result<TValue> {
        let mut t = value_tokens(arg)?;
        let spec = template::parse_let_spec(&mut t)?;
        let spec = spec.into_iter().map(|(n, e)| TValue::StrictArray(vec![TValue::String(n), e])).collect();
        Ok(TValue::StrictArray(vec![TValue::StrictArray(spec), tokens_value(t)]))
    }
    fn read_string_literal(arg: &TValue) -> template::Result<TValue> {
        let (lit, rest) = template::read_string_literal(arg.string()?)?;
        Ok(TValue::StrictArray(vec![TValue::String(lit), TValue::String(rest)]))
    }
    fn eval_string_literal(arg: &TValue) -> template::Result<TValue> {
        Ok(TValue::String(template::eval_string_literal(arg.string()?)?))
    }
    let f: fn(&TValue) -> template::Result<TValue> = match op {
        17 => tokenize,
        18 => parse,
        19 => parse_let_spec,
        20 => read_string_literal,
        21 => eval_string_literal,
        _ => return None,
    };
    Some(f(arg))
}

/// ホストを使う処理
fn template_with_host(op: i32, arg: &TValue, h: &mut dyn template::Host) -> Option<template::Result<TValue>> {
    let mut e = Engine::new(h);
    let flag = matches!(arg, TValue::Bool(true));
    let r: template::Result<TValue> = match op {
        1 => e.read_template(flag).map(|t| TValue::Number(t as f64)),
        2 => e.read_cmd(flag).map(|t| TValue::Number(t as f64)),
        3 => e.read_if(flag).map(|_| TValue::Null),
        4 => e.read_loop(flag).map(|_| TValue::Null),
        5 => e.read_foreach(flag).map(|_| TValue::Null),
        6 => e.read_let(flag).map(|_| TValue::Null),
        7 => e.read_fragment(flag).map(|_| TValue::Null),
        8 => e.read_variable_value(flag).map(|s| TValue::StrictArray(s.into_iter().map(TValue::String).collect())),
        9 => arg.string().map(|s| s.to_vec()).and_then(|s| e.eval_str(&s)),
        10 => e.eval(arg),
        11 => arg.strict_array().map(|a| a.to_vec()).and_then(|a| e.eval_form(&a)),
        12 => arg.string().map(|s| s.to_vec()).and_then(|s| e.eval_condition(&s)).map(TValue::Bool),
        13 => arg.string().map(|s| s.to_vec()).and_then(|s| e.get_int_variable(&s)).map(|n| TValue::Number(n as f64)),
        14 => arg.string().map(|s| s.to_vec()).and_then(|s| e.get_bool_variable(&s)).map(TValue::Bool),
        15 => arg.string().map(|s| s.to_vec()).and_then(|s| e.get_string_variable(&s)).map(TValue::String),
        // [lambda, [引数の式...]]
        16 => match arg {
            TValue::StrictArray(a) if a.len() == 2 => a[1].strict_array().and_then(|arr| e.apply(&a[0], arr)),
            _ => return None,
        },
        _ => return None,
    };
    Some(e.finish(r))
}

/// テンプレートの処理を 1 つ呼ぶ (C++ の `Template` の各メソッド)。`arg` と `*result` は
/// src/template/value.rs の形式の値。返り値は 0 成功、1 コールバックの中断、
/// 2〜6 は例外 (`*result` にメッセージ): 2 `GeneralException`、3 `StreamException`、
/// 4 `std::runtime_error`、5 `std::out_of_range`、6 `std::invalid_argument`。
/// 7 は呼び出し方の誤り (知らない `op`、壊れた `arg`、ホストが必要なのに NULL)。
///
/// # Safety
/// `arg` は `arg_len` バイト読めること。`host` は NULL か、有効な `CTemplateHost` (その `reader`
/// は NULL か有効な `CReader`) を指すこと。`result` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_template_call(op: i32, host: *const CTemplateHost, arg: *const u8, arg_len: usize, result: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let arg = match tvalue::decode(unsafe { input(arg, arg_len) }) {
        Some(v) => v,
        None => return 7,
    };
    let r = match template_pure(op, &arg) {
        Some(r) => r,
        None => {
            if host.is_null() {
                return 7;
            }
            // SAFETY: 関数の Safety 節
            let h = unsafe { &*host };
            // SAFETY: 同上
            let r = if h.reader.is_null() { None } else { Some(unsafe { reader(h.reader) }) };
            let mut hr = CTemplateHostRef { h, r };
            match template_with_host(op, &arg, &mut hr) {
                Some(r) => r,
                None => return 7,
            }
        }
    };
    let (code, bytes) = match r {
        Ok(v) => (0, tvalue::encoded(&v)),
        Err(template::Error::Abort) => (1, Vec::new()),
        Err(template::Error::General(m)) => (2, m),
        Err(template::Error::Stream(m)) => (3, m),
        Err(template::Error::Runtime(m)) => (4, m),
        Err(template::Error::OutOfRange(m)) => (5, m),
        Err(template::Error::InvalidArgument(m)) => (6, m),
    };
    // SAFETY: 関数の Safety 節
    unsafe { *result = into_buf(bytes) };
    code
}

// ---------------------------------------------------------------- 公開ディレクトリとコンソール (段階5b)

use crate::{commands, public};

/// 連結したバイト列と各要素の長さ (`pcrs_str_join` と同じ形) を要素に戻す。長さが合わなければ `None`。
///
/// # Safety
/// `joined` は `joined_len` バイト、`lens` は `count` 個の `usize` が読めること。
unsafe fn input_parts(joined: *const u8, joined_len: usize, lens: *const usize, count: usize) -> Option<Vec<Vec<u8>>> {
    // SAFETY: 関数の Safety 節
    let (joined, lens) = unsafe { (input(joined, joined_len), input_usize(lens, count)) };
    let mut parts = Vec::with_capacity(lens.len());
    let mut p = 0usize;
    for &len in lens {
        parts.push(joined.get(p..p.checked_add(len)?)?.to_vec());
        p += len;
    }
    Some(parts)
}

/// `PublicController::formatUptime`
#[no_mangle]
pub extern "C" fn pcrs_public_format_uptime(total_seconds: u32) -> PcrsBuf {
    into_buf(public::format_uptime(total_seconds).into_bytes())
}

/// `PublicController::acceptableLanguages`
///
/// # Safety
/// `s` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_public_acceptable_languages(s: *const u8, n: usize) -> PcrsVec {
    // SAFETY: 関数の Safety 節
    into_vec(public::acceptable_languages(unsafe { input(s, n) }))
}

/// commands.cpp の `parse_options`。成功なら 0 で、`*out` に (名前, 値) の組を `*num_options` 組
/// 並べたあとに位置引数を並べる。知らないオプションなら -1 で、`*err` にメッセージ
/// (`FormatException`)。引数の形が壊れていれば -2。
///
/// # Safety
/// 各入力は `pcrs_str_join` の引数と同じく読めること。`out`、`num_options`、`err` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_commands_parse_options(
    args_joined: *const u8, args_joined_len: usize, args_lens: *const usize, args_count: usize,
    names_joined: *const u8, names_joined_len: usize, names_lens: *const usize, names_count: usize,
    out: *mut PcrsVec, num_options: *mut usize, err: *mut PcrsBuf,
) -> i32 {
    // SAFETY: 関数の Safety 節
    let parts = unsafe {
        (
            input_parts(args_joined, args_joined_len, args_lens, args_count),
            input_parts(names_joined, names_joined_len, names_lens, names_count),
        )
    };
    let (args, names) = match parts {
        (Some(a), Some(n)) => (a, n),
        _ => return -2,
    };
    match commands::parse_options(&args, &names) {
        Ok((options, positionals)) => {
            let n = options.len();
            let mut flat: Vec<Vec<u8>> = Vec::with_capacity(n * 2 + positionals.len());
            for (k, v) in options {
                flat.push(k);
                flat.push(v);
            }
            flat.extend(positionals);
            // SAFETY: 関数の Safety 節
            unsafe {
                out.write(into_vec(flat));
                *num_options = n;
            }
            0
        }
        Err(msg) => {
            // SAFETY: 関数の Safety 節
            unsafe { *err = into_buf(msg) };
            -1
        }
    }
}

// ---------------------------------------------------------------- PCP (src/pcp)

use crate::pcp::{self, InfoField, Ip, Level as PcpLevel, Target};

/// アドレスの atom の値 (C の `pcrs_pcp_ip`)。`kind` は 0 (なし)、4 (`v4`)、16 (`v6`)。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CPcpIp {
    pub kind: u8,
    pub v4: u32,
    pub v6: [u8; 16],
}

fn c_ip(ip: Option<Ip>) -> CPcpIp {
    match ip {
        None => CPcpIp { kind: 0, v4: 0, v6: [0; 16] },
        Some(Ip::V4(v)) => CPcpIp { kind: 4, v4: v, v6: [0; 16] },
        Some(Ip::V6(a)) => CPcpIp { kind: 16, v4: 0, v6: a },
    }
}

/// `readHostAtoms` が読んだ `ChanHit` の値 (C の `pcrs_pcp_hit`)。`set` のビットが立っている
/// メンバーだけ `ChanHit` に入れる (ほかは `ChanHit::init()` の値のまま)。
#[repr(C)]
pub struct CPcpHit {
    pub set: u32,
    pub rhost_ip: [CPcpIp; 2],
    pub rhost_port_set: [bool; 2],
    pub rhost_port: [i32; 2],
    pub num_listeners: i32,
    pub num_relays: i32,
    pub up_time: i32,
    pub oldest_pos: i32,
    pub newest_pos: i32,
    pub version: i32,
    pub version_vp: i32,
    pub version_ex_number: i32,
    pub flags1: i32,
    pub uphost_port: i32,
    pub uphost_hops: i32,
    pub version_ex_prefix: [u8; 2],
    pub session_id: [u8; 16],
    pub uphost_ip: CPcpIp,
    pub chan_id: [u8; 16],
    pub num_hops: i32,
}

fn c_hit(h: &pcp::Hit) -> CPcpHit {
    let mut set = 0u32;
    let mut take = |bit: u32, v: Option<i32>| -> i32 {
        match v {
            Some(x) => {
                set |= 1 << bit;
                x
            }
            None => 0,
        }
    };
    let num_listeners = take(0, h.num_listeners);
    let num_relays = take(1, h.num_relays);
    let up_time = take(2, h.up_time);
    let oldest_pos = take(3, h.oldest_pos);
    let newest_pos = take(4, h.newest_pos);
    let version = take(5, h.version);
    let version_vp = take(6, h.version_vp);
    let version_ex_number = take(8, h.version_ex_number);
    let flags1 = take(9, h.flags1);
    let uphost_port = take(11, h.uphost_port);
    let uphost_hops = take(12, h.uphost_hops);
    if h.version_ex_prefix.is_some() {
        set |= 1 << 7;
    }
    if h.session_id.is_some() {
        set |= 1 << 10;
    }
    CPcpHit {
        set,
        rhost_ip: [c_ip(h.rhost_ip[0]), c_ip(h.rhost_ip[1])],
        rhost_port_set: [h.rhost_port[0].is_some(), h.rhost_port[1].is_some()],
        rhost_port: [h.rhost_port[0].unwrap_or(0), h.rhost_port[1].unwrap_or(0)],
        num_listeners,
        num_relays,
        up_time,
        oldest_pos,
        newest_pos,
        version,
        version_vp,
        version_ex_number,
        flags1,
        uphost_port,
        uphost_hops,
        version_ex_prefix: h.version_ex_prefix.unwrap_or([0; 2]),
        session_id: h.session_id.unwrap_or([0; 16]),
        uphost_ip: c_ip(h.uphost_ip),
        chan_id: h.chan_id,
        num_hops: h.num_hops,
    }
}

/// PCP の処理の状態 (C の `pcrs_pcp_state`。`BroadcastState` と `PCPStream::nextRootPacket`)
#[repr(C)]
pub struct CPcpState {
    pub chan_id: [u8; 16],
    pub bc_id: [u8; 16],
    pub num_hops: i32,
    pub for_me: bool,
    pub stream_pos: u32,
    pub group: i32,
    pub next_root_packet: u32,
}

const PCP_EV_ROUTE_ADD: i32 = 1;
const PCP_EV_UPDATE_INTERVAL: i32 = 2;
const PCP_EV_UPGRADE: i32 = 3;
const PCP_EV_TRACKER_UPDATE: i32 = 4;
const PCP_EV_ROOT_MESSAGE: i32 = 5;
const PCP_EV_CHAN_BEGIN: i32 = 6;
const PCP_EV_CHAN_INFO_STRING: i32 = 7;
const PCP_EV_CHAN_INFO_BITRATE: i32 = 8;
const PCP_EV_CHAN_BCID: i32 = 9;
const PCP_EV_CHAN_ID: i32 = 10;
const PCP_EV_CHAN_END: i32 = 11;

/// PCP の処理から見た C++ 側 (C の `pcrs_pcp_host`)。int を返すものは、成功で 0、C++ の例外で
/// 中断したら -1。
#[repr(C)]
pub struct CPcpHost {
    pub ctx: *mut c_void,
    pub session_id: unsafe extern "C" fn(ctx: *mut c_void, out: *mut u8),
    pub is_root: unsafe extern "C" fn(ctx: *mut c_void) -> bool,
    pub time: unsafe extern "C" fn(ctx: *mut c_void) -> u32,
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: i32, msg: *const u8, len: usize),
    pub event: unsafe extern "C" fn(ctx: *mut c_void, op: i32, arg: i32, data: *const u8, len: usize) -> i32,
    pub hit: unsafe extern "C" fn(ctx: *mut c_void, hit: *const CPcpHit, add: bool) -> i32,
    pub push: unsafe extern "C" fn(ctx: *mut c_void, ip: *const CPcpIp, port_set: bool, port: i32, chan_id: *const u8) -> i32,
    pub chan_has_channel: unsafe extern "C" fn(ctx: *mut c_void) -> bool,
    pub chan_packet: unsafe extern "C" fn(ctx: *mut c_void, kind: i32, pos: u32, cont: bool, data: *const u8, len: usize) -> i32,
    pub broadcast: unsafe extern "C" fn(ctx: *mut c_void, target: i32, pack: *const u8, len: usize, chan_id: *const u8, dest_id: *const u8) -> i32,
}

struct CPcpHostRef<'a>(&'a CPcpHost);

impl CPcpHostRef<'_> {
    fn event(&mut self, op: i32, arg: i32, data: &[u8]) -> Result<(), Abort> {
        // SAFETY: 下の impl の説明のとおり
        status(unsafe { (self.0.event)(self.0.ctx, op, arg, data.as_ptr(), data.len()) })
    }
}

// 以下の unsafe ブロックはどれも、CPcpHost を渡した C++ 側が関数ポインタと ctx の有効性を
// 保証していることに頼る。渡すポインタは呼び出しの間だけ有効。
impl pcp::Host for CPcpHostRef<'_> {
    fn session_id(&mut self) -> [u8; 16] {
        let mut out = [0u8; 16];
        // SAFETY: 上記。out は 16 バイト書ける
        unsafe { (self.0.session_id)(self.0.ctx, out.as_mut_ptr()) };
        out
    }
    fn is_root(&mut self) -> bool {
        // SAFETY: 上記
        unsafe { (self.0.is_root)(self.0.ctx) }
    }
    fn time(&mut self) -> u32 {
        // SAFETY: 上記
        unsafe { (self.0.time)(self.0.ctx) }
    }
    fn log(&mut self, level: PcpLevel, msg: &[u8]) {
        // SAFETY: 上記
        unsafe { (self.0.log)(self.0.ctx, level as i32, msg.as_ptr(), msg.len()) }
    }
    fn route_add(&mut self, id: &[u8; 16]) -> Result<(), Abort> {
        self.event(PCP_EV_ROUTE_ADD, 0, id)
    }
    fn set_update_interval(&mut self, si: i32) -> Result<(), Abort> {
        self.event(PCP_EV_UPDATE_INTERVAL, si, &[])
    }
    fn upgrade(&mut self, url: &[u8]) -> Result<(), Abort> {
        self.event(PCP_EV_UPGRADE, 0, url)
    }
    fn tracker_update(&mut self) -> Result<(), Abort> {
        self.event(PCP_EV_TRACKER_UPDATE, 0, &[])
    }
    fn root_message(&mut self, msg: &[u8]) -> Result<(), Abort> {
        self.event(PCP_EV_ROOT_MESSAGE, 0, msg)
    }
    fn hit(&mut self, hit: &pcp::Hit, add: bool) -> Result<(), Abort> {
        let h = c_hit(hit);
        // SAFETY: 上記
        status(unsafe { (self.0.hit)(self.0.ctx, &h, add) })
    }
    fn push(&mut self, ip: Option<Ip>, port: Option<i32>, chan_id: &[u8; 16]) -> Result<(), Abort> {
        let ip = c_ip(ip);
        // SAFETY: 上記
        status(unsafe { (self.0.push)(self.0.ctx, &ip, port.is_some(), port.unwrap_or(0), chan_id.as_ptr()) })
    }
    fn chan_begin(&mut self, chan_id: &[u8; 16]) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_BEGIN, 0, chan_id)
    }
    fn chan_has_channel(&mut self) -> bool {
        // SAFETY: 上記
        unsafe { (self.0.chan_has_channel)(self.0.ctx) }
    }
    fn chan_packet(&mut self, pkt: &pcp::Packet) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe { (self.0.chan_packet)(self.0.ctx, pkt.kind, pkt.pos, pkt.cont, pkt.data.as_ptr(), pkt.data.len()) })
    }
    fn chan_info_string(&mut self, field: InfoField, bytes: &[u8]) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_INFO_STRING, field as i32, bytes)
    }
    fn chan_info_bitrate(&mut self, bitrate: i32) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_INFO_BITRATE, bitrate, &[])
    }
    fn chan_bcid(&mut self, id: &[u8; 16]) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_BCID, 0, id)
    }
    fn chan_id(&mut self, id: &[u8; 16]) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_ID, 0, id)
    }
    fn chan_end(&mut self) -> Result<(), Abort> {
        self.event(PCP_EV_CHAN_END, 0, &[])
    }
    fn broadcast(&mut self, target: Target, pack: &[u8], chan_id: &[u8; 16], dest_id: &[u8; 16]) -> Result<(), Abort> {
        // SAFETY: 上記
        status(unsafe {
            (self.0.broadcast)(self.0.ctx, target as i32, pack.as_ptr(), pack.len(), chan_id.as_ptr(), dest_id.as_ptr())
        })
    }
}

/// `PCPStream::readPacket` の、受け取ったパケットを処理する部分。`buf` はパケットのバッファ全体
/// (`ChanPacket::data`、`len` バイト)。返り値は 0 成功 (`*result` に `procAtom` の値)、
/// 1 コールバックの中断、2 `StreamException` (メッセージを `*err` に書く)。
///
/// # Safety
/// `host` は有効な `CPcpHost`、`buf` は `len` バイト読み書きでき、`st`、`result`、`err` は
/// 読み書きできること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_pcp_proc_packet(
    host: *const CPcpHost,
    buf: *mut u8,
    len: usize,
    st: *mut CPcpState,
    result: *mut i32,
    err: *mut PcrsBuf,
) -> i32 {
    // SAFETY: 関数の Safety 節
    let (h, st) = unsafe { (&*host, &mut *st) };
    let buf: &mut [u8] = if len == 0 || buf.is_null() {
        &mut []
    } else {
        // SAFETY: 関数の Safety 節
        unsafe { std::slice::from_raw_parts_mut(buf, len) }
    };
    let mut state = pcp::State {
        bcs: pcp::BroadcastState {
            chan_id: st.chan_id,
            bc_id: st.bc_id,
            num_hops: st.num_hops,
            for_me: st.for_me,
            stream_pos: st.stream_pos,
            group: st.group,
        },
        next_root_packet: st.next_root_packet,
    };
    let r = pcp::proc_packet(&mut CPcpHostRef(h), buf, &mut state);
    // 途中で中断しても、それまでに変えた状態は C++ 版と同じく残す
    st.chan_id = state.bcs.chan_id;
    st.bc_id = state.bcs.bc_id;
    st.num_hops = state.bcs.num_hops;
    st.for_me = state.bcs.for_me;
    st.stream_pos = state.bcs.stream_pos;
    st.group = state.bcs.group;
    st.next_root_packet = state.next_root_packet;
    match r {
        Ok(v) => {
            // SAFETY: 関数の Safety 節
            unsafe { *result = v };
            0
        }
        Err(pcp::Error::Abort) => 1,
        Err(pcp::Error::Stream(msg)) => {
            // SAFETY: 関数の Safety 節
            unsafe { *err = into_buf(msg.as_bytes().to_vec()) };
            2
        }
    }
}

// ---------------------------------------------------------------- PCP のハンドシェイク (src/pcp/handshake.rs)

use crate::pcp::handshake::{self, Hello, Kind as HelloKind, StreamIo};

/// 読んだ helo / oleh (C の `pcrs_pcp_hello`)。`set` のビットが立っている値だけ入っている。
#[repr(C)]
pub struct CPcpHello {
    /// 最初の atom が期待した ID (helo か oleh) だった
    pub header_ok: bool,
    /// 最初の atom の ID が違った (`unexpected` にその ID)
    pub is_unexpected: bool,
    pub unexpected: [u8; 4],
    pub has_agent: bool,
    pub agent: [u8; 64],
    pub agent_len: usize,
    pub set: u32,
    pub version: i32,
    pub disable: i32,
    pub os_type: i32,
    pub port: i32,
    pub ping: i32,
    pub session_id: [u8; 16],
    pub bcid: [u8; 16],
    pub remote_ip: CPcpIp,
}

fn c_hello(h: &Hello, out: &mut CPcpHello) {
    out.header_ok = h.header_ok;
    if let Some(id) = h.unexpected {
        out.is_unexpected = true;
        out.unexpected = id;
    }
    if let Some(a) = &h.agent {
        out.has_agent = true;
        out.agent[..a.len()].copy_from_slice(a);
        out.agent_len = a.len();
    }
    let mut set = 0u32;
    let mut take = |bit: u32, v: Option<i32>, dst: &mut i32| {
        if let Some(x) = v {
            set |= bit;
            *dst = x;
        }
    };
    take(1, h.version, &mut out.version);
    take(2, h.disable, &mut out.disable);
    take(16, h.os_type, &mut out.os_type);
    take(32, h.port, &mut out.port);
    take(64, h.ping, &mut out.ping);
    if let Some(s) = h.session_id {
        set |= 4;
        out.session_id = s;
    }
    if let Some(b) = h.bcid {
        set |= 8;
        out.bcid = b;
    }
    out.remote_ip = c_ip(h.remote_ip);
    out.set = set;
}

/// ハンドシェイクで受け取る helo / oleh を `r` から読む (`kind` は 0 helo、1 oleh、2 ping の oleh)。
/// 返り値は 0 成功、1 読み出しの中断 (C++ の例外)、2 `StreamException` (メッセージを `*err` に書く)。
/// どの場合も、それまでに読んだ値を `*out` に書く。`log` は読み飛ばした atom のログ。
///
/// # Safety
/// `r` は有効な `CReader`、`my_sid` は 16 バイト読め、`out` と `err` は書き込めること。
/// `log` は `log_ctx` とともに呼べること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_pcp_read_hello(
    r: *const CReader,
    kind: i32,
    my_sid: *const u8,
    log_ctx: *mut c_void,
    log: unsafe extern "C" fn(ctx: *mut c_void, msg: *const u8, len: usize),
    out: *mut CPcpHello,
    err: *mut PcrsBuf,
) -> i32 {
    let kind = match kind {
        0 => HelloKind::Helo,
        1 => HelloKind::Oleh,
        _ => HelloKind::Ping,
    };
    // SAFETY: 関数の Safety 節
    let (sid, out) = unsafe {
        let mut sid = [0u8; 16];
        sid.copy_from_slice(input(my_sid, 16));
        (sid, &mut *out)
    };
    // SAFETY: 同上
    let mut atom = pcp::AtomStream::new(StreamIo { r: unsafe { reader(r) } });
    let mut h = Hello { ping_sid_init: out.session_id, ..Default::default() };
    let mut logf = |m: &[u8]| {
        // SAFETY: 関数の Safety 節
        unsafe { log(log_ctx, m.as_ptr(), m.len()) }
    };
    let res = handshake::read_hello(&mut atom, kind, &sid, &mut logf, &mut h);
    c_hello(&h, out);
    match res {
        Ok(()) => 0,
        Err(pcp::Error::Abort) => 1,
        Err(pcp::Error::Stream(msg)) => {
            // SAFETY: 関数の Safety 節
            unsafe { *err = into_buf(msg.as_bytes().to_vec()) };
            2
        }
    }
}

/// `PCPStream::readVersion`。返り値は `pcrs_pcp_read_hello` と同じで、成功なら版を `*ver` に書く。
///
/// # Safety
/// `r` は有効な `CReader`、`ver` と `err` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_pcp_read_version(r: *const CReader, ver: *mut i32, err: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let mut io = StreamIo { r: unsafe { reader(r) } };
    match handshake::read_version(&mut io) {
        Ok(v) => {
            // SAFETY: 関数の Safety 節
            unsafe { *ver = v };
            0
        }
        Err(pcp::Error::Abort) => 1,
        Err(pcp::Error::Stream(msg)) => {
            // SAFETY: 関数の Safety 節
            unsafe { *err = into_buf(msg.as_bytes().to_vec()) };
            2
        }
    }
}

// ---------------------------------------------------------------- ChanPacketBuffer (src/chanpacket.rs)

use crate::chanpacket::{Buffer, Packet as ChanPacket, Positions, MAX_PACKETS};

/// C++ の `ChanPacketBuffer` のメンバーを指すもの (C の `pcrs_cpb`)。ロックは C++ 側で取る。
#[repr(C)]
pub struct CCpb {
    pub packets: *mut ChanPacket,
    pub last_pos: *mut u32,
    pub first_pos: *mut u32,
    pub safe_pos: *mut u32,
    pub read_pos: *mut u32,
    pub write_pos: *mut u32,
    pub accept: *mut u32,
    pub last_write_time: *mut u32,
}

/// # Safety
/// `b` は有効な `CCpb` を指し、そのポインタはどれも有効で、`packets` は 64 個の配列であること。
/// 呼ぶ側がロックを取っていて、ほかの誰もその間に書き換えないこと。
unsafe fn with_cpb<R>(b: *const CCpb, f: impl FnOnce(&mut Buffer) -> R) -> R {
    // SAFETY: 関数の Safety 節
    unsafe {
        let b = &*b;
        let mut p = Positions {
            last_pos: *b.last_pos,
            first_pos: *b.first_pos,
            safe_pos: *b.safe_pos,
            read_pos: *b.read_pos,
            write_pos: *b.write_pos,
            accept: *b.accept,
            last_write_time: *b.last_write_time,
        };
        let packets = std::slice::from_raw_parts_mut(b.packets, MAX_PACKETS as usize);
        let r = f(&mut Buffer { packets, p: &mut p });
        *b.last_pos = p.last_pos;
        *b.first_pos = p.first_pos;
        *b.safe_pos = p.safe_pos;
        *b.read_pos = p.read_pos;
        *b.write_pos = p.write_pos;
        *b.accept = p.accept;
        *b.last_write_time = p.last_write_time;
        r
    }
}

/// `ChanPacketBuffer::init` の位置の初期化
///
/// # Safety
/// `with_cpb` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_init(b: *const CCpb) {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(b, |b| b.init()) }
}

/// `writePacket`。`pack` はバッファの外のパケット (`sync` を書き換える)。
///
/// # Safety
/// `with_cpb` と同じ。`pack` は有効で、バッファの中のパケットではないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_write_packet(b: *const CCpb, pack: *mut ChanPacket, update_read_pos: bool, now: u32) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(b, |b| b.write_packet(&mut *pack, update_read_pos, now)) }
}

/// `willSkip`
///
/// # Safety
/// `with_cpb` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_will_skip(b: *const CCpb) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(b, |b| b.will_skip()) }
}

/// `readPacket` の状態: 0 読める、1 遅れすぎ (`Read too far behind`)、2 まだない (待つ)
///
/// # Safety
/// `with_cpb` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_read_state(b: *const CCpb, check_behind: bool) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        with_cpb(b, |b| {
            if check_behind && b.too_far_behind() {
                1
            } else if b.is_empty() {
                2
            } else {
                0
            }
        })
    }
}

/// `readPacket` の、次のパケットを `pack` に写して進めるところ (`pcrs_cpb_read_state` が 0 のとき)
///
/// # Safety
/// `pcrs_cpb_write_packet` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_take(b: *const CCpb, pack: *mut ChanPacket) {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(b, |b| b.take(&mut *pack)) }
}

/// `findPacket`
///
/// # Safety
/// `pcrs_cpb_write_packet` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_find_packet(b: *const CCpb, spos: u32, pack: *mut ChanPacket) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(b, |b| b.find_packet(spos, &mut *pack)) }
}

/// 位置を返すもの (`op` は C の `PCRS_CPB_*`)
///
/// # Safety
/// `with_cpb` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_pos(b: *const CCpb, op: i32, arg: u32) -> u32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        with_cpb(b, |b| match op {
            0 => b.latest_pos(),
            1 => b.oldest_pos(),
            2 => b.find_oldest_pos(arg),
            3 => b.stream_pos(arg),
            4 => b.stream_pos_end(arg),
            5 => b.latest_non_continuation_pos(),
            _ => b.oldest_non_continuation_pos(),
        })
    }
}

/// `getStatistics`。長さを `lens` (64 個書ける) に書き、その数を返す。
///
/// # Safety
/// `with_cpb` と同じ。`lens` は 64 個、`cs` と `ncs` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_statistics(b: *const CCpb, lens: *mut u32, cs: *mut i32, ncs: *mut i32) -> usize {
    // SAFETY: 関数の Safety 節
    unsafe {
        let s = with_cpb(b, |b| b.statistics());
        let n = s.packet_lengths.len().min(MAX_PACKETS as usize);
        std::ptr::copy_nonoverlapping(s.packet_lengths.as_ptr(), lens, n);
        *cs = s.continuations;
        *ncs = s.non_continuations;
        n
    }
}

/// `copyFrom` (使われていない)
///
/// # Safety
/// `b` と `src` はどちらも `with_cpb` と同じ条件で、別のバッファを指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_cpb_copy_from(b: *const CCpb, src: *const CCpb, req_pos: u32) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe { with_cpb(src, |s| with_cpb(b, |b| b.copy_from(s, req_pos))) }
}

// ---- chandir (core/common/chandir.cpp の ChannelEntry と ChannelDirectory の一部) ----

use crate::chandir;

/// バイト列を借りたもの (C の `pcrs_bytes`)
#[repr(C)]
pub struct CBytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl CBytes {
    fn of(v: &[u8]) -> CBytes {
        CBytes { ptr: v.as_ptr(), len: v.len() }
    }
}

/// `ChannelEntry` (C の `pcrs_chan_entry`)。中身は呼び出しの間だけ有効。
#[repr(C)]
pub struct CChanEntry {
    pub name: CBytes,
    pub tip: CBytes,
    pub url: CBytes,
    pub genre: CBytes,
    pub desc: CBytes,
    pub content_type: CBytes,
    pub track_artist: CBytes,
    pub track_album: CBytes,
    pub track_name: CBytes,
    pub track_contact: CBytes,
    pub encoded_name: CBytes,
    pub uptime: CBytes,
    pub status: CBytes,
    pub comment: CBytes,
    pub id: [u8; 16],
    pub num_directs: i32,
    pub num_relays: i32,
    pub bitrate: i32,
    pub direct: i32,
}

pub type ChanEntryFn = unsafe extern "C" fn(ctx: *mut c_void, e: *const CChanEntry);
pub type ChanErrorFn = unsafe extern "C" fn(ctx: *mut c_void, lineno: i32);

fn emit_entry(ctx: *mut c_void, on_entry: ChanEntryFn, e: &chandir::Entry) {
    let c = CChanEntry {
        name: CBytes::of(&e.name),
        tip: CBytes::of(&e.tip),
        url: CBytes::of(&e.url),
        genre: CBytes::of(&e.genre),
        desc: CBytes::of(&e.desc),
        content_type: CBytes::of(&e.content_type),
        track_artist: CBytes::of(&e.track_artist),
        track_album: CBytes::of(&e.track_album),
        track_name: CBytes::of(&e.track_name),
        track_contact: CBytes::of(&e.track_contact),
        encoded_name: CBytes::of(&e.encoded_name),
        uptime: CBytes::of(&e.uptime),
        status: CBytes::of(&e.status),
        comment: CBytes::of(&e.comment),
        id: e.id,
        num_directs: e.num_directs,
        num_relays: e.num_relays,
        bitrate: e.bitrate,
        direct: e.direct,
    };
    // SAFETY: 呼び出し側が渡したコールバック。c と e はこの呼び出しの間有効
    unsafe { on_entry(ctx, &c) }
}

/// `ChannelEntry::textToChannelEntries`。正しい行は `on_entry`、欄の数が違う行は `on_error` に
/// 行番号を渡す。
///
/// # Safety
/// `text` は `n` バイト読めること。コールバックは例外を投げないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chandir_parse(
    text: *const u8,
    n: usize,
    ctx: *mut c_void,
    on_entry: ChanEntryFn,
    on_error: ChanErrorFn,
) {
    // SAFETY: 関数の Safety 節
    let text = unsafe { input(text, n) };
    chandir::parse_index(text, |l| match l {
        chandir::Line::Entry(e) => emit_entry(ctx, on_entry, &e),
        // SAFETY: 関数の Safety 節
        chandir::Line::Error(lineno) => unsafe { on_error(ctx, lineno) },
    });
}

/// `ChannelEntry(fields, feedUrl)`。欄が 19 個に足りなければ -1 (`on_entry` は呼ばない)。
///
/// # Safety
/// `fields` は `count` 個読めて、それぞれ `pcrs_bytes` の約束を守ること。コールバックは例外を
/// 投げないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chandir_entry(
    fields: *const CBytes,
    count: usize,
    ctx: *mut c_void,
    on_entry: ChanEntryFn,
) -> i32 {
    let fields: &[CBytes] = if count == 0 || fields.is_null() {
        &[]
    } else {
        // SAFETY: 関数の Safety 節
        unsafe { std::slice::from_raw_parts(fields, count) }
    };
    // SAFETY: 関数の Safety 節
    let v: Vec<&[u8]> = fields.iter().map(|f| unsafe { input(f.ptr, f.len) }).collect();
    match chandir::Entry::from_fields(&v) {
        Some(e) => {
            emit_entry(ctx, on_entry, &e);
            0
        }
        None => -1,
    }
}

/// `chatUrl` (`kind` が 0) と `statsUrl` (1)
///
/// # Safety
/// `feed` は `fn_` バイト、`name` は `nn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chandir_side_url(
    feed: *const u8,
    fn_: usize,
    name: *const u8,
    nn: usize,
    kind: i32,
) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    let (feed, name) = unsafe { (input(feed, fn_), input(name, nn)) };
    into_buf(if kind == 0 { chandir::chat_url(feed, name) } else { chandir::stats_url(feed, name) })
}

bytes_to_buf!(
    /// `directoryUrlOf`
    pcrs_chandir_directory_url,
    chandir::directory_url
);

/// `formatTime`
#[no_mangle]
pub extern "C" fn pcrs_chandir_format_time(diff: u32) -> PcrsBuf {
    into_buf(chandir::format_time(diff))
}

// ---- chaninfo (core/common/chaninfo.cpp の ChanInfo と TrackInfo の一部) ----

use crate::chaninfo;
use crate::pcp::write::AtomBuf;

/// 表の関数が返す文字列の、NUL で終わる形 (C++ 版の `const char*` の返り値と同じく静的な文字列)
static C_STRS: &[&[u8]] = &[
    b"UNKNOWN\0", b"RAW\0", b"MP3\0", b"OGG\0", b"OGM\0", b"MOV\0", b"MPG\0", b"FLV\0", b"MKV\0", b"WEBM\0",
    b"MP4\0", b"PLS\0", b".ogg\0", b".mp3\0", b".mov\0", b".flv\0", b".mkv\0", b".webm\0", b".mp4\0", b"\0",
    b"audio/mpeg\0", b"application/x-ogg\0", b"video/quicktime\0", b"video/mpeg\0", b"video/x-flv\0",
    b"video/x-matroska\0", b"video/webm\0", b"video/mp4\0", b"application/octet-stream\0", b"HTTP\0",
    b"FILE\0", b"PCP\0", b"RTMP\0", b"PIPE\0", b".ram\0", b".m3u\0",
    // uptest
    b"Untried\0", b"Success\0", b"Error\0", b"invalid URL\0", b"unsupported protocol\0", b"URL already exists\0",
    // servhs
    b"text/html\0", b"text/css\0", b"image/jpeg\0", b"image/gif\0", b"image/png\0",
    b"application/javascript; charset=utf-8\0", b"image/vnd.microsoft.icon\0",
    // assets
    b"image/svg+xml\0",
    b"HTTP/1.0 411 Length required\0", b"HTTP/1.0 400 Bad Request\0", b"HTTP/1.0 413 Request Entity Too Large\0",
];

fn c_static(s: &'static [u8]) -> *const std::ffi::c_char {
    C_STRS
        .iter()
        .find(|c| &c[..c.len() - 1] == s)
        .map(|c| c.as_ptr() as *const std::ffi::c_char)
        .expect("C_STRS に載っていない文字列")
}

macro_rules! bytes_to_cstatic {
    ($name:ident, $f:path) => {
        /// # Safety
        /// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
        #[no_mangle]
        pub unsafe extern "C" fn $name(s: *const u8, n: usize) -> *const std::ffi::c_char {
            // SAFETY: 関数の Safety 節
            c_static($f(unsafe { input(s, n) }))
        }
    };
}

bytes_to_cstatic!(pcrs_chaninfo_type_ext, chaninfo::type_ext);
bytes_to_cstatic!(pcrs_chaninfo_mime_type, chaninfo::mime_type);
bytes_to_cstatic!(pcrs_chaninfo_type_from_mime, chaninfo::type_from_mime);
bytes_to_cstatic!(pcrs_chaninfo_type_from_str, chaninfo::type_from_str);
bytes_to_cstatic!(pcrs_chaninfo_playlist_ext, chaninfo::playlist_ext);

#[no_mangle]
pub extern "C" fn pcrs_chaninfo_protocol_str(p: i32) -> *const std::ffi::c_char {
    c_static(chaninfo::protocol_str(p))
}

/// # Safety
/// `s` は `n` バイト読めること (`n` が 0 なら NULL でもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chaninfo_protocol_from_str(s: *const u8, n: usize) -> i32 {
    // SAFETY: 関数の Safety 節
    chaninfo::protocol_from_str(unsafe { input(s, n) })
}

/// `ChanInfo` (C の `pcrs_chan_info`)
#[repr(C)]
pub struct CChanInfo {
    pub name: CBytes,
    pub content_type: CBytes,
    pub mime: CBytes,
    pub ext: CBytes,
    pub desc: CBytes,
    pub genre: CBytes,
    pub url: CBytes,
    pub comment: CBytes,
    pub track_contact: CBytes,
    pub track_title: CBytes,
    pub track_artist: CBytes,
    pub track_album: CBytes,
    pub track_genre: CBytes,
    pub id: [u8; 16],
    pub bcid: [u8; 16],
    pub bitrate: i32,
    pub status: i32,
}

/// # Safety
/// `c` は有効な `pcrs_chan_info` を指し、その中のバイト列は読めること。
unsafe fn info_of<'a>(c: *const CChanInfo) -> chaninfo::Info<'a> {
    // SAFETY: 関数の Safety 節
    unsafe {
        let c = &*c;
        let b = |x: &CBytes| input(x.ptr, x.len);
        chaninfo::Info {
            name: b(&c.name),
            id: c.id,
            bcid: c.bcid,
            bitrate: c.bitrate,
            content_type: b(&c.content_type),
            mime: b(&c.mime),
            ext: b(&c.ext),
            status: c.status,
            desc: b(&c.desc),
            genre: b(&c.genre),
            url: b(&c.url),
            comment: b(&c.comment),
            track: chaninfo::Track {
                contact: b(&c.track_contact),
                title: b(&c.track_title),
                artist: b(&c.track_artist),
                album: b(&c.track_album),
                genre: b(&c.track_genre),
            },
        }
    }
}

/// `getTypeStringLong`
///
/// # Safety
/// `info` は `info_of` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chaninfo_type_string_long(info: *const CChanInfo) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    let i = unsafe { info_of(info) };
    into_buf(chaninfo::type_string_long(i.content_type, i.mime, i.ext))
}

/// `match(ChanInfo&)` (`name_id_only` なら `matchNameID`)
///
/// # Safety
/// `me` と `q` は `info_of` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chaninfo_match(me: *const CChanInfo, q: *const CChanInfo, name_id_only: bool) -> bool {
    // SAFETY: 関数の Safety 節
    let (me, q) = unsafe { (info_of(me), info_of(q)) };
    if name_id_only {
        chaninfo::match_name_id(&me, &q)
    } else {
        chaninfo::match_info(&me, &q)
    }
}

/// `ChanInfo::update`。0 使わない、1 配信者の鍵が違う、2 `copy` の欄を写す、3 さらに配信者の鍵も写す。
///
/// # Safety
/// `me` と `info` は `info_of` と同じ。`copy` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chaninfo_update(me: *const CChanInfo, info: *const CChanInfo, copy: *mut u32) -> i32 {
    // SAFETY: 関数の Safety 節
    let (me, info) = unsafe { (info_of(me), info_of(info)) };
    match chaninfo::update(&me, &info) {
        chaninfo::Update::Ignore => 0,
        chaninfo::Update::BadKey => 1,
        chaninfo::Update::Apply { set_bcid, copy: c } => {
            // SAFETY: 関数の Safety 節
            unsafe { *copy = c };
            if set_bcid {
                3
            } else {
                2
            }
        }
    }
}

/// `TrackInfo::update` で写す欄 (`me` と `info` の track の欄だけ使う)
///
/// # Safety
/// `me` と `info` は `info_of` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_trackinfo_update(me: *const CChanInfo, info: *const CChanInfo) -> u32 {
    // SAFETY: 関数の Safety 節
    let (me, info) = unsafe { (info_of(me), info_of(info)) };
    chaninfo::track_update(&me.track, &info.track)
}

/// `writeInfoAtoms` (`track` が false) と `writeTrackAtoms` (true)
///
/// # Safety
/// `info` は `info_of` と同じ。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chaninfo_write_atoms(info: *const CChanInfo, track: bool) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    let i = unsafe { info_of(info) };
    let mut b = AtomBuf::default();
    if track {
        chaninfo::write_track_atoms(&mut b, &i.track);
    } else {
        chaninfo::write_info_atoms(&mut b, &i);
    }
    into_buf(b.0)
}

// ---- chanhit (core/common/chanhit.cpp の ChanHit と ChanHitList の一部) ----

use crate::chanhit;

/// `Host` (C の `pcrs_host`)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CHost {
    pub ip: [u8; 16],
    pub port: u16,
}

impl From<CHost> for chanhit::Host {
    fn from(h: CHost) -> Self {
        chanhit::Host { ip: h.ip, port: h.port }
    }
}

/// `ChanHit` (C の `pcrs_hit`)
#[repr(C)]
pub struct CHit {
    pub host: CHost,
    pub rhost: [CHost; 2],
    pub uphost: CHost,
    pub num_listeners: u32,
    pub num_relays: u32,
    pub num_hops: u32,
    pub time: u32,
    pub up_time: u32,
    pub last_contact: u32,
    pub version: u32,
    pub oldest_pos: u32,
    pub newest_pos: u32,
    pub uphost_hops: u32,
    pub version_vp: u32,
    pub version_ex_number: u32,
    pub session_id: [u8; 16],
    pub version_ex_prefix: [u8; 2],
    pub firewalled: bool,
    pub tracker: bool,
    pub recv: bool,
    pub dead: bool,
    pub direct: bool,
    pub relay: bool,
    pub cin: bool,
}

impl From<&CHit> for chanhit::Hit {
    fn from(c: &CHit) -> Self {
        chanhit::Hit {
            host: c.host.into(),
            rhost: [c.rhost[0].into(), c.rhost[1].into()],
            num_listeners: c.num_listeners,
            num_relays: c.num_relays,
            num_hops: c.num_hops,
            time: c.time,
            up_time: c.up_time,
            last_contact: c.last_contact,
            session_id: c.session_id,
            version: c.version,
            oldest_pos: c.oldest_pos,
            newest_pos: c.newest_pos,
            firewalled: c.firewalled,
            tracker: c.tracker,
            recv: c.recv,
            dead: c.dead,
            direct: c.direct,
            relay: c.relay,
            cin: c.cin,
            uphost: c.uphost.into(),
            uphost_hops: c.uphost_hops,
            version_vp: c.version_vp,
            version_ex_prefix: c.version_ex_prefix,
            version_ex_number: c.version_ex_number,
        }
    }
}

/// # Safety
/// `h` は有効な `pcrs_hit` を指すこと。
unsafe fn hit_of(h: *const CHit) -> chanhit::Hit {
    // SAFETY: 関数の Safety 節
    unsafe { (&*h).into() }
}

/// # Safety
/// `hits` は `n` 個読めること (`n` が 0 なら NULL でもよい)。
unsafe fn hits_of(hits: *const CHit, n: usize) -> Vec<chanhit::Hit> {
    if n == 0 || hits.is_null() {
        return Vec::new();
    }
    // SAFETY: 関数の Safety 節
    unsafe { std::slice::from_raw_parts(hits, n) }.iter().map(Into::into).collect()
}

/// # Safety
/// `out` は `n` 個書けること (`n` が 0 なら NULL でもよい)。
unsafe fn flags_of<'a>(out: *mut bool, n: usize) -> &'a mut [bool] {
    if n == 0 || out.is_null() {
        &mut []
    } else {
        // SAFETY: 関数の Safety 節
        unsafe { std::slice::from_raw_parts_mut(out, n) }
    }
}

/// `ChanHit::writeAtoms`
///
/// # Safety
/// `h` は有効な `pcrs_hit`、`chan_id` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hit_write_atoms(h: *const CHit, chan_id: *const u8) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    let (h, id) = unsafe { (hit_of(h), *(chan_id as *const [u8; 16])) };
    let mut b = AtomBuf::default();
    chanhit::write_atoms(&mut b, &h, &id);
    into_buf(b.0)
}

/// `versionString`
///
/// # Safety
/// `h` は有効な `pcrs_hit` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hit_version_string(h: *const CHit) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    into_buf(chanhit::version_string(&unsafe { hit_of(h) }))
}

/// `getColor` (0 red、1 purple、2 blue、3 green)
///
/// # Safety
/// `h` は有効な `pcrs_hit` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hit_color(h: *const CHit) -> i32 {
    // SAFETY: 関数の Safety 節
    chanhit::color(&unsafe { hit_of(h) })
}

/// `canGiv`
///
/// # Safety
/// `h` は有効な `pcrs_hit` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hit_can_giv(h: *const CHit) -> bool {
    // SAFETY: 関数の Safety 節
    chanhit::can_giv(&unsafe { hit_of(h) })
}

/// 一覧の数え上げ (`op` は `PCRS_HITS_*`)。返り値は C++ 版の返り値のビット列。
///
/// # Safety
/// `hits` は `n` 個読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hits_count(hits: *const CHit, n: usize, op: i32) -> u32 {
    use chanhit::Count::*;
    let op = match op {
        0 => NumHits,
        1 => NumListeners,
        2 => NumRelays,
        3 => NumTrackers,
        4 => NumFirewalled,
        5 => ClosestHit,
        6 => FurthestHit,
        7 => NewestHit,
        8 => TotalListeners,
        9 => TotalRelays,
        10 => TotalFirewalled,
        _ => return 0,
    };
    // SAFETY: 関数の Safety 節
    chanhit::count(&unsafe { hits_of(hits, n) }, op)
}

/// `ChanHitSearch` (C の `pcrs_hit_search`)
#[repr(C)]
pub struct CHitSearch {
    pub match_host: CHost,
    pub wait_delay: u32,
    pub use_firewalled: bool,
    pub trackers_only: bool,
    pub use_busy_relays: bool,
    pub use_busy_controls: bool,
    pub exclude_id: [u8; 16],
    pub num_results: i32,
}

/// `pickHits`。選んだホストの番号 (なければ -1) を返し、LAN 側のアドレスを使うなら `*lan` を true に。
///
/// # Safety
/// `hits` は `n` 個読めること。`s` は有効な `pcrs_hit_search`、`lan` は書き込めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hits_pick(hits: *const CHit, n: usize, s: *const CHitSearch, ctime: u32, lan: *mut bool) -> i32 {
    // SAFETY: 関数の Safety 節
    let (hits, s) = unsafe { (hits_of(hits, n), &*s) };
    let search = chanhit::Search {
        match_host: s.match_host.into(),
        wait_delay: s.wait_delay,
        use_firewalled: s.use_firewalled,
        trackers_only: s.trackers_only,
        use_busy_relays: s.use_busy_relays,
        use_busy_controls: s.use_busy_controls,
        exclude_id: s.exclude_id,
        num_results: s.num_results,
    };
    match chanhit::pick(&hits, &search, ctime) {
        Some(p) => {
            // SAFETY: 関数の Safety 節
            unsafe { *lan = p.lan };
            p.index as i32
        }
        None => -1,
    }
}

/// `clearDeadHits`。消すホストの `del` を true にし、残るホストの数を返す。
///
/// # Safety
/// `hits` は `n` 個読め、`del` は `n` 個書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hits_clear_dead(
    hits: *const CHit,
    n: usize,
    timeout: u32,
    clear_trackers: bool,
    ctime: u32,
    del: *mut bool,
) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe { chanhit::clear_dead(&hits_of(hits, n), timeout, clear_trackers, ctime, flags_of(del, n)) }
}

/// `deadHit` と `delHit` の対象 (`out` を true に)
///
/// # Safety
/// `hits` は `n` 個読め、`out` は `n` 個書け、`h` は有効な `pcrs_hit` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hits_same_hosts(hits: *const CHit, n: usize, h: *const CHit, out: *mut bool) {
    // SAFETY: 関数の Safety 節
    unsafe { chanhit::same_hosts(&hits_of(hits, n), &hit_of(h), flags_of(out, n)) }
}

/// `addHit`。-2 自分のホスト、-1 `del` のホストを消して先頭に加える、0 以上ならその番号を書き換える。
///
/// # Safety
/// `hits` は `n` 個読め、`del` は `n` 個書け、`h` は有効な `pcrs_hit`、`my_sid` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hits_add(hits: *const CHit, n: usize, h: *const CHit, my_sid: *const u8, del: *mut bool) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        let sid = *(my_sid as *const [u8; 16]);
        match chanhit::add(&hits_of(hits, n), &hit_of(h), &sid, flags_of(del, n)) {
            chanhit::Add::Own => -2,
            chanhit::Add::New => -1,
            chanhit::Add::Replace(i) => i as i32,
        }
    }
}

// ---- hostgraph (core/common/hostgraph.cpp の HostGraph のコンストラクター) ----

use crate::hostgraph;

/// `HostGraph` で使う `ChanHit` の欄 (C の `pcrs_graph_node`)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CGraphNode {
    pub rhost: [CHost; 2],
    pub uphost: CHost,
}

/// `HostGraph::HostGraph`。ID の順に、採った番号を `index` に、親 (の `index` の中の位置、
/// 根なら -1) を `parent` に書き、その数を返す。
///
/// # Safety
/// `nodes` は `n` 個読め、`index` と `parent` は `n` 個書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_hostgraph_build(
    nodes: *const CGraphNode,
    n: usize,
    index: *mut usize,
    parent: *mut isize,
) -> usize {
    if n == 0 {
        return 0;
    }
    // SAFETY: 関数の Safety 節
    let (nodes, index, parent) = unsafe {
        (
            std::slice::from_raw_parts(nodes, n),
            std::slice::from_raw_parts_mut(index, n),
            std::slice::from_raw_parts_mut(parent, n),
        )
    };
    let nodes: Vec<hostgraph::Node> = nodes
        .iter()
        .map(|c| hostgraph::Node { rhost: [c.rhost[0].into(), c.rhost[1].into()], uphost: c.uphost.into() })
        .collect();
    let g = hostgraph::build(&nodes);
    for (k, e) in g.iter().enumerate() {
        index[k] = e.index;
        parent[k] = e.parent.map_or(-1, |p| p as isize);
    }
    g.len()
}

// ---- uptest (core/common/uptest.cpp の通信しない部分) ----

use crate::uptest;

/// `UptestEndpoint::readInfo`。成功なら 0 で、`out` に `UptestInfo` の 14 個の欄を順に、
/// それぞれ NUL で終えて並べる。失敗なら 3〜6 (`pcrs_xml_read` と同じ)、7 "Too many
/// attributes"、8 "Bad tag value"、9 ノードか属性がない。
///
/// # Safety
/// `body` は `n` バイト読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_uptest_read_info(body: *const u8, n: usize, out: *mut PcrsBuf) -> i32 {
    use crate::xml::{AttrError, Error};
    // SAFETY: 関数の Safety 節
    match uptest::read_info(unsafe { input(body, n) }) {
        Ok(info) => {
            let mut v = Vec::new();
            for f in &info {
                v.extend_from_slice(f);
                v.push(0);
            }
            // SAFETY: 関数の Safety 節
            unsafe { *out = into_buf(v) };
            0
        }
        Err(uptest::ReadError::Xml(e)) => match e {
            Error::Abort => 1,
            Error::Callback => 2,
            Error::TagTooLong => 3,
            Error::ContentTooBig => 4,
            Error::NotXml => 5,
            Error::UnexpectedEndTag => 6,
        },
        Err(uptest::ReadError::Attr(AttrError::TooMany)) => 7,
        Err(uptest::ReadError::Attr(AttrError::BadValue)) => 8,
        Err(uptest::ReadError::Null) => 9,
    }
}

/// `UptestInfo::postURL`
///
/// # Safety
/// それぞれの先頭ポインタは、その長さだけ読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_uptest_post_url(
    addr: *const u8,
    addr_len: usize,
    port: *const u8,
    port_len: usize,
    object: *const u8,
    object_len: usize,
) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    unsafe { into_buf(uptest::post_url(input(addr, addr_len), input(port, port_len), input(object, object_len))) }
}

/// `UptestEndpoint::isReady`
#[no_mangle]
pub extern "C" fn pcrs_uptest_is_ready(status: i32, last_tried_at: u32, now: u32) -> bool {
    uptest::is_ready(status, last_tried_at, now)
}

/// `textStatus`。知らない値なら NULL。
#[no_mangle]
pub extern "C" fn pcrs_uptest_text_status(status: i32) -> *const std::ffi::c_char {
    uptest::text_status(status).map_or(std::ptr::null(), c_static)
}

/// `UptestServiceRegistry::addURL` の判断。加えてよければ NULL、だめなら理由。
///
/// # Safety
/// `scheme` と `url` はその長さだけ読め、`existing` は `count` 個の有効な `pcrs_bytes` を
/// 指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_uptest_check_add_url(
    valid: bool,
    scheme: *const u8,
    scheme_len: usize,
    url: *const u8,
    url_len: usize,
    existing: *const CBytes,
    count: usize,
) -> *const std::ffi::c_char {
    // SAFETY: 関数の Safety 節
    let (scheme, url, existing) = unsafe {
        let list: &[CBytes] = if count == 0 { &[] } else { std::slice::from_raw_parts(existing, count) };
        (
            input(scheme, scheme_len),
            input(url, url_len),
            list.iter().map(|b| input(b.ptr, b.len)).collect::<Vec<_>>(),
        )
    };
    match uptest::check_add_url(valid, scheme, url, &existing) {
        Ok(()) => std::ptr::null(),
        Err(msg) => c_static(msg),
    }
}

// ---- channel (core/common/channel.cpp と chanmgr.cpp の、スレッドやソケットに触らない部分) ----

use crate::channel;

/// `processMp3Metadata`。`StreamTitle` があれば 1、`StreamUrl` があれば 2 を足した値を返し、
/// 値の位置と長さを書く。
///
/// # Safety
/// `s` は `n` バイト読め、出力引数はどれも書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_channel_mp3_metadata(
    s: *const u8,
    n: usize,
    title_pos: *mut usize,
    title_len: *mut usize,
    url_pos: *mut usize,
    url_len: *mut usize,
) -> i32 {
    // SAFETY: 関数の Safety 節
    let (t, u) = channel::mp3_metadata(unsafe { input(s, n) });
    let mut r = 0;
    // SAFETY: 関数の Safety 節
    unsafe {
        if let Some((p, l)) = t {
            *title_pos = p;
            *title_len = l;
            r |= 1;
        }
        if let Some((p, l)) = u {
            *url_pos = p;
            *url_len = l;
            r |= 2;
        }
    }
    r
}

/// `writeTrackerUpdateAtom` の atom
///
/// # Safety
/// `info` と `hit` は有効なものを指し、`session_id` と `broadcast_id` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_channel_tracker_update_atom(
    info: *const CChanInfo,
    hit: *const CHit,
    session_id: *const u8,
    broadcast_id: *const u8,
) -> PcrsBuf {
    let mut b = AtomBuf::default();
    // SAFETY: 関数の Safety 節
    unsafe {
        channel::tracker_update_atom(
            &mut b,
            &*(session_id as *const [u8; 16]),
            &*(broadcast_id as *const [u8; 16]),
            &info_of(info),
            &hit_of(hit),
        );
    }
    into_buf(b.0)
}

/// `updateInfo` で中継先へ送る atom
///
/// # Safety
/// `info` は有効な `pcrs_chan_info` を指し、`session_id` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_channel_info_update_atom(info: *const CChanInfo, session_id: *const u8) -> PcrsBuf {
    let mut b = AtomBuf::default();
    // SAFETY: 関数の Safety 節
    unsafe { channel::info_update_atom(&mut b, &*(session_id as *const [u8; 16]), &info_of(info)) };
    into_buf(b.0)
}

bytes_to_buf!(
    /// `renderHexDump`
    pcrs_channel_hex_dump,
    channel::render_hex_dump
);

/// `getBufferString`
///
/// # Safety
/// `lens` は `n` 個読めること (`n` が 0 なら NULL でもよい)。
#[no_mangle]
pub unsafe extern "C" fn pcrs_channel_buffer_string(
    byterate: f64,
    now: u32,
    last_write_time: u32,
    lens: *const u32,
    n: usize,
    cont: i32,
    non_cont: i32,
) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    let lens: &[u32] = if n == 0 || lens.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(lens, n) } };
    into_buf(channel::buffer_string(byterate, now, last_write_time, lens, cont, non_cont))
}

/// `checkReadDelay`。眠るなら true を返し、時間 (ミリ秒) を `ms` に書く。
///
/// # Safety
/// `ms` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_channel_read_delay(read_delay: bool, len: u32, bitrate: i32, ms: *mut u32) -> bool {
    match channel::read_delay_ms(read_delay, len, bitrate) {
        Some(t) => {
            // SAFETY: 関数の Safety 節
            unsafe { *ms = t };
            true
        }
        None => false,
    }
}

/// `ChanMgr::authToken`
///
/// # Safety
/// `broadcast_id` と `id` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chanmgr_auth_token(broadcast_id: *const u8, id: *const u8) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    unsafe { into_buf(channel::auth_token(&*(broadcast_id as *const [u8; 16]), &*(id as *const [u8; 16]))) }
}

/// `ChanMgr::closeOldestIdle` で止めるチャンネルの番号。なければ -1。
///
/// # Safety
/// `idle` と `last_idle_time` は `n` 個読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_chanmgr_oldest_idle(idle: *const bool, last_idle_time: *const u32, n: usize) -> isize {
    if n == 0 {
        return -1;
    }
    // SAFETY: 関数の Safety 節
    let (idle, t) = unsafe { (std::slice::from_raw_parts(idle, n), std::slice::from_raw_parts(last_idle_time, n)) };
    channel::oldest_idle(idle, t).map_or(-1, |i| i as isize)
}

#[cfg(test)]
mod tests_7c {
    use super::*;

    #[test]
    fn uptest_strings_are_static() {
        for st in 0..3 {
            c_static(uptest::text_status(st).unwrap());
        }
        for (valid, scheme, url) in [(false, &b"http"[..], &b"a"[..]), (true, b"ftp", b"a"), (true, b"http", b"a")] {
            if let Err(m) = uptest::check_add_url(valid, scheme, url, &[b"a"]) {
                c_static(m);
            }
        }
    }
}

#[cfg(test)]
mod tests_7b {
    use super::*;

    #[test]
    fn table_strings_are_static() {
        for t in [&b"MP3"[..], b"OGG", b"OGM", b"RAW", b"MOV", b"MPG", b"FLV", b"MKV", b"WEBM", b"MP4", b"PLS", b"x", b""] {
            c_static(chaninfo::type_ext(t));
            c_static(chaninfo::mime_type(t));
            c_static(chaninfo::type_from_str(t));
            c_static(chaninfo::playlist_ext(t));
        }
        for m in [&b"application/x-ogg"[..], b"audio/mpeg", b"video/quicktime", b"video/mpeg", b"video/x-flv",
                  b"video/x-matroska", b"video/webm", b"x"] {
            c_static(chaninfo::type_from_mime(m));
        }
        for p in -1..7 {
            c_static(chaninfo::protocol_str(p));
        }
    }

    #[test]
    fn servhs_strings_are_static() {
        for v in [&b"application/ogg"[..], b"application/x-ogg", b"audio/mpeg", b"audio/x-mpeg", b"application/binary",
                  b"application/x-peercast-pcp", b"audio/x-scpls", b"audio/mpegurl", b"audio/x-mpegurl", b"audio/m3u",
                  b"text/plain"] {
            c_static(servhs::icy_content_type(v).unwrap());
        }
        for f in [&b".htm"[..], b".css", b".jpg", b".gif", b".png", b".js", b".ico"] {
            c_static(servhs::mime_type_for(f).unwrap());
        }
        for s in [&b""[..], b"-1", b"0", b"99999999"] {
            if let Err((line, _)) = servhs::jrpc_body_length(s, 1 << 20) {
                c_static(line.as_bytes());
            }
        }
    }

    #[test]
    fn public_strings_are_static() {
        for f in [&b"a.htm"[..], b"a.html", b"a.css", b"a.jpg", b"a.gif", b"a.png", b"a.js", b"a.svg", b"a.ico", b"a"] {
            c_static(public::mime_type(f));
            c_static(public::assets_mime_type(f));
        }
    }
}

// ---- jrpc (core/common/jrpc.cpp の JrpcApi) ----

use crate::jrpc;
use crate::json;

/// `pcrs_jrpc_host` の `call` に渡す引数 (C の `pcrs_jrpc_args`)
#[repr(C)]
pub struct CJrpcArgs {
    pub id: [u8; 16],
    pub i: i32,
    pub j: i32,
    pub a: CBytes,
    pub b: CBytes,
    /// `UPDATE_INFO` は 10 個、`FETCH` は url, name, desc, genre, contact, type の 6 個。ほかは NULL
    pub fields: *const CBytes,
}

/// `Channel` (C の `pcrs_jrpc_channel`)。中身は呼び出しの間だけ有効。
#[repr(C)]
pub struct CJrpcChannel {
    pub info: CChanInfo,
    pub status: i32,
    pub source_url: CBytes,
    pub source_host: CBytes,
    pub uptime: u32,
    pub local_relays: i32,
    pub local_directs: i32,
    pub total_relays: i32,
    pub total_directs: i32,
    pub is_broadcasting: bool,
    pub is_full: bool,
    pub is_receiving: bool,
    pub ip_version: i32,
    pub has_sock: bool,
    pub sock_host: CBytes,
    pub source_rate: i32,
    pub src_protocol: i32,
    pub stream_pos: u32,
}

/// `Servent` (C の `pcrs_jrpc_servent`)
#[repr(C)]
pub struct CJrpcServent {
    pub index: i32,
    pub type_str: CBytes,
    pub status_str: CBytes,
    pub send_rate: u32,
    pub recv_rate: u32,
    pub protocol: i32,
    pub agent: CBytes,
    pub has_sock: bool,
    pub sock_host: CBytes,
}

/// `ChannelEntry` (C の `pcrs_jrpc_yp`)
#[repr(C)]
pub struct CJrpcYp {
    pub feed_url: CBytes,
    pub name: CBytes,
    pub id: [u8; 16],
    pub tip: CBytes,
    pub url: CBytes,
    pub genre: CBytes,
    pub desc: CBytes,
    pub comment: CBytes,
    pub bitrate: i32,
    pub content_type: CBytes,
    pub track_name: CBytes,
    pub track_album: CBytes,
    pub track_artist: CBytes,
    pub track_contact: CBytes,
    pub num_directs: i32,
    pub num_relays: i32,
}

/// `ChanHitList` (C の `pcrs_jrpc_found`)
#[repr(C)]
pub struct CJrpcFound {
    pub info: CChanInfo,
    pub uptime: u32,
    pub skips: u32,
    pub age: u32,
    pub bcflags: u8,
    pub hosts: i32,
    pub listeners: i32,
    pub relays: i32,
    pub firewalled: i32,
    pub closest: i32,
    pub furthest: i32,
    pub newest: u32,
}

/// `ChanHit` (C の `pcrs_jrpc_found_hit`)
#[repr(C)]
pub struct CJrpcFoundHit {
    pub ip: CBytes,
    pub hops: u32,
    pub listeners: u32,
    pub relays: u32,
    pub uptime: u32,
    pub push: bool,
    pub relay: bool,
    pub direct: bool,
    pub cin: bool,
    pub stable: bool,
    pub version: u32,
    pub update: u32,
    pub tracker: bool,
}

/// C++ 側の状態 (C の `pcrs_jrpc_host`)
#[repr(C)]
pub struct CJrpcHost {
    pub ctx: *mut c_void,
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: i32, msg: *const u8, len: usize),
    /// 0 成功、1 例外、2 `std::domain_error` (どちらも `pcrs_jrpc_put_error` で `what()` を渡す)
    pub call: unsafe extern "C" fn(ctx: *mut c_void, op: i32, args: *const CJrpcArgs, out: *mut JrpcSink) -> i32,
}

/// JSON の値を C++ に渡す (C の `pcrs_json_builder`)
#[repr(C)]
pub struct CJsonBuilder {
    pub ctx: *mut c_void,
    pub null_value: unsafe extern "C" fn(ctx: *mut c_void),
    pub boolean: unsafe extern "C" fn(ctx: *mut c_void, v: bool),
    pub integer: unsafe extern "C" fn(ctx: *mut c_void, v: i64),
    pub unsigned_integer: unsafe extern "C" fn(ctx: *mut c_void, v: u64),
    pub number: unsafe extern "C" fn(ctx: *mut c_void, v: f64),
    pub string: unsafe extern "C" fn(ctx: *mut c_void, s: *const u8, n: usize),
    pub begin_array: unsafe extern "C" fn(ctx: *mut c_void),
    pub begin_object: unsafe extern "C" fn(ctx: *mut c_void),
    pub key: unsafe extern "C" fn(ctx: *mut c_void, s: *const u8, n: usize),
    pub end: unsafe extern "C" fn(ctx: *mut c_void),
}


const JRPC_AGENT: i32 = 0;
const JRPC_LOG_LINES: i32 = 1;
const JRPC_CLEAR_LOG: i32 = 2;
const JRPC_LOG_LEVEL: i32 = 3;
const JRPC_SET_LOG_LEVEL: i32 = 4;
const JRPC_FETCH: i32 = 5;
const JRPC_CHANNELS: i32 = 6;
const JRPC_FIND_CHANNEL: i32 = 7;
const JRPC_SERVENTS: i32 = 8;
const JRPC_STOP_CONNECTION: i32 = 9;
const JRPC_RELAY_TREE: i32 = 10;
const JRPC_BUMP: i32 = 11;
const JRPC_PLAY: i32 = 12;
const JRPC_STOP_CHANNEL: i32 = 13;
const JRPC_ROOT_HOST: i32 = 14;
const JRPC_CLEAR_ROOT_HOST: i32 = 15;
const JRPC_SETTINGS: i32 = 16;
const JRPC_SET_SETTING: i32 = 17;
const JRPC_STATUS: i32 = 18;
const JRPC_STATE: i32 = 19;
const JRPC_UPDATE_INFO: i32 = 20;
const JRPC_YP_CHANNELS: i32 = 21;
const JRPC_READ_STORAGE: i32 = 22;
const JRPC_WRITE_STORAGE: i32 = 23;
const JRPC_CHANNELS_FOUND: i32 = 24;

/// C++ のコールバックが結果を入れる所 (C では中身の見えない `pcrs_jrpc_sink`)
#[derive(Default)]
pub struct JrpcSink {
    bytes: Vec<Vec<u8>>,
    ints: Vec<i64>,
    channels: Vec<jrpc::ChannelData>,
    servents: Vec<jrpc::ServentData>,
    hits: Vec<(chanhit::Hit, Vec<u8>)>,
    yps: Vec<jrpc::YpEntry>,
    found: Vec<jrpc::FoundData>,
    error: Vec<u8>,
}

impl JrpcSink {
    fn int(&self, i: usize) -> i64 {
        self.ints.get(i).copied().unwrap_or(0)
    }
    fn take_bytes(&mut self, i: usize) -> Vec<u8> {
        self.bytes.get_mut(i).map(std::mem::take).unwrap_or_default()
    }
}

/// # Safety
/// `b` の中身は `b.len` バイト読めること。
unsafe fn owned(b: &CBytes) -> Vec<u8> {
    // SAFETY: 関数の Safety 節
    unsafe { input(b.ptr, b.len) }.to_vec()
}

/// # Safety
/// `c` の中のバイト列は読めること。
unsafe fn info_data(c: &CChanInfo) -> jrpc::InfoData {
    // SAFETY: 関数の Safety 節
    unsafe {
        jrpc::InfoData {
            id: c.id,
            name: owned(&c.name),
            content_type: owned(&c.content_type),
            mime: owned(&c.mime),
            desc: owned(&c.desc),
            genre: owned(&c.genre),
            url: owned(&c.url),
            comment: owned(&c.comment),
            bitrate: c.bitrate,
            track_contact: owned(&c.track_contact),
            track_title: owned(&c.track_title),
            track_artist: owned(&c.track_artist),
            track_album: owned(&c.track_album),
            track_genre: owned(&c.track_genre),
        }
    }
}

/// C++ のコールバックから、受け取り口を使う。
///
/// # Safety
/// `out` は `pcrs_jrpc_host` の `call` に渡された有効な受け取り口であること。
unsafe fn sink<'a>(out: *mut JrpcSink) -> &'a mut JrpcSink {
    // SAFETY: 関数の Safety 節
    unsafe { &mut *out }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`s` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_bytes(out: *mut JrpcSink, s: *const u8, n: usize) {
    // SAFETY: 関数の Safety 節
    unsafe { sink(out).bytes.push(input(s, n).to_vec()) }
}

/// # Safety
/// `out` は `call` に渡された受け取り口であること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_int(out: *mut JrpcSink, v: i64) {
    // SAFETY: 関数の Safety 節
    unsafe { sink(out).ints.push(v) }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`s` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_error(out: *mut JrpcSink, s: *const u8, n: usize) {
    // SAFETY: 関数の Safety 節
    unsafe { sink(out).error = input(s, n).to_vec() }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`c` は有効な `pcrs_jrpc_channel` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_channel(out: *mut JrpcSink, c: *const CJrpcChannel) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let c = &*c;
        sink(out).channels.push(jrpc::ChannelData {
            info: info_data(&c.info),
            status: c.status,
            source_url: owned(&c.source_url),
            source_host: owned(&c.source_host),
            uptime: c.uptime,
            local_relays: c.local_relays,
            local_directs: c.local_directs,
            total_relays: c.total_relays,
            total_directs: c.total_directs,
            is_broadcasting: c.is_broadcasting,
            is_full: c.is_full,
            is_receiving: c.is_receiving,
            ip_version: c.ip_version,
            sock_host: if c.has_sock { Some(owned(&c.sock_host)) } else { None },
            source_rate: c.source_rate,
            src_protocol: c.src_protocol,
            stream_pos: c.stream_pos,
        });
    }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`s` は有効な `pcrs_jrpc_servent` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_servent(out: *mut JrpcSink, s: *const CJrpcServent) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let s = &*s;
        sink(out).servents.push(jrpc::ServentData {
            index: s.index,
            type_str: owned(&s.type_str),
            status_str: owned(&s.status_str),
            send_rate: s.send_rate,
            recv_rate: s.recv_rate,
            protocol: s.protocol,
            agent: owned(&s.agent),
            sock_host: if s.has_sock { Some(owned(&s.sock_host)) } else { None },
        });
    }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`h` は有効な `pcrs_hit`、`addr` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_hit(out: *mut JrpcSink, h: *const CHit, addr: *const u8, n: usize) {
    // SAFETY: 関数の Safety 節
    unsafe { sink(out).hits.push((hit_of(h), input(addr, n).to_vec())) }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`y` は有効な `pcrs_jrpc_yp` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_yp(out: *mut JrpcSink, y: *const CJrpcYp) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let y = &*y;
        sink(out).yps.push(jrpc::YpEntry {
            feed_url: owned(&y.feed_url),
            name: owned(&y.name),
            id: y.id,
            tip: owned(&y.tip),
            url: owned(&y.url),
            genre: owned(&y.genre),
            desc: owned(&y.desc),
            comment: owned(&y.comment),
            bitrate: y.bitrate,
            content_type: owned(&y.content_type),
            track_name: owned(&y.track_name),
            track_album: owned(&y.track_album),
            track_artist: owned(&y.track_artist),
            track_contact: owned(&y.track_contact),
            num_directs: y.num_directs,
            num_relays: y.num_relays,
        });
    }
}

/// ヒットリストを 1 つ加える。続く `pcrs_jrpc_put_found_hit` は、このヒットリストのヒットになる。
///
/// # Safety
/// `out` は `call` に渡された受け取り口、`f` は有効な `pcrs_jrpc_found` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_found(out: *mut JrpcSink, f: *const CJrpcFound) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let f = &*f;
        sink(out).found.push(jrpc::FoundData {
            info: info_data(&f.info),
            uptime: f.uptime,
            skips: f.skips,
            age: f.age,
            bcflags: f.bcflags,
            hosts: f.hosts,
            listeners: f.listeners,
            relays: f.relays,
            firewalled: f.firewalled,
            closest: f.closest,
            furthest: f.furthest,
            newest: f.newest,
            hits: Vec::new(),
        });
    }
}

/// # Safety
/// `out` は `call` に渡された受け取り口、`h` は有効な `pcrs_jrpc_found_hit` を指すこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_put_found_hit(out: *mut JrpcSink, h: *const CJrpcFoundHit) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let h = &*h;
        let hit = jrpc::FoundHit {
            ip: owned(&h.ip),
            hops: h.hops,
            listeners: h.listeners,
            relays: h.relays,
            uptime: h.uptime,
            push: h.push,
            relay: h.relay,
            direct: h.direct,
            cin: h.cin,
            stable: h.stable,
            version: h.version,
            update: h.update,
            tracker: h.tracker,
        };
        if let Some(f) = sink(out).found.last_mut() {
            f.hits.push(hit);
        }
    }
}

struct FfiJrpcHost<'a>(&'a CJrpcHost);

impl FfiJrpcHost<'_> {
    fn call(&mut self, op: i32, id: &[u8; 16], i: i32, j: i32, a: &[u8], b: &[u8], fields: &[&[u8]]) -> jrpc::HostResult<JrpcSink> {
        let cf: Vec<CBytes> = fields.iter().map(|f| CBytes::of(f)).collect();
        let args = CJrpcArgs {
            id: *id,
            i,
            j,
            a: CBytes::of(a),
            b: CBytes::of(b),
            fields: if cf.is_empty() { std::ptr::null() } else { cf.as_ptr() },
        };
        let mut out = JrpcSink::default();
        // SAFETY: ホストは C++ 側が有効なものを渡す (pcrs_jrpc_call の Safety 節)。args と out は
        // この呼び出しの間だけ有効で、C++ はそれより長く持たない。
        let r = unsafe { (self.0.call)(self.0.ctx, op, &args, &mut out) };
        match r {
            0 => Ok(out),
            2 => Err(jrpc::HostError::DomainError(out.error)),
            _ => Err(jrpc::HostError::Exception(out.error)),
        }
    }

    fn op(&mut self, op: i32) -> jrpc::HostResult<JrpcSink> {
        self.call(op, &[0; 16], 0, 0, &[], &[], &[])
    }

    fn op_id(&mut self, op: i32, id: &[u8; 16]) -> jrpc::HostResult<JrpcSink> {
        self.call(op, id, 0, 0, &[], &[], &[])
    }
}

impl jrpc::Host for FfiJrpcHost<'_> {
    fn log(&mut self, level: jrpc::Level, msg: &[u8]) {
        let l = match level {
            jrpc::Level::Debug => 0,
            jrpc::Level::Info => 1,
            jrpc::Level::Warn => 2,
            jrpc::Level::Error => 3,
        };
        // SAFETY: CJrpcHost::call と同じ
        unsafe { (self.0.log)(self.0.ctx, l, msg.as_ptr(), msg.len()) }
    }
    fn agent(&mut self) -> Vec<u8> {
        self.op(JRPC_AGENT).map(|mut s| s.take_bytes(0)).unwrap_or_default()
    }
    fn log_lines(&mut self) -> jrpc::HostResult<Vec<Vec<u8>>> {
        Ok(self.op(JRPC_LOG_LINES)?.bytes)
    }
    fn clear_log(&mut self) -> jrpc::HostResult<()> {
        self.op(JRPC_CLEAR_LOG).map(|_| ())
    }
    fn log_level(&mut self) -> jrpc::HostResult<i32> {
        Ok(self.op(JRPC_LOG_LEVEL)?.int(0) as i32)
    }
    fn set_log_level(&mut self, level: i32) -> jrpc::HostResult<()> {
        self.call(JRPC_SET_LOG_LEVEL, &[0; 16], level, 0, &[], &[], &[]).map(|_| ())
    }
    fn fetch(&mut self, req: &jrpc::FetchRequest) -> jrpc::HostResult<Option<[u8; 16]>> {
        let fields: [&[u8]; 6] = [&req.url, &req.name, &req.desc, &req.genre, &req.contact, &req.type_str];
        let mut s = self.call(JRPC_FETCH, &[0; 16], req.bitrate, req.ipv6 as i32, &[], &[], &fields)?;
        Ok(match s.bytes.first() {
            Some(b) if b.len() == 16 => {
                let mut id = [0u8; 16];
                id.copy_from_slice(&s.take_bytes(0));
                Some(id)
            }
            _ => None,
        })
    }
    fn channels(&mut self) -> jrpc::HostResult<Vec<jrpc::ChannelData>> {
        Ok(self.op(JRPC_CHANNELS)?.channels)
    }
    fn find_channel(&mut self, id: &[u8; 16]) -> jrpc::HostResult<Option<jrpc::ChannelData>> {
        Ok(self.op_id(JRPC_FIND_CHANNEL, id)?.channels.pop())
    }
    fn servents(&mut self, id: &[u8; 16]) -> jrpc::HostResult<Vec<jrpc::ServentData>> {
        Ok(self.op_id(JRPC_SERVENTS, id)?.servents)
    }
    fn stop_connection(&mut self, id: &[u8; 16], connection_id: i32) -> jrpc::HostResult<bool> {
        Ok(self.call(JRPC_STOP_CONNECTION, id, connection_id, 0, &[], &[], &[])?.int(0) != 0)
    }
    fn relay_tree(&mut self, id: &[u8; 16]) -> jrpc::HostResult<jrpc::RelayTreeData> {
        let s = self.op_id(JRPC_RELAY_TREE, id)?;
        Ok(match s.int(0) {
            0 => jrpc::RelayTreeData::NoChannel,
            1 => jrpc::RelayTreeData::NoHitList,
            _ => jrpc::RelayTreeData::Hits(s.hits),
        })
    }
    fn bump(&mut self, id: &[u8; 16]) -> jrpc::HostResult<bool> {
        Ok(self.op_id(JRPC_BUMP, id)?.int(0) != 0)
    }
    fn play(&mut self, id: &[u8; 16]) -> jrpc::HostResult<()> {
        self.op_id(JRPC_PLAY, id).map(|_| ())
    }
    fn stop_channel(&mut self, id: &[u8; 16]) -> jrpc::HostResult<()> {
        self.op_id(JRPC_STOP_CHANNEL, id).map(|_| ())
    }
    fn root_host(&mut self) -> jrpc::HostResult<Vec<u8>> {
        Ok(self.op(JRPC_ROOT_HOST)?.take_bytes(0))
    }
    fn clear_root_host(&mut self) -> jrpc::HostResult<()> {
        self.op(JRPC_CLEAR_ROOT_HOST).map(|_| ())
    }
    fn settings(&mut self) -> jrpc::HostResult<jrpc::Settings> {
        let s = self.op(JRPC_SETTINGS)?;
        Ok(jrpc::Settings {
            max_relays: s.int(0) as u32,
            max_relays_per_channel: s.int(1) as i32,
            max_direct: s.int(2) as u32,
            max_bitrate_out: s.int(3) as u32,
        })
    }
    fn set_setting(&mut self, key: jrpc::SettingKey, value: i32) -> jrpc::HostResult<()> {
        let k = match key {
            jrpc::SettingKey::MaxRelays => 0,
            jrpc::SettingKey::MaxRelaysPerChannel => 1,
            jrpc::SettingKey::MaxDirect => 2,
            jrpc::SettingKey::MaxBitrateOut => 3,
        };
        self.call(JRPC_SET_SETTING, &[0; 16], k, value, &[], &[], &[]).map(|_| ())
    }
    fn status(&mut self) -> jrpc::HostResult<jrpc::Status> {
        let mut s = self.op(JRPC_STATUS)?;
        Ok(jrpc::Status {
            uptime: s.int(0) as u32,
            firewall: s.int(1) as i32,
            port: s.int(2) as u16,
            global_ip: s.take_bytes(0),
            local_ip: s.take_bytes(1),
        })
    }
    fn state(&mut self, which: usize) -> jrpc::HostResult<Vec<u8>> {
        Ok(self.call(JRPC_STATE, &[0; 16], which as i32, 0, &[], &[], &[])?.take_bytes(0))
    }
    fn update_info(&mut self, id: &[u8; 16], fields: &[Vec<u8>; 10]) -> jrpc::HostResult<()> {
        let f: Vec<&[u8]> = fields.iter().map(|v| v.as_slice()).collect();
        self.call(JRPC_UPDATE_INFO, id, 0, 0, &[], &[], &f).map(|_| ())
    }
    fn yp_channels(&mut self) -> jrpc::HostResult<Vec<jrpc::YpEntry>> {
        Ok(self.op(JRPC_YP_CHANNELS)?.yps)
    }
    fn read_storage(&mut self, key: &[u8]) -> jrpc::HostResult<Option<Vec<u8>>> {
        let mut s = self.call(JRPC_READ_STORAGE, &[0; 16], 0, 0, key, &[], &[])?;
        Ok(if s.int(0) != 0 { Some(s.take_bytes(0)) } else { None })
    }
    fn write_storage(&mut self, key: &[u8], value: &[u8]) -> jrpc::HostResult<()> {
        self.call(JRPC_WRITE_STORAGE, &[0; 16], 0, 0, key, value, &[]).map(|_| ())
    }
    fn channels_found(&mut self) -> jrpc::HostResult<Vec<jrpc::FoundData>> {
        Ok(self.op(JRPC_CHANNELS_FOUND)?.found)
    }
}

/// `JrpcApi::call`。0 なら `out` に応答の JSON、1 なら応答を書き出せなかった例外の `what()`。
///
/// # Safety
/// `req` は `n` バイト読めること。`host` は有効な `pcrs_jrpc_host` を指し、そのコールバックは
/// 渡された受け取り口と引数をその呼び出しの間だけ使うこと。`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_call(req: *const u8, n: usize, host: *const CJrpcHost, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    let (req, host) = unsafe { (input(req, n), &*host) };
    let (code, buf) = match jrpc::call(req, &mut FfiJrpcHost(host)) {
        Ok(r) => (0, r),
        Err(w) => (1, w),
    };
    // SAFETY: 関数の Safety 節
    unsafe { *out = into_buf(buf) };
    code
}

fn build_json(v: &json::Value, b: &CJsonBuilder) {
    // SAFETY: pcrs_jrpc_invoke の Safety 節
    unsafe {
        match v {
            json::Value::Null => (b.null_value)(b.ctx),
            json::Value::Bool(x) => (b.boolean)(b.ctx, *x),
            json::Value::Int(x) => (b.integer)(b.ctx, *x),
            json::Value::UInt(x) => (b.unsigned_integer)(b.ctx, *x),
            json::Value::Float(x) => (b.number)(b.ctx, *x),
            json::Value::Str(s) => (b.string)(b.ctx, s.as_ptr(), s.len()),
            json::Value::Array(a) => {
                (b.begin_array)(b.ctx);
                for e in a {
                    build_json(e, b);
                }
                (b.end)(b.ctx);
            }
            json::Value::Object(o) => {
                (b.begin_object)(b.ctx);
                for (k, e) in o {
                    (b.key)(b.ctx, k.as_ptr(), k.len());
                    build_json(e, b);
                }
                (b.end)(b.ctx);
            }
        }
    }
}

/// C++ から JrpcApi のメソッドを直接呼ぶ。`args` は位置引数の配列の JSON。
/// 0 なら結果を `builder` に通知する。1 method_not_found、2 invalid_params、3 application_error
/// (`code` に番号)、4 そのほかの例外。1〜4 は `what` に `what()` を入れる。
///
/// # Safety
/// `method` と `args` はそれぞれの長さ読めること。`host` は `pcrs_jrpc_call` と同じ。
/// `builder` は有効な `pcrs_json_builder` を指し、`code` と `what` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_jrpc_invoke(
    method: *const u8,
    method_len: usize,
    args: *const u8,
    args_len: usize,
    host: *const CJrpcHost,
    builder: *const CJsonBuilder,
    code: *mut i32,
    what: *mut PcrsBuf,
) -> i32 {
    // SAFETY: 関数の Safety 節
    let (method, args, host, builder) = unsafe { (input(method, method_len), input(args, args_len), &*host, &*builder) };
    let args = match json::parse(args) {
        Ok(json::Value::Array(a)) => a,
        Ok(_) => Vec::new(),
        Err(e) => {
            // SAFETY: 関数の Safety 節
            unsafe { *what = into_buf(e.what().to_vec()) };
            return 4;
        }
    };
    let (r, c, mut w) = match jrpc::invoke(method, args, &mut FfiJrpcHost(host)) {
        Ok(v) => {
            build_json(&v, builder);
            (0, 0, Vec::new())
        }
        Err(jrpc::CallError::MethodNotFound(w)) => (1, 0, w),
        Err(jrpc::CallError::InvalidParams(w)) => (2, 0, w),
        Err(jrpc::CallError::Application(c, w)) => (3, c, w),
        Err(jrpc::CallError::Internal(w)) => (4, 0, w),
    };
    // what() は C の文字列なので NUL の手前まで
    if let Some(n) = w.iter().position(|&b| b == 0) {
        w.truncate(n);
    }
    // SAFETY: 関数の Safety 節
    unsafe {
        *code = c;
        *what = into_buf(w);
    }
    r
}

// ---- servhs (core/common/servhs.cpp の要求の解釈と判断) ----

use crate::servhs;

/// `ptr` を `n` バイト読む。
///
/// # Safety
/// `input` と同じ。
unsafe fn bytes<'a>(ptr: *const u8, n: usize) -> &'a [u8] {
    // SAFETY: 関数の Safety 節
    unsafe { input(ptr, n) }
}

/// # Safety
/// `out` は書けること。
unsafe fn put(out: *mut PcrsBuf, v: Vec<u8>) {
    // SAFETY: 関数の Safety 節
    unsafe { *out = into_buf(v) };
}

/// `handshakeHTTP` の振り分け (`servhs::RequestKind` の番号)
///
/// # Safety
/// `line` は `n` バイト、`password` は `pn` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_request_kind(line: *const u8, n: usize, password: *const u8, pn: usize) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe { servhs::request_kind(bytes(line, n), bytes(password, pn)) as i32 }
}

// `handshakeIncoming` の `stristr(buf, HTTP_PROTO1)`
bytes_to_bool!(pcrs_servhs_is_http, servhs::is_http);

// `ServMgr::isValidHtmlPath`
bytes_to_bool!(pcrs_servhs_is_valid_html_path, servhs::is_valid_html_path);

// `isDecimal`
bytes_to_bool!(pcrs_servhs_is_decimal, servhs::is_decimal);

/// `handshakeGET` の振り分け (`servhs::GetKind` の番号)。C++ 版が NUL を書く位置があれば `*has_cut` を
/// true にして、`fn` からの位置 (-1 もある) を `*cut` に書く。
///
/// # Safety
/// `path` は `n` バイト読め、`has_cut` と `cut` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_get_route(path: *const u8, n: usize, has_cut: *mut bool, cut: *mut isize) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        let (kind, c) = servhs::get_route(bytes(path, n));
        *has_cut = c.is_some();
        *cut = c.unwrap_or(0);
        kind as i32
    }
}

/// `/admin.cgi` の引数。`pass=` と `song=` があれば true で、値を書く (`mount=` と `url=` はあれば
/// `*has_*` を true にする)。false のときも出力は常に書く (`pcrs_buf_free` で返す)。
///
/// # Safety
/// `fn_` は `n` バイト読め、出力はすべて書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_admin_cgi(
    fn_: *const u8,
    n: usize,
    song: *mut PcrsBuf,
    has_mount: *mut bool,
    mount: *mut PcrsBuf,
    has_url: *mut bool,
    url: *mut PcrsBuf,
) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let a = servhs::admin_cgi(bytes(fn_, n));
        let ok = a.is_some();
        let a = a.unwrap_or(servhs::AdminCgi { song: Vec::new(), mount: None, url: None });
        *has_mount = a.mount.is_some();
        *has_url = a.url.is_some();
        put(song, a.song);
        put(mount, a.mount.unwrap_or_default());
        put(url, a.url.unwrap_or_default());
        ok
    }
}

/// `handshakePOST` の振り分け (`servhs::PostKind` の番号)。行が 3 つに分かれなければ -1。
/// `*args` は常に書く。
///
/// # Safety
/// `line` は `n` バイト読め、`args` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_post_route(line: *const u8, n: usize, args: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        match servhs::post_route(bytes(line, n)) {
            Some((kind, a)) => {
                put(args, a);
                kind as i32
            }
            None => {
                put(args, Vec::new());
                -1
            }
        }
    }
}

/// `handshakeGIV` のチャンネル ID
///
/// # Safety
/// `line` は `n` バイト読め、`id` は 16 バイト書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_giv_id(line: *const u8, n: usize, id: *mut u8) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let v = servhs::giv_id(bytes(line, n));
        std::ptr::copy_nonoverlapping(v.as_ptr(), id, 16);
    }
}

/// `handshakeSOURCE`。ICY の行ならパスワードがあるので true。出力は常に書く。
///
/// # Safety
/// `line` は `n` バイト読め、`password` と `mount` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_source(line: *const u8, n: usize, password: *mut PcrsBuf, mount: *mut PcrsBuf) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let s = servhs::source(bytes(line, n));
        let has = s.password.is_some();
        put(password, s.password.unwrap_or_default());
        put(mount, s.mount);
        has
    }
}

/// `Servent::hasValidAuthToken`
///
/// # Safety
/// `s` は `n` バイト、`broadcast_id` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_valid_auth_token(s: *const u8, n: usize, broadcast_id: *const u8) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let mut bcid = [0u8; 16];
        std::ptr::copy_nonoverlapping(broadcast_id, bcid.as_mut_ptr(), 16);
        servhs::valid_auth_token(bytes(s, n), &bcid)
    }
}

/// `handshakeAuth` の Cookie ヘッダー。0 見つからない、1 見つかった (`*id` に値)、2 `=` のない組が
/// あった。`*id` は常に書く。
///
/// # Safety
/// `header` は `n` バイト読め、`id` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_cookie_id(header: *const u8, n: usize, port: u16, id: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        match servhs::cookie_id(bytes(header, n), port) {
            servhs::CookieParse::NotFound => {
                put(id, Vec::new());
                0
            }
            servhs::CookieParse::Found(v) => {
                put(id, v);
                1
            }
            servhs::CookieParse::Invalid => {
                put(id, Vec::new());
                2
            }
        }
    }
}

/// `nextCGIarg` を最後まで繰り返したもの。名前と値を交互に並べる。
///
/// # Safety
/// `cmd` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_cgi_args(cmd: *const u8, n: usize) -> PcrsVec {
    // SAFETY: 関数の Safety 節
    let args = servhs::cgi_args(unsafe { bytes(cmd, n) });
    into_vec(args.into_iter().flat_map(|(k, v)| [k, v]).collect())
}

/// `CMD_apply` の引数を読み、行うことを順に `op` で知らせる (`servhs::ApplyKey` の番号、数、文字列)。
///
/// # Safety
/// `cmd` は `n` バイト読めること。`op` は例外を投げないこと。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_apply_ops(
    cmd: *const u8,
    n: usize,
    ctx: *mut c_void,
    op: unsafe extern "C" fn(ctx: *mut c_void, key: i32, value: i32, s: *const u8, len: usize),
) {
    // SAFETY: 関数の Safety 節
    for o in servhs::apply_ops(unsafe { bytes(cmd, n) }) {
        // SAFETY: 関数の Safety 節
        unsafe { op(ctx, o.key as i32, o.int, o.str.as_ptr(), o.str.len()) };
    }
}

/// `CMD_redirect` の飛び先。`url=` がなければ false (`*out` は空)。
///
/// # Safety
/// `cmd` は `n` バイト読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_redirect_url(cmd: *const u8, n: usize, out: *mut PcrsBuf) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let r = servhs::redirect_url(bytes(cmd, n));
        let ok = r.is_some();
        put(out, r.unwrap_or_default());
        ok
    }
}

/// `CMD_chooseLanguage` の Referer の書き換え。見つからなければ false (`*out` は空)。
///
/// # Safety
/// `referer` と `path` はそれぞれの長さ読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_rewrite_referer(
    referer: *const u8,
    n: usize,
    path: *const u8,
    pn: usize,
    out: *mut PcrsBuf,
) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let r = servhs::rewrite_referer(bytes(referer, n), bytes(path, pn));
        let ok = r.is_some();
        put(out, r.unwrap_or_default());
        ok
    }
}

/// `readICYHeader` のヘッダーの行が何か (`servhs::IcyHeader` の番号)
///
/// # Safety
/// `line` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_icy_header(line: *const u8, n: usize) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe { servhs::icy_header(bytes(line, n)) as i32 }
}

/// `readICYHeader` の content-type の種類 ("OGG" などか、"PCP")。当てはまらなければ NULL。
///
/// # Safety
/// `value` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_icy_content_type(value: *const u8, n: usize) -> *const std::ffi::c_char {
    // SAFETY: 関数の Safety 節
    match servhs::icy_content_type(unsafe { bytes(value, n) }) {
        Some(t) => c_static(t),
        None => std::ptr::null(),
    }
}

/// `Servent::fileNameToMimeType`。当てはまらなければ NULL。
///
/// # Safety
/// `name` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_mime_type(name: *const u8, n: usize) -> *const std::ffi::c_char {
    // SAFETY: 関数の Safety 節
    match servhs::mime_type_for(unsafe { bytes(name, n) }) {
        Some(t) => c_static(t),
        None => std::ptr::null(),
    }
}

/// `handshakeLocalFile` のページの種類 (`servhs::LocalPage` の番号) と、`?` の後ろの `id`。
///
/// # Safety
/// `fn_` は `n` バイト読め、`split_ok` と `id` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_local_file(fn_: *const u8, n: usize, split_ok: *mut bool, id: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        let f = servhs::local_file(bytes(fn_, n));
        *split_ok = f.split_ok;
        put(id, f.id);
        f.page as i32
    }
}

/// `handshakeLocalFile` の `String fileName = documentRoot; fileName.append(fn)`
///
/// # Safety
/// `root` と `fn_` はそれぞれの長さ読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_local_file_name(root: *const u8, rn: usize, fn_: *const u8, n: usize) -> PcrsBuf {
    // SAFETY: 関数の Safety 節
    into_buf(servhs::local_file_name(unsafe { bytes(root, rn) }, unsafe { bytes(fn_, n) }))
}

/// `invokeCGIScript` の SERVER_NAME。Host ヘッダーが `名前:ポート` の形でなければ false。
///
/// # Safety
/// `host` は `n` バイト読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_cgi_server_name(host: *const u8, n: usize, out: *mut PcrsBuf) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let r = servhs::cgi_server_name(bytes(host, n));
        let ok = r.is_some();
        put(out, r.unwrap_or_default());
        ok
    }
}

/// CGI スクリプトの出力のヘッダーの行。形が合わなければ false。出力は常に書く。
///
/// # Safety
/// `line` は `n` バイト読め、`name` と `value` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_cgi_header_line(line: *const u8, n: usize, name: *mut PcrsBuf, value: *mut PcrsBuf) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        match servhs::cgi_header_line(bytes(line, n)) {
            Some((k, v)) => {
                put(name, k);
                put(value, v);
                true
            }
            None => {
                put(name, Vec::new());
                put(value, Vec::new());
                false
            }
        }
    }
}

/// `handshakeJRPC` の本体の長さ。だめなら負の数 (-411、-400、-413) で、`*status_line` に返す状態の行。
///
/// # Safety
/// `s` は `n` バイト読め、`status_line` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servhs_jrpc_body_length(
    s: *const u8,
    n: usize,
    max: i32,
    status_line: *mut *const std::ffi::c_char,
) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        match servhs::jrpc_body_length(bytes(s, n), max) {
            Ok(len) => {
                *status_line = std::ptr::null();
                len
            }
            Err((line, code)) => {
                *status_line = c_static(line.as_bytes());
                -code
            }
        }
    }
}

// ---- 段階 8c: mapper、assets、public、HTTPRequest ----

use crate::mapper;

/// `FileSystemMapper::toLocalFilePath` の前半。`vpath` が `virtual_path` の下でなければ false。
///
/// # Safety
/// 各入力はそれぞれの長さ読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_mapper_local_path(
    virtual_path: *const u8,
    vn: usize,
    document_root: *const u8,
    dn: usize,
    vpath: *const u8,
    n: usize,
    out: *mut PcrsBuf,
) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let r = mapper::local_path(bytes(virtual_path, vn), bytes(document_root, dn), bytes(vpath, n));
        let ok = r.is_some();
        put(out, r.unwrap_or_default());
        ok
    }
}

/// `FileSystemMapper::resolvePath` で試すパスと言語を、(パス, 言語) の順に並べる。
///
/// # Safety
/// `raw` は `n` バイト読めること。`langs_*` は `pcrs_str_join` の引数と同じく読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_mapper_candidates(
    raw: *const u8,
    n: usize,
    langs_joined: *const u8,
    langs_joined_len: usize,
    langs_lens: *const usize,
    langs_count: usize,
) -> PcrsVec {
    // SAFETY: 関数の Safety 節
    let (raw, langs) = unsafe { (bytes(raw, n), input_parts(langs_joined, langs_joined_len, langs_lens, langs_count)) };
    let langs = langs.unwrap_or_default();
    into_vec(mapper::candidates(raw, &langs).into_iter().flat_map(|(p, l)| [p, l]).collect())
}

/// 解決したパスが文書のディレクトリの中にあるか
///
/// # Safety
/// 各入力はそれぞれの長さ読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_mapper_inside(document_root: *const u8, dn: usize, resolved: *const u8, n: usize) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe { mapper::inside(bytes(document_root, dn), bytes(resolved, n)) }
}

/// `PublicController::operator()` の振り分け (`public::Route` の番号)
///
/// # Safety
/// `path` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_public_route(path: *const u8, n: usize) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe { public::route(bytes(path, n)) as i32 }
}

/// public.cpp の `MIMEType`
///
/// # Safety
/// `path` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_public_mime_type(path: *const u8, n: usize) -> *const std::ffi::c_char {
    // SAFETY: 関数の Safety 節
    c_static(public::mime_type(unsafe { bytes(path, n) }))
}

/// assets.cpp の `MIMEType`
///
/// # Safety
/// `path` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_assets_mime_type(path: *const u8, n: usize) -> *const std::ffi::c_char {
    // SAFETY: 関数の Safety 節
    c_static(public::assets_mime_type(unsafe { bytes(path, n) }))
}

/// `AssetsController::operator()` で 304 を返すか
///
/// # Safety
/// `ims` は `n` バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_assets_not_modified(last_modified: i64, ims: *const u8, n: usize) -> bool {
    // SAFETY: 関数の Safety 節
    public::not_modified(last_modified, unsafe { bytes(ims, n) })
}

/// `HTTPRequest` のコンストラクターの、URL のパスとクエリーへの分割
///
/// # Safety
/// `url` は `n` バイト読め、`path` と `query` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_http_split_url(url: *const u8, n: usize, path: *mut PcrsBuf, query: *mut PcrsBuf) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let (p, q) = crate::http::split_request_url(bytes(url, n));
        put(path, p);
        put(query, q);
    }
}

/// `PublicController::createChannelIndex`。0 なら `out` に index.txt、1 なら例外の `what()`。
///
/// # Safety
/// `host` は `pcrs_jrpc_call` と同じ。`tip` は `n` バイト読め、`out` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_public_channel_index(host: *const CJrpcHost, tip: *const u8, n: usize, out: *mut PcrsBuf) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        let (code, buf) = match jrpc::channel_index(&mut FfiJrpcHost(&*host), bytes(tip, n)) {
            Ok(r) => (0, r),
            Err(w) => (1, w),
        };
        put(out, buf);
        code
    }
}

// ---- 段階 9a: 正規表現 (差分テストで std::regex と比べるため) ----

/// 。正規表現の誤りなら -1、一致しなければ 0、一致すれば 1 で  に各グループ
/// (一致しなかったグループは空)。
///
/// # Safety
///  と  はそれぞれの長さ読め、 は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_regex_exec(pattern: *const u8, pn: usize, subject: *const u8, sn: usize, out: *mut PcrsVec) -> i32 {
    // SAFETY: 関数の Safety 節
    unsafe {
        match crate::server::regex::Regex::new(bytes(pattern, pn)) {
            Err(_) => {
                *out = into_vec(Vec::new());
                -1
            }
            Ok(r) => {
                let v = r.exec(bytes(subject, sn));
                let found = !v.is_empty();
                *out = into_vec(v);
                found as i32
            }
        }
    }
}

// ---- 段階 9a: IP アドレス、ホスト、フィルター (差分テストで C++ 版と比べるため) ----

use crate::server::host::{Host as SHost, Ip as SIp};

/// `IP::tryParse`。読めれば true で `*out` に 16 バイト。
///
/// # Safety
/// `s` は `n` バイト読め、`out` は 16 バイト書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_ip_parse(s: *const u8, n: usize, out: *mut u8) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        match SIp::parse(bytes(s, n)) {
            Some(ip) => {
                std::ptr::copy_nonoverlapping(ip.0.as_ptr(), out, 16);
                true
            }
            None => false,
        }
    }
}

/// `IP::str`
///
/// # Safety
/// `ip` は 16 バイト読めること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_ip_str(ip: *const u8) -> PcrsBuf {
    let mut a = [0u8; 16];
    // SAFETY: 関数の Safety 節
    unsafe { std::ptr::copy_nonoverlapping(ip, a.as_mut_ptr(), 16) };
    into_buf(SIp(a).str().into_bytes())
}

/// `Host::fromStrIP` (`name` が false) か `Host::fromStrName` (true)。`*ip` に 16 バイト、`*port` にポート。
///
/// # Safety
/// `s` は `n` バイト読め、`ip` は 16 バイト、`port` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_host_from_str(s: *const u8, n: usize, default_port: u16, name: bool, ip: *mut u8, port: *mut u16) {
    // SAFETY: 関数の Safety 節
    unsafe {
        let h = if name { SHost::from_str_name(bytes(s, n), default_port) } else { SHost::from_str_ip(bytes(s, n), default_port) };
        std::ptr::copy_nonoverlapping(h.ip.0.as_ptr(), ip, 16);
        *port = h.port;
    }
}

/// `ServFilter`: `pattern` を `setPattern` したものの `getPattern` と、`flags` を付けたときに `fl` で
/// ホスト (`ip`、`port`) に一致するか (`matches`)。`*global` に `isGlobal`、`*set` に `isSet`。
///
/// # Safety
/// `pattern` は `n` バイト、`ip` は 16 バイト読め、`out`、`global`、`set` は書けること。
#[no_mangle]
pub unsafe extern "C" fn pcrs_servfilter_probe(
    pattern: *const u8,
    n: usize,
    flags: u32,
    ip: *const u8,
    port: u16,
    fl: u32,
    out: *mut PcrsBuf,
    global: *mut bool,
    set: *mut bool,
) -> bool {
    // SAFETY: 関数の Safety 節
    unsafe {
        let mut f = crate::server::servfilter::ServFilter::default();
        f.set_pattern(bytes(pattern, n));
        f.flags = flags;
        let mut a = [0u8; 16];
        std::ptr::copy_nonoverlapping(ip, a.as_mut_ptr(), 16);
        put(out, f.pattern());
        *global = f.is_global();
        *set = f.is_set();
        f.matches(fl, &SHost::new(SIp(a), port))
    }
}
