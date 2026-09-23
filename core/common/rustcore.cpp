// Rust 実装 (peercast-rs) を呼ぶ関数。docs/rust-migration.md を参照。
//
// WITH_RUST_CORE が定義されているときだけ、cgi.cpp と str.cpp の該当する関数の代わりに
// ここの関数が使われる。定義されていない場合 (CMake や MinGW のビルド) は、このファイルは空で、
// 従来どおり cgi.cpp と str.cpp の C++ 実装が使われる。
#ifdef WITH_RUST_CORE

#include <cstdio>
#include <stdexcept>
#include <string>

#include "cgi.h"
#include "common.h" // FormatException
#include "peercast_rs.h"
#include "gnuid.h" // GnuID
#include "jis.h" // JISConverter
#include "host.h" // Host
#include "md5.h" // md5::hexdigest
#include "_string.h" // String
#include "http.h" // HTTP
#include "amf0.h" // amf0::Deserializer
#include "dechunker.h" // Dechunker
#include "stream.h" // Stream
#include <exception>
#include <vector>
#include "str.h"
#include "LUrlParser.h" // LUrlParser::clParseURL
#include "url.h" // URLSource
#include "chaninfo.h" // ChanInfo::PROTOCOL
#include "rustbridge.h"
#include "flv.h" // FLVStream (段階4、メディアコンテナの本体は rustmedia.h)

using rustbridge::RustBuf;

namespace
{

inline const uint8_t* bytes(const std::string& s)
{
    return reinterpret_cast<const uint8_t*>(s.data());
}

} // namespace

namespace cgi
{

std::string escape(const std::string& in)
{
    return RustBuf(pcrs_cgi_escape(bytes(in), in.size())).str();
}

std::string unescape(const std::string& in)
{
    return RustBuf(pcrs_cgi_unescape(bytes(in), in.size())).str();
}

std::string unescape_html(const std::string& in)
{
    return RustBuf(pcrs_cgi_unescape_html(bytes(in), in.size())).str();
}

std::string escape_html(const std::string& in)
{
    return RustBuf(pcrs_cgi_escape_html(bytes(in), in.size())).str();
}

std::string escape_javascript(const std::string& in)
{
    return RustBuf(pcrs_cgi_escape_javascript(bytes(in), in.size())).str();
}

bool isSafeLocalPath(const std::string& path)
{
    return pcrs_cgi_is_safe_local_path(bytes(path), path.size());
}

time_t parseHttpDate(const std::string& str)
{
    return static_cast<time_t>(pcrs_cgi_parse_http_date(bytes(str), str.size()));
}

} // namespace cgi

namespace str
{

std::string codepoint_to_utf8(uint32_t codepoint)
{
    RustBuf buf;
    if (pcrs_str_codepoint_to_utf8(codepoint, buf.out()) != 0)
    {
        char message[96];
        snprintf(message, sizeof(message),
                 "Codepoint U+%04lX is not a valid Unicode scalar value",
                 static_cast<unsigned long>(codepoint));
        throw std::out_of_range(message);
    }
    return buf.str();
}

std::string inspect(const std::string& s)
{
    return RustBuf(pcrs_str_inspect(bytes(s), s.size())).str();
}

std::string json_inspect(const std::string& s)
{
    RustBuf buf;
    if (pcrs_str_json_inspect(bytes(s), s.size(), buf.out()) != 0)
        throw std::invalid_argument("json_inspect: UTF-8 validation failed");
    return buf.str();
}

bool is_http_url(const std::string& url)
{
    return pcrs_str_is_http_url(bytes(url), url.size());
}

namespace
{

// pcrs_vec の持ち主。C++ 側の vector<string> に変換してから返す。
std::vector<std::string> take_vec(pcrs_vec v)
{
    std::vector<std::string> res;
    res.reserve(v.count);
    const char* p = reinterpret_cast<const char*>(v.joined.ptr);
    size_t off = 0;
    for (size_t i = 0; i < v.count; i++)
    {
        size_t len = v.lens[i];
        res.emplace_back(p + off, len);
        off += len;
    }
    pcrs_vec_free(v);
    return res;
}

} // namespace

std::string hexdump(const std::string& in)
{
    return RustBuf(pcrs_str_hexdump(bytes(in), in.size())).str();
}

std::string repeat(const std::string& in, int n)
{
    // n <= 0 は Rust 側で空文字列になる (C++ 版と同じ)。
    std::string res;
    for (int i = 0; i < n; i++) res += in;
    return res; // 短い繰り返しなので、わざわざ FFI を呼ばない
}

std::string group_digits(const std::string& in, const std::string& separator)
{
    return RustBuf(pcrs_str_group_digits(bytes(in), in.size(), bytes(separator), separator.size())).str();
}

std::vector<std::string> split(const std::string& in, const std::string& separator)
{
    return take_vec(pcrs_str_split(bytes(in), in.size(), bytes(separator), separator.size()));
}

std::vector<std::string> split(const std::string& in, const std::string& separator, int limit)
{
    pcrs_vec v{};
    if (pcrs_str_split_limit(bytes(in), in.size(), bytes(separator), separator.size(), limit, &v) != 0)
        throw std::domain_error("limit <= 0");
    return take_vec(v);
}

bool contains(const std::string& haystack, const std::string& needle)
{
    return pcrs_str_contains(bytes(haystack), haystack.size(), bytes(needle), needle.size());
}

std::string replace_prefix(const std::string& s, const std::string& prefix, const std::string& replacement)
{
    return RustBuf(pcrs_str_replace_prefix(bytes(s), s.size(), bytes(prefix), prefix.size(),
                                            bytes(replacement), replacement.size())).str();
}

std::string replace_suffix(const std::string& s, const std::string& suffix, const std::string& replacement)
{
    return RustBuf(pcrs_str_replace_suffix(bytes(s), s.size(), bytes(suffix), suffix.size(),
                                            bytes(replacement), replacement.size())).str();
}

std::string upcase(const std::string& input)
{
    return RustBuf(pcrs_str_upcase(bytes(input), input.size())).str();
}

std::string downcase(const std::string& input)
{
    return RustBuf(pcrs_str_downcase(bytes(input), input.size())).str();
}

std::string capitalize(const std::string& input)
{
    return RustBuf(pcrs_str_capitalize(bytes(input), input.size())).str();
}

bool has_prefix(const std::string& subject, const std::string& prefix)
{
    return pcrs_str_has_prefix(bytes(subject), subject.size(), bytes(prefix), prefix.size());
}

bool has_suffix(const std::string& subject, const std::string& suffix)
{
    return pcrs_str_has_suffix(bytes(subject), subject.size(), bytes(suffix), suffix.size());
}

std::string join(const std::string& delimiter, const std::vector<std::string>& vec)
{
    std::string joined;
    std::vector<size_t> lens;
    lens.reserve(vec.size());
    for (auto& s : vec) { joined += s; lens.push_back(s.size()); }
    return RustBuf(pcrs_str_join(bytes(delimiter), delimiter.size(),
                                  reinterpret_cast<const uint8_t*>(joined.data()), joined.size(),
                                  lens.data(), lens.size())).str();
}

std::string ascii_dump(const std::string& in, const std::string& replacement)
{
    return RustBuf(pcrs_str_ascii_dump(bytes(in), in.size(), bytes(replacement), replacement.size())).str();
}

std::string extension_without_dot(const std::string& filename)
{
    return RustBuf(pcrs_str_extension_without_dot(bytes(filename), filename.size())).str();
}

int count(const std::string& haystack, const std::string& needle)
{
    int32_t n = 0;
    if (pcrs_str_count(bytes(haystack), haystack.size(), bytes(needle), needle.size(), &n) != 0)
        throw std::domain_error("cannot count empty strings");
    return n;
}

std::string rstrip(const std::string& str)
{
    return RustBuf(pcrs_str_rstrip(bytes(str), str.size())).str();
}

std::string strip(const std::string& str)
{
    return RustBuf(pcrs_str_strip(bytes(str), str.size())).str();
}

std::string escapeshellarg_unix(const std::string& str)
{
    return RustBuf(pcrs_str_escapeshellarg_unix(bytes(str), str.size())).str();
}

std::vector<std::string> to_lines(const std::string& text)
{
    return take_vec(pcrs_str_to_lines(bytes(text), text.size()));
}

std::string indent_tab(const std::string& text, int n)
{
    RustBuf buf;
    if (pcrs_str_indent_tab(bytes(text), text.size(), n, buf.out()) != 0)
        throw std::domain_error("domain error");
    return buf.str();
}

std::vector<std::string> shellwords(const std::string& str)
{
    pcrs_vec v{};
    int error_kind = 0;
    if (pcrs_str_shellwords(bytes(str), str.size(), &v, &error_kind) != 0)
    {
        switch (error_kind)
        {
        case 0: throw FormatException("Unterminated single-quoted string");
        case 1: throw FormatException("Unterminated double-quoted string");
        default: throw FormatException("Unfinished backlash escape");
        }
    }
    return take_vec(v);
}

bool validate_utf8(const std::string& s)
{
    return pcrs_str_validate_utf8(bytes(s), s.size());
}

std::string truncate_utf8(const std::string& s, size_t length)
{
    RustBuf buf;
    if (pcrs_str_truncate_utf8(bytes(s), s.size(), length, buf.out()) != 0)
        throw std::invalid_argument("truncate_utf8: UTF-8 validation failed");
    return buf.str();
}

std::string valid_utf8(const std::string& s)
{
    return RustBuf(pcrs_str_valid_utf8(bytes(s), s.size())).str();
}

} // namespace str

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

std::string md5::hexdigest(std::string str)
{
    return RustBuf(pcrs_md5_hexdigest(reinterpret_cast<const uint8_t*>(str.data()), str.size())).str();
}

void GnuID::toStr(char *str) const
{
    uint8_t buf[32];
    pcrs_gnuid_to_str(id, buf);
    memcpy(str, buf, 32);
    str[32] = 0;
}

void GnuID::fromStr(const char *str)
{
    pcrs_gnuid_from_str(reinterpret_cast<const uint8_t*>(str), strlen(str), id);
}

void GnuID::encode(Host *h, const char *salt1, const char *salt2, unsigned char salt3)
{
    // Rust 版と同じ意味にするため、元の C++ 実装 ((unsigned char*)&h->ip の先頭4バイト) を
    // そのまま踏襲する。IP アドレスとしての正しさではなく、既存のビット列との互換性が目的。
    const uint8_t* ip_bytes = h ? reinterpret_cast<const uint8_t*>(&h->ip) : nullptr;
    pcrs_gnuid_encode(id, ip_bytes, h != nullptr,
                       reinterpret_cast<const uint8_t*>(salt1), salt1 ? strlen(salt1) : 0,
                       reinterpret_cast<const uint8_t*>(salt2), salt2 ? strlen(salt2) : 0,
                       salt3);
}

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

unsigned short JISConverter::sjisToUnicode(unsigned short sjis)
{
    return pcrs_jis_sjis_to_unicode(sjis);
}

unsigned short JISConverter::eucToUnicode(unsigned short euc)
{
    return pcrs_jis_euc_to_unicode(euc);
}

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

namespace
{

// Rust が返した内容を String::data に書く。Rust 側は MAX_LEN - 1 バイト以下しか返さないが、
// 念のためここでも切り詰める。途中に NUL が含まれていても、そのまま写す (C++ 版と同じ)。
void storeTo(char (&data)[String::MAX_LEN], pcrs_buf buf)
{
    RustBuf owner(buf);
    size_t n = buf.len < String::MAX_LEN - 1 ? buf.len : String::MAX_LEN - 1;
    if (n)
        memcpy(data, buf.ptr, n);
    data[n] = '\0';
}

inline const uint8_t* cbytes(const char* s)
{
    return reinterpret_cast<const uint8_t*>(s);
}

} // namespace

String& String::setFromStopwatch(unsigned int t)
{
    storeTo(data, pcrs_string_from_stopwatch(t));
    type = T_ASCII;
    return *this;
}

String& String::setFromString(const char *str, TYPE t)
{
    storeTo(data, pcrs_string_from_string(cbytes(str), strlen(str)));
    type = t;
    return *this;
}

String& String::setUnquote(const char *p, TYPE t)
{
    storeTo(data, pcrs_string_unquote(cbytes(p), strlen(p)));
    type = t;
    return *this;
}

int String::base64WordToChars(char *out, const char *input)
{
    return pcrs_base64_word_to_chars(cbytes(input), reinterpret_cast<uint8_t*>(out));
}

// 以下の変換関数は、入力として自分自身の data を渡されることがある (convertTo)。
// Rust の結果を受け取ってから data に書くので、入力と出力が同じでも問題ない。

void String::BASE642ASCII(const char *input)
{
    storeTo(data, pcrs_string_base64_to_ascii(cbytes(input), strlen(input)));
}

void String::UNKNOWN2UNICODE(const char *in, bool safe)
{
    storeTo(data, pcrs_string_unknown_to_unicode(cbytes(in), strlen(in), safe));
}

void String::ASCII2ESC(const char *in, bool safe)
{
    storeTo(data, pcrs_string_ascii_to_esc(cbytes(in), strlen(in), safe));
}

void String::ESC2ASCII(const char *in)
{
    storeTo(data, pcrs_string_esc_to_ascii(cbytes(in), strlen(in)));
}

void String::ASCII2META(const char *in, bool safe)
{
    storeTo(data, pcrs_string_ascii_to_meta(cbytes(in), strlen(in), safe));
}

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

bool HTTP::isCrossOriginRequest(const std::string& secFetchSite,
                                const std::string& origin,
                                const std::string& host)
{
    return pcrs_http_is_cross_origin_request(bytes(secFetchSite), secFetchSite.size(),
                                             bytes(origin), origin.size(),
                                             bytes(host), host.size());
}

bool HTTP::isLoopbackHostHeader(const std::string& host)
{
    return pcrs_http_is_loopback_host_header(bytes(host), host.size());
}

namespace
{
// strncpy(dst, src, len); dst[len - 1] = 0; と同じ結果 (len - 1 バイトまで写して NUL 終端)。
void copyTruncated(char* dst, size_t len, const std::string& s)
{
    if (!dst || len == 0)
        return;
    size_t n = s.size() < len - 1 ? s.size() : len - 1;
    memcpy(dst, s.data(), n);
    dst[n] = '\0';
}
} // namespace

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

using rustbridge::StreamReader;
using rustbridge::Amf0Builder;
using rustbridge::readPrimitive;

namespace amf0 {

bool Deserializer::readBool(Stream &in) { return readPrimitive<bool>(in, pcrs_amf0_read_bool); }
int32_t Deserializer::readInt32(Stream &in) { return readPrimitive<int32_t>(in, pcrs_amf0_read_int32); }
int16_t Deserializer::readInt16(Stream& in) { return readPrimitive<int16_t>(in, pcrs_amf0_read_int16); }
double Deserializer::readDouble(Stream &in) { return readPrimitive<double>(in, pcrs_amf0_read_double); }

std::string Deserializer::readString(Stream &in)
{
    StreamReader reader(in);
    pcrs_buf buf = {nullptr, 0};
    if (pcrs_amf0_read_string(reader.get(), &buf) != 0)
    {
        reader.rethrowIfAborted();
        throw StreamException("AMF0: read aborted");
    }
    return RustBuf(buf).str();
}

Value Deserializer::readValue(Stream &in)
{
    StreamReader reader(in);
    Amf0Builder builder;
    int8_t unknownType = 0;
    int code = pcrs_amf0_read_value(reader.get(), builder.get(), &unknownType);
    builder.check(code, unknownType, reader);
    return builder.result;
}

std::vector<KeyValuePair> Deserializer::readObject(Stream &in)
{
    StreamReader reader(in);
    Amf0Builder builder;
    int8_t unknownType = 0;
    int code = pcrs_amf0_read_object(reader.get(), builder.get(), &unknownType);
    builder.check(code, unknownType, reader);
    return builder.topPairs;
}

} // namespace amf0

void Dechunker::getNextChunk()
{
    rustbridge::nextChunk(m_stream, MAX_CHUNK_SIZE, m_buffer, m_eof);
}

LUrlParser::clParseURL LUrlParser::clParseURL::ParseURL(const std::string& URL)
{
    pcrs_vec v{};
    int code = pcrs_url_parse(bytes(URL), URL.size(), &v);
    if (code != 0)
        return clParseURL(static_cast<LUrlParserError>(code));

    std::vector<std::string> parts;
    const char* p = reinterpret_cast<const char*>(v.joined.ptr);
    size_t off = 0;
    for (size_t i = 0; i < v.count; i++)
    {
        parts.emplace_back(p + off, v.lens[i]);
        off += v.lens[i];
    }
    pcrs_vec_free(v);

    clParseURL result;
    result.m_Scheme = parts.at(0);
    result.m_Host = parts.at(1);
    result.m_Port = parts.at(2);
    result.m_Path = parts.at(3);
    result.m_Query = parts.at(4);
    result.m_Fragment = parts.at(5);
    result.m_UserName = parts.at(6);
    result.m_Password = parts.at(7);
    result.m_ErrorCode = LUrlParserError_Ok;
    return result;
}

bool LUrlParser::clParseURL::GetPort(int* OutPort) const
{
    if (!IsValid())
        return false;
    int port = pcrs_url_port_number(bytes(m_Port), m_Port.size());
    if (port == 0)
        return false;
    if (OutPort)
        *OutPort = port;
    return true;
}

ChanInfo::PROTOCOL URLSource::getSourceProtocol(char*& fileName)
{
    size_t skip = 0;
    int proto = pcrs_url_source_protocol(cbytes(fileName), strlen(fileName), &skip);
    fileName += skip;
    return static_cast<ChanInfo::PROTOCOL>(proto);
}

void XML::read(Stream &in)
{
    rustbridge::XmlReader(*this).read(in);
}

void XML::Node::setAttributes(const char *n)
{
    rustbridge::parseXmlAttributes(n, attrData, attr, numAttr);
}

int XML::Node::getBinaryContent(void *ptr, int size)
{
    // C++ 版は内容がない (contData が NULL) と NULL を読んで落ちていた。Rust 版では空として扱う。
    const char* in = contData ? contData : "";
    pcrs_buf out = {nullptr, 0};
    if (pcrs_xml_binary_content(reinterpret_cast<const uint8_t*>(in), strlen(in),
                                size > 0 ? static_cast<size_t>(size) : 0, &out) != 0)
        throw StreamException("Too much binary data");
    RustBuf owner(out);
    if (out.len)
        memcpy(ptr, out.ptr, out.len);
    return static_cast<int>(out.len);
}

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

void HTTP::parseAuthorizationHeader(const char* arg, char* user, char* pass, size_t ulen, size_t plen)
{
    if (!arg)
        return;
    pcrs_buf u = {nullptr, 0}, p = {nullptr, 0};
    if (!pcrs_http_parse_basic_auth(cbytes(arg), strlen(arg), &u, &p))
        return;
    RustBuf ub(u), pb(p);
    copyTruncated(user, ulen, ub.str());
    copyTruncated(pass, plen, pb.str());
}

#endif // WITH_RUST_CORE

#ifdef WITH_RUST_CORE

std::pair<bool,int> FLVStream::readMetaData(void* data, int size)
{
    RustBuf err;
    int32_t bitrate = 0;
    int r = pcrs_flv_read_meta_data(static_cast<const uint8_t*>(data), size > 0 ? size : 0, &bitrate, err.out());
    if (r == 2)
        LOG_ERROR("readMetaData: %s", err.str().c_str());
    if (r == 1)
        return std::make_pair(true, static_cast<int>(bitrate));
    return std::make_pair(false, 0);
}

#endif // WITH_RUST_CORE
