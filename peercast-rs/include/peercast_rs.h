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

/* jis (core/common/jis.cpp) */
uint16_t pcrs_jis_sjis_to_unicode(uint16_t sjis);
uint16_t pcrs_jis_euc_to_unicode(uint16_t euc);

/* String (core/common/_string.cpp)。出力は data[MAX_LEN] に書く内容 (MAX_LEN - 1 バイト以下)。 */
pcrs_buf pcrs_string_ascii_to_esc(const uint8_t *s, size_t n, bool safe);
pcrs_buf pcrs_string_ascii_to_meta(const uint8_t *s, size_t n, bool safe);
pcrs_buf pcrs_string_unknown_to_unicode(const uint8_t *s, size_t n, bool safe);
pcrs_buf pcrs_string_esc_to_ascii(const uint8_t *s, size_t n);
pcrs_buf pcrs_string_base64_to_ascii(const uint8_t *s, size_t n);
pcrs_buf pcrs_string_from_string(const uint8_t *s, size_t n);
pcrs_buf pcrs_string_unquote(const uint8_t *s, size_t n);
pcrs_buf pcrs_string_from_stopwatch(uint32_t t);
int      pcrs_base64_word_to_chars(const uint8_t *word /* 4 bytes */, uint8_t *out /* 3 bytes */);

/* http (core/common/http.cpp の行の解析) と cgi::parseHttpDate */
int32_t  pcrs_http_parse_status_line(const uint8_t *s, size_t n, size_t *cut);
bool     pcrs_http_parse_header_line(const uint8_t *s, size_t n, size_t *arg_offset,
                                     pcrs_buf *name, pcrs_buf *value);
bool     pcrs_http_parse_basic_auth(const uint8_t *s, size_t n, pcrs_buf *user, pcrs_buf *pass);
bool     pcrs_http_is_cross_origin_request(const uint8_t *site, size_t site_n,
                                           const uint8_t *origin, size_t origin_n,
                                           const uint8_t *host, size_t host_n);
bool     pcrs_http_is_loopback_host_header(const uint8_t *s, size_t n);
int64_t  pcrs_cgi_parse_http_date(const uint8_t *s, size_t n);

/*
 * C++ の Stream を読むコールバック。どれも成功で 0、C++ の例外で中断したら -1 を返す。
 * コールバックから例外を外に出してはいけない (Rust を通り抜けられない)。例外は ctx に
 * 保存しておき、Rust の関数から戻ったあとで投げ直す。
 */
typedef struct pcrs_reader {
    void *ctx;
    int (*read_char)(void *ctx, uint8_t *out);                      /* Stream::readChar */
    int (*read_exact)(void *ctx, uint8_t *buf, size_t n);           /* Stream::read(int) */
    int (*read_some)(void *ctx, uint8_t *buf, size_t n, size_t *got); /* Stream::read(void*, int) */
    int (*eof)(void *ctx, bool *out);                               /* Stream::eof */
} pcrs_reader;

/* amf0 (core/common/amf0.cpp)。読んだ値をコールバックで通知する。コールバックも例外を外に出さない。 */
typedef struct pcrs_amf0_builder {
    void *ctx;
    void (*number)(void *ctx, double v);
    void (*string)(void *ctx, const uint8_t *s, size_t n);
    void (*boolean)(void *ctx, bool b);
    void (*null)(void *ctx);
    void (*date)(void *ctx, double unix_time, uint16_t timezone);
    void (*begin_object)(void *ctx, int kind /* 0: object, 1: ECMA array */);
    void (*key)(void *ctx, const uint8_t *s, size_t n);
    void (*end_object)(void *ctx);
    void (*begin_strict_array)(void *ctx);
    void (*end_strict_array)(void *ctx);
} pcrs_amf0_builder;

/* 0 成功、1 読み出しの中断、2 深すぎる、3 値が多すぎる、4 不明な型 (*unknown_type に型) */
int pcrs_amf0_read_value(const pcrs_reader *r, const pcrs_amf0_builder *b, int8_t *unknown_type);
int pcrs_amf0_read_object(const pcrs_reader *r, const pcrs_amf0_builder *b, int8_t *unknown_type);
/* 0 成功、-1 読み出しの中断 */
int pcrs_amf0_read_bool(const pcrs_reader *r, bool *out);
int pcrs_amf0_read_int32(const pcrs_reader *r, int32_t *out);
int pcrs_amf0_read_int16(const pcrs_reader *r, int16_t *out);
int pcrs_amf0_read_double(const pcrs_reader *r, double *out);
int pcrs_amf0_read_string(const pcrs_reader *r, pcrs_buf *out);

/* dechunker (core/common/dechunker.cpp)。*data は常に書かれる (空のこともある)。
 * 0 なし、1 読み出しの中断、2 "Protocol error"、3 "Chunk size too large"、
 * 4 最後のチャンク ("Closed on read")、5 "Premature end" */
int pcrs_dechunk_next(const pcrs_reader *r, size_t max_chunk_size, pcrs_buf *data);

/* xml (core/common/xml.cpp)。要素をコールバックで通知する。どれも成功で 0、例外で中断したら -1。 */
typedef struct pcrs_xml_builder {
    void *ctx;
    int (*content)(void *ctx, const uint8_t *s, size_t n);
    int (*start_tag)(void *ctx, const uint8_t *s, size_t n, bool single);
    int (*end_tag)(void *ctx);
} pcrs_xml_builder;

/* 0 成功、1 読み出しの中断、2 通知先の中断、3 "Tag too long"、4 "Content too big"、
 * 5 "Not XML document"、6 "Unexpected end tag" */
int pcrs_xml_read(const pcrs_reader *r, const pcrs_xml_builder *b);
/* 0 成功、1 "Too many attributes"、2 "Bad tag value"。positions は 2 * (n + 1) 個書けること */
int pcrs_xml_parse_attributes(const uint8_t *s, size_t n, pcrs_buf *data, size_t *positions, size_t *count);
/* 0 成功、-1 "Too much binary data" */
int pcrs_xml_binary_content(const uint8_t *s, size_t n, size_t size, pcrs_buf *out);

#ifdef __cplusplus
}
#endif

#endif
