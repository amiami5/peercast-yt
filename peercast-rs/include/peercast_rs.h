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

/* PCP の受け取ったパケットの処理 (core/common/pcp.cpp の PCPStream::procAtom 以下と、
 * ChanInfo::readInfoAtoms / readTrackAtoms)。解析と判断は Rust、チャンネルやサーバーの状態
 * (chanMgr、servMgr、Channel) は C++ に残り、pcrs_pcp_host のコールバックで読み書きする */
typedef struct pcrs_pcp_ip {
    uint8_t kind;               /* 0 なし、4 (v4 を IP(unsigned int) に)、16 (v6 を in6_addr に) */
    uint32_t v4;
    uint8_t v6[16];
} pcrs_pcp_ip;

/* pcrs_pcp_hit の set のビット (立っているメンバーだけ ChanHit に入れる) */
enum {
    PCRS_PCP_HIT_NUML = 1 << 0, PCRS_PCP_HIT_NUMR = 1 << 1, PCRS_PCP_HIT_UPTIME = 1 << 2,
    PCRS_PCP_HIT_OLDPOS = 1 << 3, PCRS_PCP_HIT_NEWPOS = 1 << 4, PCRS_PCP_HIT_VERSION = 1 << 5,
    PCRS_PCP_HIT_VERSION_VP = 1 << 6, PCRS_PCP_HIT_VEX_PREFIX = 1 << 7, PCRS_PCP_HIT_VEX_NUMBER = 1 << 8,
    PCRS_PCP_HIT_FLAGS1 = 1 << 9, PCRS_PCP_HIT_SESSION_ID = 1 << 10, PCRS_PCP_HIT_UPHOST_PORT = 1 << 11,
    PCRS_PCP_HIT_UPHOST_HOPS = 1 << 12
};

/* readHostAtoms が読んだ ChanHit の値。rhost_ip と uphost_ip は kind が 0 でなければ入れる */
typedef struct pcrs_pcp_hit {
    uint32_t set;
    pcrs_pcp_ip rhost_ip[2];
    bool rhost_port_set[2];
    int32_t rhost_port[2];
    int32_t num_listeners, num_relays, up_time, oldest_pos, newest_pos, version, version_vp,
            version_ex_number, flags1, uphost_port, uphost_hops;
    uint8_t version_ex_prefix[2];
    uint8_t session_id[16];
    pcrs_pcp_ip uphost_ip;
    uint8_t chan_id[16];        /* 常に入れる */
    int32_t num_hops;           /* 常に入れる */
} pcrs_pcp_hit;

/* BroadcastState と PCPStream::nextRootPacket */
typedef struct pcrs_pcp_state {
    uint8_t chan_id[16];
    uint8_t bc_id[16];
    int32_t num_hops;
    bool for_me;
    uint32_t stream_pos;
    int32_t group;
    uint32_t next_root_packet;
} pcrs_pcp_state;

/* pcrs_pcp_host の event の op */
enum {
    PCRS_PCP_EV_ROUTE_ADD = 1,          /* routeList.add(data 16 バイト) */
    PCRS_PCP_EV_UPDATE_INTERVAL = 2,    /* chanMgr->setUpdateInterval(arg) */
    PCRS_PCP_EV_UPGRADE = 3,            /* servMgr->downloadURL に data を入れて NT_UPGRADE を通知 */
    PCRS_PCP_EV_TRACKER_UPDATE = 4,     /* chanMgr->broadcastTrackerUpdate(remoteID, true) */
    PCRS_PCP_EV_ROOT_MESSAGE = 5,       /* ルートからのメッセージ data (servMgr->rootMsg と違えば入れ替えて通知) */
    PCRS_PCP_EV_CHAN_BEGIN = 6,         /* readChanAtoms の始め。data (16 バイト) のチャンネルとヒットリストを探す */
    PCRS_PCP_EV_CHAN_INFO_STRING = 7,   /* newInfo の文字列 arg (PCRS_PCP_INFO_*) に readString のとおり data を写す */
    PCRS_PCP_EV_CHAN_INFO_BITRATE = 8,  /* newInfo.bitrate = arg */
    PCRS_PCP_EV_CHAN_BCID = 9,          /* newInfo.bcID = data (16 バイト) */
    PCRS_PCP_EV_CHAN_ID = 10,           /* newInfo.id = data (16 バイト) にして、チャンネルとヒットリストを探し直す */
    PCRS_PCP_EV_CHAN_END = 11           /* readChanAtoms の終わり (ヒットリストの更新、チャンネルのログ、updateInfo) */
};

/* PCRS_PCP_EV_CHAN_INFO_STRING の arg */
enum {
    PCRS_PCP_INFO_NAME = 0, PCRS_PCP_INFO_GENRE = 1, PCRS_PCP_INFO_URL = 2, PCRS_PCP_INFO_DESC = 3,
    PCRS_PCP_INFO_COMMENT = 4, PCRS_PCP_INFO_TYPE = 5, PCRS_PCP_INFO_STREAMTYPE = 6, PCRS_PCP_INFO_STREAMEXT = 7,
    PCRS_PCP_TRACK_TITLE = 8, PCRS_PCP_TRACK_CREATOR = 9, PCRS_PCP_TRACK_URL = 10, PCRS_PCP_TRACK_ALBUM = 11
};

/* pcrs_pcp_host の broadcast の target */
enum {
    PCRS_PCP_BCAST_UP = 0,      /* chanMgr->broadcastPacketUp */
    PCRS_PCP_BCAST_COUT = 1,    /* servMgr->broadcastPacket(..., Servent::T_COUT) */
    PCRS_PCP_BCAST_CIN = 2,     /* 同 T_CIN */
    PCRS_PCP_BCAST_RELAY = 3    /* 同 T_RELAY */
};

/* int を返すコールバックは、成功で 0、C++ の例外で中断したら -1 */
typedef struct pcrs_pcp_host {
    void *ctx;
    void (*session_id)(void *ctx, uint8_t *out16);                  /* servMgr->sessionID */
    bool (*is_root)(void *ctx);                                     /* servMgr->isRoot */
    uint32_t (*time)(void *ctx);                                    /* sys->getTime() */
    void (*log)(void *ctx, int level, const uint8_t *msg, size_t len); /* 0 DEBUG、1 INFO、2 ERROR */
    int (*event)(void *ctx, int op, int32_t arg, const uint8_t *data, size_t len); /* PCRS_PCP_EV_* */
    int (*hit)(void *ctx, const pcrs_pcp_hit *hit, bool add);      /* add なら chanMgr->addHit、でなければ delHit */
    /* 自分宛ての push。ip->kind が 0 なら ip、port_set が false なら port は Host の初期値のまま */
    int (*push)(void *ctx, const pcrs_pcp_ip *ip, bool port_set, int32_t port, const uint8_t *chan_id16);
    bool (*chan_has_channel)(void *ctx);                            /* readChanAtoms の ch が NULL でないか */
    /* readPktAtoms のチャンネル側 (rawData への書き込みなど)。kind は ChanPacket::TYPE */
    int (*chan_packet)(void *ctx, int32_t kind, uint32_t pos, bool cont, const uint8_t *data, size_t len);
    int (*broadcast)(void *ctx, int target, const uint8_t *pack, size_t len, const uint8_t *chan_id16, const uint8_t *dest_id16);
} pcrs_pcp_host;

/* PCPStream::readPacket の、受け取ったパケットの処理 (mem.rewind() から procAtom まで)。buf は
 * ChanPacket::data 全体 (len バイト。helo への返事をここに書く)。0 成功 (*result に procAtom の値)、
 * 1 コールバックの中断、2 StreamException (*err にメッセージ。pcrs_buf_free で返す)。
 * *st は途中で中断しても書き戻す */
int pcrs_pcp_proc_packet(const pcrs_pcp_host *host, uint8_t *buf, size_t len, pcrs_pcp_state *st,
                         int32_t *result, pcrs_buf *err);

/* PCP のハンドシェイクで受け取る helo / oleh (servent.cpp の handshakeIncomingPCP、
 * handshakeOutgoingPCP、pingHost) と PCPStream::readVersion。Stream から pcrs_reader で読む。
 * 返事を書くことと、読んだ値を使った処理は C++ に残る */
enum { PCRS_PCP_KIND_HELO = 0, PCRS_PCP_KIND_OLEH = 1, PCRS_PCP_KIND_PING = 2 };
/* pcrs_pcp_hello の set のビット */
enum {
    PCRS_PCP_HELLO_VERSION = 1, PCRS_PCP_HELLO_DISABLE = 2, PCRS_PCP_HELLO_SESSION_ID = 4,
    PCRS_PCP_HELLO_BCID = 8, PCRS_PCP_HELLO_OSTYPE = 16, PCRS_PCP_HELLO_PORT = 32, PCRS_PCP_HELLO_PING = 64
};
typedef struct pcrs_pcp_hello {
    bool header_ok;             /* 最初の atom の見出しを読み、ID が期待したもの (helo か oleh) だった */
    bool is_unexpected;         /* 最初の atom の ID が違った (unexpected にその ID) */
    uint8_t unexpected[4];
    bool has_agent;             /* agent.set(arg) の値 (NUL の手前まで、agent_len バイト) */
    uint8_t agent[64];
    size_t agent_len;
    uint32_t set;               /* PCRS_PCP_HELLO_* */
    int32_t version, disable, os_type, port, ping;  /* port と ping は readShort の値 */
    uint8_t session_id[16];     /* rid (ping では sid) に書いたもの。ping では、呼ぶ前に sid の値を入れておく */
    uint8_t bcid[16];
    pcrs_pcp_ip remote_ip;      /* oleh の rip。kind が 0 ならなし */
} pcrs_pcp_hello;
/* 0 成功、1 読み出しの中断 (C++ の例外)、2 StreamException (*err にメッセージ。pcrs_buf_free で返す)。
 * どの場合も、それまでに読んだ値を *out に書く (*out は呼ぶ側が 0 で埋めておく。ping の session_id は上記)。
 * log は読み飛ばした atom のログ ("PCP handshake skip: ...") */
int pcrs_pcp_read_hello(const pcrs_reader *r, int kind, const uint8_t *my_sid16, void *log_ctx,
                        void (*log)(void *ctx, const uint8_t *msg, size_t len), pcrs_pcp_hello *out, pcrs_buf *err);
/* PCPStream::readVersion。返り値は pcrs_pcp_read_hello と同じで、成功なら版を *ver に書く */
int pcrs_pcp_read_version(const pcrs_reader *r, int32_t *ver, pcrs_buf *err);

/* チャンネルのパケットのバッファ (core/common/chanpacket.cpp の ChanPacketBuffer)。パケットと位置は
 * C++ のクラスのメンバーのまま、pcrs_cpb でそれを指して渡す。ロックと readPacket の待ち合わせは
 * C++ 側 */
typedef struct pcrs_chan_packet {  /* ChanPacket と同じ並び */
    int32_t type;
    uint32_t len;
    uint32_t pos;
    uint32_t sync;
    bool cont;
    uint8_t data[16384];
} pcrs_chan_packet;
typedef struct pcrs_cpb {
    pcrs_chan_packet *packets;     /* 64 個 */
    uint32_t *last_pos, *first_pos, *safe_pos, *read_pos, *write_pos, *accept, *last_write_time;
} pcrs_cpb;
/* pcrs_cpb_pos の op */
enum {
    PCRS_CPB_LATEST_POS = 0, PCRS_CPB_OLDEST_POS = 1, PCRS_CPB_FIND_OLDEST_POS = 2, PCRS_CPB_STREAM_POS = 3,
    PCRS_CPB_STREAM_POS_END = 4, PCRS_CPB_LATEST_NONCONT_POS = 5, PCRS_CPB_OLDEST_NONCONT_POS = 6
};
void pcrs_cpb_init(const pcrs_cpb *b);
bool pcrs_cpb_write_packet(const pcrs_cpb *b, pcrs_chan_packet *pack, bool update_read_pos, uint32_t now);
bool pcrs_cpb_will_skip(const pcrs_cpb *b);
/* 0 読める、1 遅れすぎ (check_behind のときだけ)、2 まだない */
int pcrs_cpb_read_state(const pcrs_cpb *b, bool check_behind);
void pcrs_cpb_take(const pcrs_cpb *b, pcrs_chan_packet *pack);
bool pcrs_cpb_find_packet(const pcrs_cpb *b, uint32_t spos, pcrs_chan_packet *pack);
uint32_t pcrs_cpb_pos(const pcrs_cpb *b, int op, uint32_t arg);
/* 長さを lens (64 個) に書き、その数を返す */
size_t pcrs_cpb_statistics(const pcrs_cpb *b, uint32_t *lens, int *continuations, int *non_continuations);
int pcrs_cpb_copy_from(const pcrs_cpb *b, const pcrs_cpb *src, uint32_t req_pos);

#ifdef __cplusplus
}
#endif

#endif
