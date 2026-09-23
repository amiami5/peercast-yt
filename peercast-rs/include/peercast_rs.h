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

/* URL (core/common/LUrlParser.cpp, url.cpp) */
/* 0 成功 (*out に scheme, host, port, path, query, fragment, user_name, password の 8 要素)、
 * 失敗なら LUrlParserError の値 (2〜5) */
int pcrs_url_parse(const uint8_t *s, size_t n, pcrs_vec *out);
/* 1〜65535 ならその値、そうでなければ 0 */
int pcrs_url_port_number(const uint8_t *s, size_t n);
/* ChanInfo::PROTOCOL の値を返し、読み飛ばす長さを *skip に書く */
int pcrs_url_source_protocol(const uint8_t *s, size_t n, size_t *skip);

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

/* メディアコンテナの解析 (core/common の flv, mkv, ogg, mp3, mp4)。
 * 解析器は pcrs_media_host を通してだけ、入力の Stream とチャンネル (Channel) を触る。
 * int を返すコールバックは、成功で 0、C++ の例外で中断したら -1 (例外は ctx に保存しておき、
 * Rust の関数から戻ったあとで投げ直す)。ほかのコールバックは例外を投げないこと。 */
enum {
    PCRS_MEDIA_MP3 = 1,
    PCRS_MEDIA_FLV = 2,
    PCRS_MEDIA_OGG = 3,
    PCRS_MEDIA_MKV = 4,
    PCRS_MEDIA_MP4 = 5
};

/* pcrs_media_host.head の kind */
enum {
    /* FLV, MP4: rawData.init()、streamIndex++ のあと、data を headPack に入れて位置 0 で送り、
     * streamPos をその長さにする */
    PCRS_HEAD_NEW_STREAM = 0,
    /* MKV: streamIndex++、rawData.init()、streamPos = 0 のあと、data の新しいパケットを
     * headPack に代入してから送り、streamPos を進める */
    PCRS_HEAD_MKV = 1,
    /* OGG: head_append で溜めた headPack を現在の streamPos の位置で送る
     * (startTime を今の時刻にし、streamPos を進める)。data は使わない */
    PCRS_HEAD_OGG = 2
};

/* ログの重要度 */
enum { PCRS_LOG_TRACE = 0, PCRS_LOG_DEBUG = 1, PCRS_LOG_INFO = 2, PCRS_LOG_WARN = 3, PCRS_LOG_ERROR = 4 };

typedef struct pcrs_track_field {
    const uint8_t *ptr;
    size_t len;
    bool present;               /* false なら値なし (ptr は NULL) */
} pcrs_track_field;

/* OGG Vorbis のコメントから取った曲の情報 (ChanInfo::track) */
typedef struct pcrs_track {
    pcrs_track_field artist, title, genre, contact, album;
} pcrs_track;

typedef struct pcrs_media_host {
    void *ctx;
    const pcrs_reader *reader;                                      /* 入力の Stream */
    int (*ready)(void *ctx, bool *out);                             /* in.readReady() */
    /* in.stat.bytesInPerSecAvg() / 1000 * 8 が info.bitrate を超えていれば updateInfo で更新 */
    int (*raise_bitrate)(void *ctx);
    /* T_DATA のパケットを streamPos の位置で newPacket し、streamPos を進める。
     * read_delay なら newPacket のあとに checkReadDelay(len) を呼ぶ */
    int (*packet)(void *ctx, const uint8_t *data, size_t len, bool cont, bool read_delay);
    int (*head)(void *ctx, int kind, const uint8_t *data, size_t len); /* PCRS_HEAD_* */
    uint32_t (*head_len)(void *ctx);                                /* headPack.len */
    void (*head_clear)(void *ctx);                                  /* headPack.len = 0 */
    /* headPack の後ろに付け足す (呼ぶ側が MAX_DATALEN 未満に収まることを確かめる) */
    int (*head_append)(void *ctx, const uint8_t *data, size_t len);
    int (*set_bitrate)(void *ctx, int32_t bitrate);                 /* info.bitrate を updateInfo で */
    /* info.bitrate を直接書き換え、ogm なら info.contentType を T_OGM にする */
    void (*ogg_set_info)(void *ctx, int32_t bitrate, bool ogm);
    /* info.track を空にしてから値を入れ (String::T_ASCII から T_UNICODE に変換)、updateInfo */
    int (*set_track)(void *ctx, const pcrs_track *track);
    int (*mp3_metadata)(void *ctx, const uint8_t *buf, size_t len); /* processMp3Metadata (NUL 終端) */
    int32_t (*icy_meta_interval)(void *ctx);
    bool (*read_delay)(void *ctx);
    double (*dtime)(void *ctx);                                     /* sys->getDTime() */
    uint32_t (*time)(void *ctx);                                    /* sys->getTime() */
    void (*sleep)(void *ctx, int32_t ms);                           /* sys->sleep() */
    void (*sleep_until)(void *ctx, double t);                       /* Channel::sleepUntil */
    void (*log)(void *ctx, int level, const uint8_t *msg, size_t len);
} pcrs_media_host;

typedef struct pcrs_media pcrs_media;

/* 解析器を作る (kind は PCRS_MEDIA_*、知らない値なら NULL)。pcrs_media_free で解放する */
pcrs_media *pcrs_media_new(int kind);
void pcrs_media_free(pcrs_media *p);
/* readHeader / readPacket。0 成功、1 コールバックの中断、
 * 2 エラー (StreamException のメッセージを *err に書く。受け取った側が pcrs_buf_free で返す) */
int pcrs_media_read_header(pcrs_media *p, const pcrs_media_host *host, pcrs_buf *err);
int pcrs_media_read_packet(pcrs_media *p, const pcrs_media_host *host, pcrs_buf *err);
/* テンプレート (core/common/template.cpp)。式とディレクティブの処理は Rust、スコープ (変数) と
 * 正規表現 (std::regex) は C++ に残り、pcrs_template_host のコールバックで使う。
 * 値の受け渡しには src/template/value.rs の形式 (型 1 バイト + 中身、リトルエンディアン) を使う。
 * int を返すコールバックは、成功で 0、C++ の例外で中断したら -1。 */
typedef struct pcrs_template_host {
    void *ctx;
    const pcrs_reader *reader;      /* テンプレートの Stream。ディレクティブを読まない呼び出しでは NULL */
    int (*position)(void *ctx, int32_t *out);                   /* Stream::getPosition */
    int (*seek)(void *ctx, int32_t pos);                        /* Stream::seekTo */
    int (*write)(void *ctx, const uint8_t *data, size_t len);   /* 出力 (outp) */
    /* Template::writeVariable。値は次のコールバックまで有効な C++ 側のバッファを指す */
    int (*lookup)(void *ctx, const uint8_t *name, size_t n, const uint8_t **value, size_t *value_len);
    void (*push_scope)(void *ctx);                              /* GenericScope を先頭に置く */
    void (*pop_scope)(void *ctx);
    bool (*front_is_generic)(void *ctx);
    int (*set_front)(void *ctx, const uint8_t *name, size_t n, const uint8_t *value, size_t value_len);
    int (*regex_check)(void *ctx, const uint8_t *pattern, size_t n);  /* Regexp を作る */
    int (*regex_match)(void *ctx, const uint8_t *pattern, size_t n, const uint8_t *subject, size_t m, bool *out);
    void (*selected_fragment)(void *ctx, const uint8_t **out, size_t *n);
    void (*current_fragment)(void *ctx, const uint8_t **out, size_t *n);
    void (*set_current_fragment)(void *ctx, const uint8_t *f, size_t n);
    void (*log_error)(void *ctx, const uint8_t *msg, size_t n);
} pcrs_template_host;

/* pcrs_template_call の op。( ) の中は引数と結果の値 */
enum {
    PCRS_TMPL_READ_TEMPLATE = 1,        /* (bool 出力あり) -> 数 (TMPL_*) */
    PCRS_TMPL_READ_CMD = 2,             /* (bool) -> 数 */
    PCRS_TMPL_READ_IF = 3,              /* (bool) -> null */
    PCRS_TMPL_READ_LOOP = 4,
    PCRS_TMPL_READ_FOREACH = 5,
    PCRS_TMPL_READ_LET = 6,
    PCRS_TMPL_READ_FRAGMENT = 7,
    PCRS_TMPL_READ_VARIABLE_VALUE = 8,  /* (bool) -> 表示する文字列の厳密配列 (表示しなければ空) */
    PCRS_TMPL_EVAL_STR = 9,             /* (文字列) -> 値 */
    PCRS_TMPL_EVAL = 10,                /* (式) -> 値 */
    PCRS_TMPL_EVAL_FORM = 11,           /* (式) -> 値 */
    PCRS_TMPL_EVAL_CONDITION = 12,      /* (文字列) -> bool */
    PCRS_TMPL_GET_INT = 13,             /* (名前) -> 数 */
    PCRS_TMPL_GET_BOOL = 14,            /* (名前) -> bool */
    PCRS_TMPL_GET_STRING = 15,          /* (名前) -> 文字列 */
    PCRS_TMPL_APPLY = 16,               /* ([lambda, [式...]]) -> 値 */
    /* 以下はホストを使わない (host は NULL でよい) */
    PCRS_TMPL_TOKENIZE = 17,            /* (文字列) -> [トークン...] */
    PCRS_TMPL_PARSE = 18,               /* ([トークン...]) -> [式, [残りのトークン...]] */
    PCRS_TMPL_PARSE_LET_SPEC = 19,      /* ([トークン...]) -> [[[名前, 式]...], [残り...]] */
    PCRS_TMPL_READ_STRING_LITERAL = 20, /* (文字列) -> [リテラル, 残り] */
    PCRS_TMPL_EVAL_STRING_LITERAL = 21  /* (文字列) -> 文字列 */
};

/* 0 成功 (*result に値)、1 コールバックの中断、2〜6 は例外 (*result にメッセージ):
 * 2 GeneralException、3 StreamException、4 std::runtime_error、5 std::out_of_range、
 * 6 std::invalid_argument。7 は呼び出し方の誤り。*result は常に書かれる (pcrs_buf_free で返す) */
int pcrs_template_call(int op, const pcrs_template_host *host, const uint8_t *arg, size_t arg_len, pcrs_buf *result);

/* 公開ディレクトリ (core/common/public.cpp の PublicController) */
pcrs_buf pcrs_public_format_uptime(uint32_t total_seconds);
pcrs_vec pcrs_public_acceptable_languages(const uint8_t *s, size_t n);
/* コンソールのコマンドの引数 (core/common/commands.cpp の parse_options)。入力の形は pcrs_str_join と同じ。
 * 0 成功 (*out に (名前, 値) を *num_options 組並べたあとに位置引数)、-1 知らないオプション
 * (*err に FormatException のメッセージ)、-2 引数の形が壊れている */
int pcrs_commands_parse_options(const uint8_t *args_joined, size_t args_joined_len, const size_t *args_lens, size_t args_count,
                                const uint8_t *names_joined, size_t names_joined_len, const size_t *names_lens, size_t names_count,
                                pcrs_vec *out, size_t *num_options, pcrs_buf *err);

/* FLVStream::readMetaData。onMetaData でビットレートがあれば 1 (*bitrate に書く)、なければ 0、
 * 形式が壊れていれば 2 (理由を *err に書く) */
int pcrs_flv_read_meta_data(const uint8_t *data, size_t n, int32_t *bitrate, pcrs_buf *err);

#ifdef __cplusplus
}
#endif

#endif
