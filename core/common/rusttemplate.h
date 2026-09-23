// テンプレートエンジン (peercast-rs の src/template) を Template から使うための部品。
// WITH_RUST_CORE のビルドで template.cpp から使うほか、peercast-rs の差分テストからも使う。
//
// 式とディレクティブの処理は Rust が行い、スコープ (変数の値) と正規表現は C++ のまま、
// pcrs_template_host のコールバックで Rust に貸す。
#ifndef _RUSTTEMPLATE_H
#define _RUSTTEMPLATE_H

#include <cstring>
#include <exception>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include "regexp.h"
#include "rustbridge.h"
#include "template.h"

namespace rustbridge
{

// ---- 値の受け渡し (src/template/value.rs の encode / decode と同じ形式)

inline void putU32(std::string& out, uint32_t v)
{
    for (int i = 0; i < 4; i++)
        out += static_cast<char>((v >> (8 * i)) & 0xff);
}

inline void putF64(std::string& out, double d)
{
    uint64_t u;
    memcpy(&u, &d, 8);
    for (int i = 0; i < 8; i++)
        out += static_cast<char>((u >> (8 * i)) & 0xff);
}

inline void putBytes(std::string& out, const std::string& s)
{
    putU32(out, static_cast<uint32_t>(s.size()));
    out += s;
}

inline void encodeValue(const amf0::Value& v, std::string& out)
{
    switch (v.m_type)
    {
    case amf0::Value::kNull: out += '\0'; break;
    case amf0::Value::kNumber: out += '\1'; putF64(out, v.m_number); break;
    case amf0::Value::kString: out += '\2'; putBytes(out, v.m_string); break;
    case amf0::Value::kBool: out += '\3'; out += v.m_bool ? '\1' : '\0'; break;
    case amf0::Value::kDate:
        out += '\4';
        putF64(out, v.m_date.unixTime);
        out += static_cast<char>(v.m_date.timezone & 0xff);
        out += static_cast<char>(v.m_date.timezone >> 8);
        break;
    case amf0::Value::kObject:
    case amf0::Value::kArray:
        out += (v.m_type == amf0::Value::kObject) ? '\5' : '\6';
        putU32(out, static_cast<uint32_t>(v.m_object.size()));
        for (auto& pair : v.m_object)
        {
            putBytes(out, pair.first);
            encodeValue(pair.second, out);
        }
        break;
    case amf0::Value::kStrictArray:
        out += '\7';
        putU32(out, static_cast<uint32_t>(v.m_strict_array.size()));
        for (auto& e : v.m_strict_array)
            encodeValue(e, out);
        break;
    default:
        throw std::runtime_error("encodeValue: unknown type");
    }
}

inline std::string encodeValue(const amf0::Value& v)
{
    std::string out;
    encodeValue(v, out);
    return out;
}

class ValueDecoder
{
public:
    ValueDecoder(const uint8_t* p, size_t n) : m_p(p), m_n(n), m_pos(0) {}

    amf0::Value decode()
    {
        amf0::Value v = value();
        if (m_pos != m_n)
            throw std::runtime_error("decodeValue: trailing data");
        return v;
    }

private:
    const uint8_t* take(size_t n)
    {
        if (n > m_n - m_pos)
            throw std::runtime_error("decodeValue: truncated");
        const uint8_t* p = m_p + m_pos;
        m_pos += n;
        return p;
    }
    uint32_t u32()
    {
        const uint8_t* p = take(4);
        return p[0] | (p[1] << 8) | (p[2] << 16) | (static_cast<uint32_t>(p[3]) << 24);
    }
    double f64()
    {
        const uint8_t* p = take(8);
        uint64_t u = 0;
        for (int i = 7; i >= 0; i--)
            u = (u << 8) | p[i];
        double d;
        memcpy(&d, &u, 8);
        return d;
    }
    std::string bytes()
    {
        uint32_t n = u32();
        const uint8_t* p = take(n);
        return std::string(reinterpret_cast<const char*>(p), n);
    }

    amf0::Value value()
    {
        switch (*take(1))
        {
        case 0: return amf0::Value();
        case 1: return amf0::Value(f64());
        case 2: return amf0::Value(bytes());
        case 3: return amf0::Value(*take(1) != 0);
        case 4:
        {
            double t = f64();
            const uint8_t* p = take(2);
            return amf0::Value::date(t, static_cast<uint16_t>(p[0] | (p[1] << 8)));
        }
        case 5:
        case 6:
        {
            bool object = m_p[m_pos - 1] == 5;
            uint32_t n = u32();
            std::map<std::string, amf0::Value> m;
            for (uint32_t i = 0; i < n; i++)
            {
                std::string k = bytes();
                m[k] = value();
            }
            amf0::Value v(m);
            if (!object)
                v.m_type = amf0::Value::kArray;
            return v;
        }
        case 7:
        {
            uint32_t n = u32();
            std::vector<amf0::Value> a;
            for (uint32_t i = 0; i < n; i++)
                a.push_back(value());
            return amf0::Value(a);
        }
        default:
            throw std::runtime_error("decodeValue: unknown type");
        }
    }

    const uint8_t* m_p;
    size_t m_n, m_pos;
};

inline amf0::Value decodeValue(const uint8_t* p, size_t n)
{
    return ValueDecoder(p, n).decode();
}

// ---- Template を Rust に貸す

// 1 回の呼び出しの間、Template (のスコープ) と入出力の Stream を Rust に貸す。Rust の求めで
// 置いた GenericScope は、ここで持って、終わったら (例外で中断しても) 取り除く。
class TemplateHost
{
public:
    TemplateHost(Template& t, Stream* in, Stream* out)
        : m_t(t), m_in(in), m_out(out), m_reader(in ? new StreamReader(*in) : nullptr)
    {
        m_host.ctx = this;
        m_host.reader = m_reader ? m_reader->get() : nullptr;
        m_host.position = [](void* c, int32_t* out) {
            return self(c)->guard([&]() { *out = self(c)->m_in->getPosition(); });
        };
        m_host.seek = [](void* c, int32_t pos) {
            return self(c)->guard([&]() { self(c)->m_in->seekTo(pos); });
        };
        m_host.write = [](void* c, const uint8_t* data, size_t len) {
            return self(c)->guard([&]() { self(c)->m_out->write(data, static_cast<int>(len)); });
        };
        m_host.lookup = [](void* c, const uint8_t* name, size_t n, const uint8_t** value, size_t* len) {
            return self(c)->guard([&]() {
                std::string s(reinterpret_cast<const char*>(name), n);
                amf0::Value v;
                self(c)->m_t.writeVariable(v, s.c_str());
                self(c)->m_buf = encodeValue(v);
                *value = reinterpret_cast<const uint8_t*>(self(c)->m_buf.data());
                *len = self(c)->m_buf.size();
            });
        };
        m_host.push_scope = [](void* c) {
            GenericScope* scope = new GenericScope;
            self(c)->m_pushed.emplace_back(scope);
            self(c)->m_t.prependScope(*scope);
        };
        m_host.pop_scope = [](void* c) { self(c)->popFront(); };
        m_host.front_is_generic = [](void* c) {
            auto& scopes = self(c)->m_t.m_scopes;
            return !scopes.empty() && dynamic_cast<GenericScope*>(scopes.front()) != nullptr;
        };
        m_host.set_front = [](void* c, const uint8_t* name, size_t n, const uint8_t* value, size_t len) {
            return self(c)->guard([&]() {
                auto& scopes = self(c)->m_t.m_scopes;
                GenericScope* front = scopes.empty() ? nullptr : dynamic_cast<GenericScope*>(scopes.front());
                if (!front)
                    throw GeneralException("Cannot change this scope.");
                front->vars[std::string(reinterpret_cast<const char*>(name), n)] = decodeValue(value, len);
            });
        };
        m_host.regex_check = [](void* c, const uint8_t* pattern, size_t n) {
            return self(c)->guard([&]() { Regexp r(std::string(reinterpret_cast<const char*>(pattern), n)); });
        };
        m_host.regex_match = [](void* c, const uint8_t* pattern, size_t n, const uint8_t* subject, size_t m, bool* out) {
            return self(c)->guard([&]() {
                Regexp r(std::string(reinterpret_cast<const char*>(pattern), n));
                *out = r.matches(std::string(reinterpret_cast<const char*>(subject), m));
            });
        };
        m_host.selected_fragment = [](void* c, const uint8_t** out, size_t* n) {
            *out = reinterpret_cast<const uint8_t*>(self(c)->m_t.selectedFragment.data());
            *n = self(c)->m_t.selectedFragment.size();
        };
        m_host.current_fragment = [](void* c, const uint8_t** out, size_t* n) {
            *out = reinterpret_cast<const uint8_t*>(self(c)->m_t.currentFragment.data());
            *n = self(c)->m_t.currentFragment.size();
        };
        m_host.set_current_fragment = [](void* c, const uint8_t* f, size_t n) {
            self(c)->m_t.currentFragment.assign(reinterpret_cast<const char*>(f), n);
        };
        m_host.log_error = [](void*, const uint8_t* msg, size_t n) {
            LOG_ERROR("%.*s", static_cast<int>(n), reinterpret_cast<const char*>(msg));
        };
    }

    ~TemplateHost()
    {
        // 例外で中断して残った GenericScope を取り除く
        // (C++ 版では、スタック上の破棄されたスコープへのポインタが残っていた)。
        for (auto& p : m_pushed)
            m_t.m_scopes.remove(p.get());
    }

    TemplateHost(const TemplateHost&) = delete;
    TemplateHost& operator=(const TemplateHost&) = delete;

    const pcrs_template_host* get() const { return &m_host; }

    // コールバックの中で起きた例外があれば投げ直す
    void rethrowIfAborted()
    {
        if (m_ex)
            std::rethrow_exception(m_ex);
        if (m_reader)
            m_reader->rethrowIfAborted();
    }

private:
    static TemplateHost* self(void* c) { return static_cast<TemplateHost*>(c); }

    template <typename F>
    int guard(F f)
    {
        try {
            f();
            return 0;
        } catch (...) {
            m_ex = std::current_exception();
            return -1;
        }
    }

    void popFront()
    {
        auto& scopes = m_t.m_scopes;
        if (scopes.empty())
            return;
        Template::Scope* front = scopes.front();
        scopes.pop_front();
        for (auto it = m_pushed.begin(); it != m_pushed.end(); ++it)
        {
            if (it->get() == front)
            {
                m_pushed.erase(it);
                break;
            }
        }
    }

    Template& m_t;
    Stream* m_in;
    Stream* m_out;
    std::unique_ptr<StreamReader> m_reader;
    std::vector<std::unique_ptr<GenericScope>> m_pushed;
    std::string m_buf;
    pcrs_template_host m_host;
    std::exception_ptr m_ex;
};

// pcrs_template_call の結果に応じて、値を返すか、C++ 版と同じ種類の例外を投げる。
inline amf0::Value templateResult(int code, RustBuf& result)
{
    std::string r = result.str();
    switch (code)
    {
    case 0: return decodeValue(reinterpret_cast<const uint8_t*>(r.data()), r.size());
    case 2: throw GeneralException(r);
    case 3: throw StreamException(r);
    case 4: throw std::runtime_error(r);
    case 5: throw std::out_of_range(r);
    case 6: throw std::invalid_argument(r);
    default: throw GeneralException("Template: internal error");
    }
}

// Template の処理を 1 つ Rust で行う。t が NULL なら、ホストを使わない処理 (字句解析など) だけ。
inline amf0::Value templateCall(Template* t, Stream* in, Stream* out, int op, const amf0::Value& arg)
{
    std::string a = encodeValue(arg);
    const uint8_t* ap = reinterpret_cast<const uint8_t*>(a.data());
    RustBuf result;
    if (!t)
    {
        int code = pcrs_template_call(op, nullptr, ap, a.size(), result.out());
        return templateResult(code, result);
    }
    TemplateHost host(*t, in, out);
    int code = pcrs_template_call(op, host.get(), ap, a.size(), result.out());
    host.rethrowIfAborted();
    return templateResult(code, result);
}

} // namespace rustbridge

#endif
