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

#ifdef __cplusplus
}
#endif

#endif
