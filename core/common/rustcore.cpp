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
