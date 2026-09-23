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
