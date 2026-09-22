/*
 * peercast-rs の C ABI。実装は src/ffi.rs。
 *
 * 約束事:
 *  - 入力は「先頭ポインタ + 長さ」のバイト列 (std::string の data() と size())。
 *    長さが 0 のときは、ポインタは NULL でもよい。
 *  - 出力は pcrs_buf。Rust が確保したバイト列なので、使い終わったら必ず pcrs_buf_free で返す
 *    (free() や delete では解放しない)。
 *  - 失敗しうる関数は、成功で 0、失敗で -1 を返す。失敗のとき *out には何も書かれない。
 */
#ifndef PEERCAST_RS_H
#define PEERCAST_RS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct pcrs_buf {
    uint8_t *ptr;
    size_t len;
} pcrs_buf;

void pcrs_buf_free(pcrs_buf buf);

/* cgi (core/common/cgi.cpp) */
pcrs_buf pcrs_cgi_escape(const uint8_t *s, size_t n);
pcrs_buf pcrs_cgi_unescape(const uint8_t *s, size_t n);
pcrs_buf pcrs_cgi_escape_html(const uint8_t *s, size_t n);
pcrs_buf pcrs_cgi_unescape_html(const uint8_t *s, size_t n);
pcrs_buf pcrs_cgi_escape_javascript(const uint8_t *s, size_t n);
bool     pcrs_cgi_is_safe_local_path(const uint8_t *s, size_t n);

/* str (core/common/str.cpp) */
bool     pcrs_str_validate_utf8(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_valid_utf8(const uint8_t *s, size_t n);
int      pcrs_str_truncate_utf8(const uint8_t *s, size_t n, size_t limit, pcrs_buf *out);
int      pcrs_str_codepoint_to_utf8(uint32_t codepoint, pcrs_buf *out);
pcrs_buf pcrs_str_inspect(const uint8_t *s, size_t n);
int      pcrs_str_json_inspect(const uint8_t *s, size_t n, pcrs_buf *out);
bool     pcrs_str_is_http_url(const uint8_t *s, size_t n);

/* strutil (core/common/str.cpp の一部) */
typedef struct pcrs_vec {
    pcrs_buf joined;   /* 全部の要素を連結したバイト列 */
    size_t *lens;      /* 各要素の長さ (count 個)。count が 0 なら NULL でもよい */
    size_t count;
} pcrs_vec;

void pcrs_vec_free(pcrs_vec v);

pcrs_buf pcrs_str_hexdump(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_upcase(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_downcase(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_capitalize(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_group_digits(const uint8_t *s, size_t sn, const uint8_t *sep, size_t sepn);
bool     pcrs_str_contains(const uint8_t *s, size_t sn, const uint8_t *t, size_t tn);
bool     pcrs_str_has_prefix(const uint8_t *s, size_t sn, const uint8_t *t, size_t tn);
bool     pcrs_str_has_suffix(const uint8_t *s, size_t sn, const uint8_t *t, size_t tn);
pcrs_buf pcrs_str_replace_prefix(const uint8_t *s, size_t sn, const uint8_t *prefix, size_t pn,
                                  const uint8_t *repl, size_t rn);
pcrs_buf pcrs_str_replace_suffix(const uint8_t *s, size_t sn, const uint8_t *suffix, size_t fn_,
                                  const uint8_t *repl, size_t rn);
pcrs_buf pcrs_str_ascii_dump(const uint8_t *s, size_t n, const uint8_t *repl, size_t rn);
pcrs_buf pcrs_str_extension_without_dot(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_rstrip(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_strip(const uint8_t *s, size_t n);
pcrs_buf pcrs_str_escapeshellarg_unix(const uint8_t *s, size_t n);
pcrs_vec pcrs_str_split(const uint8_t *s, size_t sn, const uint8_t *sep, size_t sepn);
int      pcrs_str_split_limit(const uint8_t *s, size_t sn, const uint8_t *sep, size_t sepn,
                               int limit, pcrs_vec *out);
pcrs_buf pcrs_str_join(const uint8_t *delim, size_t dn, const uint8_t *parts_joined,
                        size_t parts_joined_len, const size_t *parts_lens, size_t parts_count);
pcrs_vec pcrs_str_to_lines(const uint8_t *text, size_t n);
int      pcrs_str_indent_tab(const uint8_t *text, size_t tn, int n, pcrs_buf *out);
int      pcrs_str_count(const uint8_t *h, size_t hn, const uint8_t *nd, size_t ndn, int32_t *out);
/* error_kind: 0=閉じていない '、1=閉じていない "、2=末尾の \\ */
int      pcrs_str_shellwords(const uint8_t *s, size_t n, pcrs_vec *out, int *error_kind);

/* md5 (core/common/md5.cpp) */
pcrs_buf pcrs_md5_hexdigest(const uint8_t *s, size_t n);

/* gnuid (core/common/gnuid.cpp の純粋な部分) */
void pcrs_gnuid_to_str(const uint8_t *id /* 16 bytes */, uint8_t *out /* 32 bytes */);
void pcrs_gnuid_from_str(const uint8_t *s, size_t n, uint8_t *out /* 16 bytes */);
void pcrs_gnuid_encode(uint8_t *id /* 16 bytes, in/out */, const uint8_t *ip /* 4 bytes or NULL */,
                        bool has_ip, const uint8_t *salt1, size_t salt1n,
                        const uint8_t *salt2, size_t salt2n, uint8_t salt3);

#ifdef __cplusplus
}
#endif

#endif
