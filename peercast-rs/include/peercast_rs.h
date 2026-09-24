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

/* イエローページのチャンネル一覧 (core/common/chandir.cpp) */
typedef struct pcrs_bytes {    /* 借りたバイト列 */
    const uint8_t *ptr;
    size_t len;
} pcrs_bytes;
typedef struct pcrs_chan_entry {  /* ChannelEntry (feedUrl を除く)。中身はコールバックの間だけ有効 */
    pcrs_bytes name, tip, url, genre, desc, content_type, track_artist, track_album, track_name,
               track_contact, encoded_name, uptime, status, comment;
    uint8_t id[16];
    int32_t num_directs, num_relays, bitrate, direct;
} pcrs_chan_entry;
typedef void (*pcrs_chan_entry_fn)(void *ctx, const pcrs_chan_entry *e);
/* textToChannelEntries。正しい行は on_entry、欄の数が 19 でない行は on_error に行番号を渡す。
 * コールバックは例外を投げないこと */
void pcrs_chandir_parse(const uint8_t *text, size_t n, void *ctx, pcrs_chan_entry_fn on_entry,
                        void (*on_error)(void *ctx, int32_t lineno));
/* ChannelEntry(fields, feedUrl)。欄が 19 個に足りなければ -1 */
int pcrs_chandir_entry(const pcrs_bytes *fields, size_t count, void *ctx, pcrs_chan_entry_fn on_entry);
/* chatUrl (kind 0) と statsUrl (kind 1) */
pcrs_buf pcrs_chandir_side_url(const uint8_t *feed, size_t fn_, const uint8_t *name, size_t nn, int kind);
pcrs_buf pcrs_chandir_directory_url(const uint8_t *s, size_t n);
pcrs_buf pcrs_chandir_format_time(uint32_t diff);

/* チャンネルの情報 (core/common/chaninfo.cpp の ChanInfo と TrackInfo)。文字列は String の中身
 * (NUL の手前まで使う)。表の関数は、C++ 版と同じく静的な文字列を返す */
const char *pcrs_chaninfo_type_ext(const uint8_t *s, size_t n);
const char *pcrs_chaninfo_mime_type(const uint8_t *s, size_t n);
const char *pcrs_chaninfo_type_from_mime(const uint8_t *s, size_t n);
const char *pcrs_chaninfo_type_from_str(const uint8_t *s, size_t n);
const char *pcrs_chaninfo_playlist_ext(const uint8_t *s, size_t n);
const char *pcrs_chaninfo_protocol_str(int p);
int pcrs_chaninfo_protocol_from_str(const uint8_t *s, size_t n);
typedef struct pcrs_chan_info {
    pcrs_bytes name, content_type, mime, ext, desc, genre, url, comment;
    pcrs_bytes track_contact, track_title, track_artist, track_album, track_genre;
    uint8_t id[16], bcid[16];
    int32_t bitrate, status;
} pcrs_chan_info;
/* ChanInfo::update で写す欄 (ビット) */
enum {
    PCRS_CI_BITRATE = 1 << 0, PCRS_CI_CONTENT_TYPE = 1 << 1, PCRS_CI_MIME = 1 << 2, PCRS_CI_EXT = 1 << 3,
    PCRS_CI_DESC = 1 << 4, PCRS_CI_NAME = 1 << 5, PCRS_CI_COMMENT = 1 << 6, PCRS_CI_GENRE = 1 << 7,
    PCRS_CI_URL = 1 << 8, PCRS_CI_TRACK_CONTACT = 1 << 9, PCRS_CI_TRACK_TITLE = 1 << 10,
    PCRS_CI_TRACK_ARTIST = 1 << 11, PCRS_CI_TRACK_ALBUM = 1 << 12, PCRS_CI_TRACK_GENRE = 1 << 13
};
pcrs_buf pcrs_chaninfo_type_string_long(const pcrs_chan_info *info);
/* match(ChanInfo&)。name_id_only なら matchNameID */
bool pcrs_chaninfo_match(const pcrs_chan_info *me, const pcrs_chan_info *q, bool name_id_only);
/* ChanInfo::update。0 使わない、1 配信者の鍵が違う、2 *copy の欄を写す、3 さらに配信者の鍵も写す */
int pcrs_chaninfo_update(const pcrs_chan_info *me, const pcrs_chan_info *info, uint32_t *copy);
/* TrackInfo::update で写す欄 (track_* だけ使う) */
uint32_t pcrs_trackinfo_update(const pcrs_chan_info *me, const pcrs_chan_info *info);
/* writeInfoAtoms (track が false) と writeTrackAtoms (true) のバイト列 */
pcrs_buf pcrs_chaninfo_write_atoms(const pcrs_chan_info *info, bool track);

/* チャンネルを中継しているホスト (core/common/chanhit.cpp の ChanHit と ChanHitList)。一覧は
 * 連結リストの並びどおりの配列で渡す */
typedef struct pcrs_host {
    uint8_t ip[16];                /* IP::serialize() */
    uint16_t port;
} pcrs_host;
typedef struct pcrs_hit {
    pcrs_host host, rhost[2], uphost;
    uint32_t num_listeners, num_relays, num_hops, time, up_time, last_contact, version, oldest_pos,
             newest_pos, uphost_hops, version_vp, version_ex_number;
    uint8_t session_id[16];
    uint8_t version_ex_prefix[2];
    bool firewalled, tracker, recv, dead, direct, relay, cin;
} pcrs_hit;
typedef struct pcrs_hit_search {
    pcrs_host match_host;
    uint32_t wait_delay;
    bool use_firewalled, trackers_only, use_busy_relays, use_busy_controls;
    uint8_t exclude_id[16];
    int32_t num_results;
} pcrs_hit_search;
/* pcrs_hits_count の op */
enum {
    PCRS_HITS_NUM_HITS = 0, PCRS_HITS_NUM_LISTENERS = 1, PCRS_HITS_NUM_RELAYS = 2, PCRS_HITS_NUM_TRACKERS = 3,
    PCRS_HITS_NUM_FIREWALLED = 4, PCRS_HITS_CLOSEST = 5, PCRS_HITS_FURTHEST = 6, PCRS_HITS_NEWEST = 7,
    PCRS_HITS_TOTAL_LISTENERS = 8, PCRS_HITS_TOTAL_RELAYS = 9, PCRS_HITS_TOTAL_FIREWALLED = 10
};
pcrs_buf pcrs_hit_write_atoms(const pcrs_hit *h, const uint8_t *chan_id);
pcrs_buf pcrs_hit_version_string(const pcrs_hit *h);
int pcrs_hit_color(const pcrs_hit *h);          /* 0 red、1 purple、2 blue、3 green */
bool pcrs_hit_can_giv(const pcrs_hit *h);
/* 返り値は C++ 版の返り値 (int か unsigned int) のビット列 */
uint32_t pcrs_hits_count(const pcrs_hit *hits, size_t n, int op);
/* pickHits。選んだ番号 (なければ -1)。LAN 側のアドレスを使うなら *lan を true に */
int pcrs_hits_pick(const pcrs_hit *hits, size_t n, const pcrs_hit_search *s, uint32_t ctime, bool *lan);
/* clearDeadHits。消すものの del を true にし、残る数を返す */
int pcrs_hits_clear_dead(const pcrs_hit *hits, size_t n, uint32_t timeout, bool clear_trackers, uint32_t ctime, bool *del);
/* deadHit と delHit の対象 */
void pcrs_hits_same_hosts(const pcrs_hit *hits, size_t n, const pcrs_hit *h, bool *out);
/* addHit。-2 自分のホスト、-1 del のものを消して先頭に加える、0 以上ならその番号を書き換える */
int pcrs_hits_add(const pcrs_hit *hits, size_t n, const pcrs_hit *h, const uint8_t *my_sid, bool *del);

/* ---- hostgraph (core/common/hostgraph.cpp の HostGraph のコンストラクター) ---- */
typedef struct pcrs_graph_node {
    pcrs_host rhost[2], uphost;
} pcrs_graph_node;
/* nodes は自分、続いてリストの順。ID (rhost[0], rhost[1]) の順に、採った番号 (同じ ID なら
   最後のもの) を index に、親の位置 (根なら -1) を parent に書き、その数を返す */
size_t pcrs_hostgraph_build(const pcrs_graph_node *nodes, size_t n, size_t *index, ptrdiff_t *parent);

/* ---- uptest (core/common/uptest.cpp の通信しない部分) ---- */
/* readInfo。0 なら out に UptestInfo の 14 個の欄を順に NUL で区切って入れる。失敗なら 3〜6
   (pcrs_xml_read と同じ)、7 "Too many attributes"、8 "Bad tag value"、9 ノードか属性がない */
int pcrs_uptest_read_info(const uint8_t *body, size_t n, pcrs_buf *out);
pcrs_buf pcrs_uptest_post_url(const uint8_t *addr, size_t addr_len, const uint8_t *port, size_t port_len,
                              const uint8_t *object, size_t object_len);
bool pcrs_uptest_is_ready(int status, uint32_t last_tried_at, uint32_t now);
const char *pcrs_uptest_text_status(int status);  /* 知らない値なら NULL */
/* addURL の判断。加えてよければ NULL、だめなら理由 (静的な文字列) */
const char *pcrs_uptest_check_add_url(bool valid, const uint8_t *scheme, size_t scheme_len,
                                      const uint8_t *url, size_t url_len, const pcrs_bytes *existing, size_t count);

/* ---- channel (core/common/channel.cpp と chanmgr.cpp の、スレッドやソケットに触らない部分) ---- */
/* processMp3Metadata。StreamTitle があれば 1、StreamUrl があれば 2 を足して返し、値 (引用符は
   付いたまま) の位置と長さを書く */
int pcrs_channel_mp3_metadata(const uint8_t *s, size_t n, size_t *title_pos, size_t *title_len,
                              size_t *url_pos, size_t *url_len);
/* writeTrackerUpdateAtom と、updateInfo で中継先へ送る atom */
pcrs_buf pcrs_channel_tracker_update_atom(const pcrs_chan_info *info, const pcrs_hit *hit,
                                          const uint8_t *session_id, const uint8_t *broadcast_id);
pcrs_buf pcrs_channel_info_update_atom(const pcrs_chan_info *info, const uint8_t *session_id);
pcrs_buf pcrs_channel_hex_dump(const uint8_t *s, size_t n);   /* renderHexDump */
pcrs_buf pcrs_channel_buffer_string(double byterate, uint32_t now, uint32_t last_write_time,
                                    const uint32_t *lens, size_t n, int cont, int non_cont);
/* checkReadDelay。眠るなら true で、時間 (ミリ秒) を ms に書く */
bool pcrs_channel_read_delay(bool read_delay, uint32_t len, int32_t bitrate, uint32_t *ms);
pcrs_buf pcrs_chanmgr_auth_token(const uint8_t *broadcast_id, const uint8_t *id);
/* closeOldestIdle で止めるチャンネルの番号。なければ -1 */
ptrdiff_t pcrs_chanmgr_oldest_idle(const bool *idle, const uint32_t *last_idle_time, size_t n);

/* ---- jrpc (core/common/jrpc.cpp の JrpcApi) ---- */
/* 要求の解釈と結果の JSON の組み立ては Rust (src/jrpc.rs、JSON は src/json)。サーバーの状態は
   pcrs_jrpc_host の call で触る。call は op ごとに下の値を受け取り、結果を pcrs_jrpc_put_* で渡す。
   中身の見えない受け取り口 (pcrs_jrpc_sink) と args は、その call の間だけ有効 */
typedef struct pcrs_jrpc_sink pcrs_jrpc_sink;

typedef struct pcrs_jrpc_args {
    uint8_t id[16];
    int32_t i, j;
    pcrs_bytes a, b;
    const pcrs_bytes *fields;   /* UPDATE_INFO は 10 個、FETCH は url, name, desc, genre, contact, type の 6 個 */
} pcrs_jrpc_args;

typedef struct pcrs_jrpc_channel {
    pcrs_chan_info info;
    int32_t status;
    pcrs_bytes source_url;
    pcrs_bytes source_host;     /* sourceHost.host.str() */
    uint32_t uptime;            /* info.getUptime() */
    int32_t local_relays, local_directs, total_relays, total_directs;
    bool is_broadcasting, is_full, is_receiving;
    int32_t ip_version;
    bool has_sock;
    pcrs_bytes sock_host;       /* sock->host.str() */
    int32_t source_rate;        /* sourceData ? getSourceRate() : 0 */
    int32_t src_protocol;
    uint32_t stream_pos;
} pcrs_jrpc_channel;

typedef struct pcrs_jrpc_servent {
    int32_t index;
    pcrs_bytes type, status;    /* getTypeStr()、getStatusStr() */
    uint32_t send_rate, recv_rate;
    int32_t protocol;           /* outputProtocol */
    pcrs_bytes agent;
    bool has_sock;
    pcrs_bytes sock_host;
} pcrs_jrpc_servent;

typedef struct pcrs_jrpc_yp {   /* ChannelEntry */
    pcrs_bytes feed_url, name;
    uint8_t id[16];
    pcrs_bytes tip, url, genre, desc, comment;
    int32_t bitrate;
    pcrs_bytes content_type, track_name, track_album, track_artist, track_contact;
    int32_t num_directs, num_relays;
} pcrs_jrpc_yp;

typedef struct pcrs_jrpc_found {    /* ChanHitList (getChannelsFound) */
    pcrs_chan_info info;
    uint32_t uptime, skips, age;
    uint8_t bcflags;
    int32_t hosts, listeners, relays, firewalled, closest, furthest;
    uint32_t newest;                /* sys->getTime() - newestHit() */
} pcrs_jrpc_found;

typedef struct pcrs_jrpc_found_hit {
    pcrs_bytes ip;                  /* host.str() */
    uint32_t hops, listeners, relays, uptime;
    bool push, relay, direct, cin, stable;
    uint32_t version;
    uint32_t update;                /* sys->getTime() - time */
    bool tracker;
} pcrs_jrpc_found_hit;

/* call の op。( ) の中は使う引数、-> のあとは結果 */
enum {
    PCRS_JRPC_AGENT = 0,            /* -> bytes: PCX_AGENT */
    PCRS_JRPC_LOG_LINES = 1,        /* -> bytes: ログの各行 */
    PCRS_JRPC_CLEAR_LOG = 2,
    PCRS_JRPC_LOG_LEVEL = 3,        /* -> int */
    PCRS_JRPC_SET_LOG_LEVEL = 4,    /* (i) */
    PCRS_JRPC_FETCH = 5,            /* (fields, i: bitrate, j: IPv6 なら 1) -> 作れたら bytes: チャンネル ID (16 バイト) */
    PCRS_JRPC_CHANNELS = 6,         /* -> channel: chanMgr->channel の並び */
    PCRS_JRPC_FIND_CHANNEL = 7,     /* (id) -> あれば channel */
    PCRS_JRPC_SERVENTS = 8,         /* (id) -> servent: chanID が id のもの */
    PCRS_JRPC_STOP_CONNECTION = 9,  /* (id, i: connectionId) -> int: 止めたら 1 */
    PCRS_JRPC_RELAY_TREE = 10,      /* (id) -> int: 0 チャンネルなし、1 ヒットリストなし、2 あり。2 なら hit: 自分とヒットリスト */
    PCRS_JRPC_BUMP = 11,            /* (id) -> int: チャンネルがあれば 1 */
    PCRS_JRPC_PLAY = 12,            /* (id) */
    PCRS_JRPC_STOP_CHANNEL = 13,    /* (id) */
    PCRS_JRPC_ROOT_HOST = 14,       /* -> bytes */
    PCRS_JRPC_CLEAR_ROOT_HOST = 15,
    PCRS_JRPC_SETTINGS = 16,        /* -> int: maxRelays, maxRelaysPerChannel, maxDirect, maxBitrateOut */
    PCRS_JRPC_SET_SETTING = 17,     /* (i: 0〜3 は上の順、j: 値) */
    PCRS_JRPC_STATUS = 18,          /* -> int: uptime, firewall, port、bytes: globalIP, localIP */
    PCRS_JRPC_STATE = 19,           /* (i: 0 servMgr 1 chanMgr 2 stats 3 notificationBuffer 4 sys 5 ypList) -> bytes: getState().inspect() */
    PCRS_JRPC_UPDATE_INFO = 20,     /* (id, fields: name, desc, genre, url, comment, track の contact, title, artist, album, genre) */
    PCRS_JRPC_YP_CHANNELS = 21,     /* -> yp */
    PCRS_JRPC_READ_STORAGE = 22,    /* (a: key) -> int: 開けたら 1、bytes: 中身 */
    PCRS_JRPC_WRITE_STORAGE = 23,   /* (a: key, b: 中身) */
    PCRS_JRPC_CHANNELS_FOUND = 24   /* -> found と、そのあとに found_hit (IP アドレスのあるヒット) */
};

typedef struct pcrs_jrpc_host {
    void *ctx;
    void (*log)(void *ctx, int level, const uint8_t *msg, size_t len);  /* 0 DEBUG、1 INFO、2 WARN、3 ERROR */
    /* 0 成功、1 例外、2 std::domain_error。1 と 2 は pcrs_jrpc_put_error で what() を渡す */
    int (*call)(void *ctx, int op, const pcrs_jrpc_args *args, pcrs_jrpc_sink *out);
} pcrs_jrpc_host;

void pcrs_jrpc_put_bytes(pcrs_jrpc_sink *out, const uint8_t *s, size_t n);
void pcrs_jrpc_put_int(pcrs_jrpc_sink *out, int64_t v);
void pcrs_jrpc_put_error(pcrs_jrpc_sink *out, const uint8_t *s, size_t n);
void pcrs_jrpc_put_channel(pcrs_jrpc_sink *out, const pcrs_jrpc_channel *c);
void pcrs_jrpc_put_servent(pcrs_jrpc_sink *out, const pcrs_jrpc_servent *s);
void pcrs_jrpc_put_hit(pcrs_jrpc_sink *out, const pcrs_hit *h, const uint8_t *addr, size_t n);  /* addr: rhost[0].ip.str() */
void pcrs_jrpc_put_yp(pcrs_jrpc_sink *out, const pcrs_jrpc_yp *y);
void pcrs_jrpc_put_found(pcrs_jrpc_sink *out, const pcrs_jrpc_found *f);
void pcrs_jrpc_put_found_hit(pcrs_jrpc_sink *out, const pcrs_jrpc_found_hit *h);  /* 直前の found のヒット */

/* JrpcApi::call。0 なら *out に応答の JSON、1 なら応答を書き出せなかった例外の what() */
int pcrs_jrpc_call(const uint8_t *req, size_t n, const pcrs_jrpc_host *host, pcrs_buf *out);

/* JSON の値の通知 (配列とオブジェクトは begin_* と end で囲む。オブジェクトの値の前に key) */
typedef struct pcrs_json_builder {
    void *ctx;
    void (*null_value)(void *ctx);
    void (*boolean)(void *ctx, bool v);
    void (*integer)(void *ctx, int64_t v);
    void (*unsigned_integer)(void *ctx, uint64_t v);
    void (*number)(void *ctx, double v);
    void (*string)(void *ctx, const uint8_t *s, size_t n);
    void (*begin_array)(void *ctx);
    void (*begin_object)(void *ctx);
    void (*key)(void *ctx, const uint8_t *s, size_t n);
    void (*end)(void *ctx);
} pcrs_json_builder;

/* メソッドを直接呼ぶ (JrpcApi::getChannels など)。args は位置引数の配列の JSON。0 なら結果を
   builder に通知する。1 method_not_found、2 invalid_params、3 application_error (*code に番号)、
   4 そのほかの例外。1〜4 は *what に what() (常に pcrs_buf_free で返す) */
int pcrs_jrpc_invoke(const uint8_t *method, size_t method_len, const uint8_t *args, size_t args_len,
                     const pcrs_jrpc_host *host, const pcrs_json_builder *builder, int32_t *code, pcrs_buf *what);

/* ---- servhs (core/common/servhs.cpp の要求の解釈と判断) ---- */
/* 文字列は C の文字列 (NUL の手前まで) を渡す。pcrs_buf の出力は常に書く (pcrs_buf_free で返す) */
enum {  /* pcrs_servhs_request_kind */
    PCRS_REQ_GET = 0, PCRS_REQ_POST = 1, PCRS_REQ_GIV = 2, PCRS_REQ_PCP = 3, PCRS_REQ_SOURCE = 4,
    PCRS_REQ_SHOUTCAST = 5, PCRS_REQ_BAD = 6
};
int pcrs_servhs_request_kind(const uint8_t *line, size_t n, const uint8_t *password, size_t pn);
bool pcrs_servhs_is_http(const uint8_t *line, size_t n);          /* stristr(line, "HTTP/1.") */
bool pcrs_servhs_is_valid_html_path(const uint8_t *s, size_t n);  /* ServMgr::isValidHtmlPath */
bool pcrs_servhs_is_decimal(const uint8_t *s, size_t n);          /* ^(0|[1-9][0-9]*)$ */

enum {  /* pcrs_servhs_get_route */
    PCRS_GET_ADMIN = 0, PCRS_GET_ADMIN_SLASH = 1, PCRS_GET_HTML_INDEX = 2, PCRS_GET_HTML = 3,
    PCRS_GET_ADMIN_CGI = 4, PCRS_GET_PLS = 5, PCRS_GET_STREAM = 6, PCRS_GET_CHANNEL = 7, PCRS_GET_API1 = 8,
    PCRS_GET_PUBLIC = 9, PCRS_GET_ASSETS = 10, PCRS_GET_CGI_BIN_FLV = 11, PCRS_GET_CGI_BIN = 12,
    PCRS_GET_CMD = 13, PCRS_GET_OTHER = 14
};
/* handshakeGET のパス (cmdLine + 4)。" HTTP/1." の手前で切る位置 (fn からの位置。-1 もある) があれば
   *has_cut を true にして *cut に書く */
int pcrs_servhs_get_route(const uint8_t *path, size_t n, bool *has_cut, ptrdiff_t *cut);
/* /admin.cgi。pass= と song= があれば true。mount= と url= はあれば *has_* を true にする */
bool pcrs_servhs_admin_cgi(const uint8_t *fn, size_t n, pcrs_buf *song, bool *has_mount, pcrs_buf *mount,
                           bool *has_url, pcrs_buf *url);
enum { PCRS_POST_API1 = 0, PCRS_POST_PUSH = 1, PCRS_POST_ADMIN = 2, PCRS_POST_OTHER = 3 };
/* handshakePOST。行が空白で 3 つに分かれなければ -1。*args は ? の後ろ */
int pcrs_servhs_post_route(const uint8_t *line, size_t n, pcrs_buf *args);
void pcrs_servhs_giv_id(const uint8_t *line, size_t n, uint8_t *id);   /* 16 バイト */
/* handshakeSOURCE。ICY の行ならパスワードがあるので true (ICE/1.0 なら false) */
bool pcrs_servhs_source(const uint8_t *line, size_t n, pcrs_buf *password, pcrs_buf *mount);
bool pcrs_servhs_valid_auth_token(const uint8_t *s, size_t n, const uint8_t *broadcast_id);
/* Cookie ヘッダーの <port>_id。0 見つからない、1 見つかった、2 = のない組があった */
int pcrs_servhs_cookie_id(const uint8_t *header, size_t n, uint16_t port, pcrs_buf *id);
/* nextCGIarg を最後まで繰り返したもの (名前と値を交互に並べる) */
pcrs_vec pcrs_servhs_cgi_args(const uint8_t *cmd, size_t n);
enum {  /* pcrs_servhs_apply_ops の key (src/servhs.rs の ApplyKey) */
    PCRS_APPLY_SERVER_NAME = 0, PCRS_APPLY_SERVER_ACTIVE, PCRS_APPLY_PORT, PCRS_APPLY_ICY_META, PCRS_APPLY_PASS_NEW,
    PCRS_APPLY_ROOT, PCRS_APPLY_BR_ROOT, PCRS_APPLY_GET_UPD, PCRS_APPLY_HU_INT, PCRS_APPLY_FORCE_IP,
    PCRS_APPLY_HTML_PATH, PCRS_APPLY_DJ_MSG, PCRS_APPLY_PC_MSG, PCRS_APPLY_MAX_CIN, PCRS_APPLY_MAX_SIN,
    PCRS_APPLY_MAX_UP, PCRS_APPLY_MAX_RELAYS, PCRS_APPLY_MAX_DIRECT, PCRS_APPLY_MAX_RELAY_PC, PCRS_APPLY_FILT_IP,
    PCRS_APPLY_FILT_BAN, PCRS_APPLY_FILT_PRIVATE, PCRS_APPLY_FILT_NETWORK, PCRS_APPLY_FILT_DIRECT,
    PCRS_APPLY_CHANNEL_FEED_URL, PCRS_APPLY_CLIENT_ACTIVE, PCRS_APPLY_YP, PCRS_APPLY_DEAD_HIT_AGE, PCRS_APPLY_REFRESH,
    PCRS_APPLY_CHAT, PCRS_APPLY_RANDOMIZE_CHID, PCRS_APPLY_PUBLIC_DIRECTORY, PCRS_APPLY_AUTH, PCRS_APPLY_EXPIRE,
    PCRS_APPLY_LOG_LEVEL, PCRS_APPLY_ALLOW_HTML, PCRS_APPLY_ALLOW_NETWORK, PCRS_APPLY_ALLOW_BROADCAST,
    PCRS_APPLY_ALLOW_DIRECT, PCRS_APPLY_TRANSCODING, PCRS_APPLY_PRESET, PCRS_APPLY_AUDIO_CODEC,
    PCRS_APPLY_PREFERRED_THEME, PCRS_APPLY_ACCENT_COLOR
};
/* CMD_apply の引数を読み、行うことを順に op で知らせる。op は例外を投げないこと */
void pcrs_servhs_apply_ops(const uint8_t *cmd, size_t n, void *ctx,
                           void (*op)(void *ctx, int key, int32_t value, const uint8_t *s, size_t len));
bool pcrs_servhs_redirect_url(const uint8_t *cmd, size_t n, pcrs_buf *out);    /* CMD_redirect */
bool pcrs_servhs_rewrite_referer(const uint8_t *referer, size_t n, const uint8_t *path, size_t pn, pcrs_buf *out);
enum {  /* pcrs_servhs_icy_header */
    PCRS_ICY_NAME = 0, PCRS_ICY_URL, PCRS_ICY_BITRATE, PCRS_ICY_GENRE, PCRS_ICY_DESC, PCRS_ICY_AUTHORIZATION,
    PCRS_ICY_CHANNEL_ID, PCRS_ICY_PASSWORD, PCRS_ICY_CONTENT_TYPE, PCRS_ICY_OTHER
};
int pcrs_servhs_icy_header(const uint8_t *line, size_t n);
const char *pcrs_servhs_icy_content_type(const uint8_t *value, size_t n);  /* "OGG" など、"PCP"、NULL */
const char *pcrs_servhs_mime_type(const uint8_t *name, size_t n);          /* fileNameToMimeType */
enum { PCRS_PAGE_PLAY = 0, PCRS_PAGE_RELAY_INFO = 1, PCRS_PAGE_CONNECTIONS = 2, PCRS_PAGE_PLAIN = 3 };
int pcrs_servhs_local_file(const uint8_t *fn, size_t n, bool *split_ok, pcrs_buf *id);
pcrs_buf pcrs_servhs_local_file_name(const uint8_t *root, size_t rn, const uint8_t *fn, size_t n);
bool pcrs_servhs_cgi_server_name(const uint8_t *host, size_t n, pcrs_buf *out);
bool pcrs_servhs_cgi_header_line(const uint8_t *line, size_t n, pcrs_buf *name, pcrs_buf *value);
/* handshakeJRPC の本体の長さ。だめなら負の数 (-411、-400、-413) で、*status_line に状態の行 */
int32_t pcrs_servhs_jrpc_body_length(const uint8_t *s, size_t n, int32_t max, const char **status_line);

/* ---- 段階 8c: mapper、assets、public、HTTPRequest ---- */

/* FileSystemMapper::toLocalFilePath の前半。vpath が virtual_path の下でなければ false */
bool pcrs_mapper_local_path(const uint8_t *virtual_path, size_t vn, const uint8_t *document_root, size_t dn,
                            const uint8_t *vpath, size_t n, pcrs_buf *out);
/* resolvePath で試すパスと言語を (パス, 言語) の順に並べる。langs は pcrs_str_join の引数と同じ形 */
pcrs_vec pcrs_mapper_candidates(const uint8_t *raw, size_t n, const uint8_t *langs_joined, size_t langs_joined_len,
                                const size_t *langs_lens, size_t langs_count);
/* 解決したパスが文書のディレクトリの中 (そのものではない) にあるか。続きが区切りであることも見る */
bool pcrs_mapper_inside(const uint8_t *document_root, size_t dn, const uint8_t *resolved, size_t n);
/* PublicController::operator() の振り分け: 0 /public、1 /public/、2 index.txt、3 play.html、4 ほか */
int32_t pcrs_public_route(const uint8_t *path, size_t n);
/* public.cpp と assets.cpp の MIMEType (静的な文字列) */
const char *pcrs_public_mime_type(const uint8_t *path, size_t n);
const char *pcrs_assets_mime_type(const uint8_t *path, size_t n);
/* AssetsController で 304 を返すか。last_modified はわからなければ -1 */
bool pcrs_assets_not_modified(int64_t last_modified, const uint8_t *ims, size_t n);
/* HTTPRequest のコンストラクターの URL の分割 */
void pcrs_http_split_url(const uint8_t *url, size_t n, pcrs_buf *path, pcrs_buf *query);
/* PublicController::createChannelIndex。0 なら out に index.txt、1 なら例外の what() */
int pcrs_public_channel_index(const pcrs_jrpc_host *host, const uint8_t *tip, size_t n, pcrs_buf *out);

/* ---- 段階 9a ---- */

/* Regexp::exec。-1 正規表現の誤り、0 一致しない、1 一致 (*out に各グループ) */
int32_t pcrs_regex_exec(const uint8_t *pattern, size_t pn, const uint8_t *subject, size_t sn, pcrs_vec *out);
/* IP::tryParse (*out に 16 バイト) と IP::str */
bool pcrs_ip_parse(const uint8_t *s, size_t n, uint8_t *out);
pcrs_buf pcrs_ip_str(const uint8_t *ip);
/* Host::fromStrIP (name が false) か Host::fromStrName (true) */
void pcrs_host_from_str(const uint8_t *s, size_t n, uint16_t default_port, bool name, uint8_t *ip, uint16_t *port);
/* ServFilter の setPattern、getPattern、matches、isGlobal、isSet */
bool pcrs_servfilter_probe(const uint8_t *pattern, size_t n, uint32_t flags, const uint8_t *ip, uint16_t port, uint32_t fl,
                           pcrs_buf *out, bool *global, bool *set);

#ifdef __cplusplus
}
#endif

#endif
