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
#include "str.h"

namespace
{

inline const uint8_t* bytes(const std::string& s)
{
    return reinterpret_cast<const uint8_t*>(s.data());
}

// pcrs_buf の持ち主。例外が飛んでも確実に Rust 側へ返す。
class RustBuf
{
public:
    RustBuf() : m_buf{nullptr, 0} {}
    explicit RustBuf(pcrs_buf buf) : m_buf(buf) {}
    ~RustBuf() { pcrs_buf_free(m_buf); }
    RustBuf(const RustBuf&) = delete;
    RustBuf& operator=(const RustBuf&) = delete;

    pcrs_buf* out() { return &m_buf; }
    std::string str() const
    {
        return std::string(reinterpret_cast<const char*>(m_buf.ptr), m_buf.len);
    }

private:
    pcrs_buf m_buf;
};

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
