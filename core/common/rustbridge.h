// C++ と Rust (peercast-rs) の橋渡しに使う小さな部品。rustcore.cpp と、peercast-rs の差分テストから使う。
// WITH_RUST_CORE のビルドでだけ使う (peercast_rs.h が必要)。
#ifndef _RUSTBRIDGE_H
#define _RUSTBRIDGE_H

#include <cstring>
#include <deque>
#include <exception>
#include <stdexcept>
#include <string>
#include <vector>

#include "amf0.h"
#include "peercast_rs.h"
#include "stream.h"
#include "xml.h"

namespace rustbridge
{

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


// Stream を pcrs_reader として Rust に渡す。コールバックの中で起きた例外は保存しておき、
// Rust の関数から戻ったあとで rethrowIfAborted() が投げ直す (例外は Rust を通り抜けられない)。
class StreamReader
{
public:
    explicit StreamReader(Stream& in) : m_in(in)
    {
        m_reader.ctx = this;
        m_reader.read_char = readChar;
        m_reader.read_exact = readExact;
        m_reader.read_some = readSome;
        m_reader.eof = eof;
    }

    const pcrs_reader* get() const { return &m_reader; }

    void rethrowIfAborted()
    {
        if (m_ex)
            std::rethrow_exception(m_ex);
    }

private:
    static int readChar(void* ctx, uint8_t* out)
    {
        auto self = static_cast<StreamReader*>(ctx);
        try {
            *out = static_cast<uint8_t>(self->m_in.readChar());
            return 0;
        } catch (...) {
            self->m_ex = std::current_exception();
            return -1;
        }
    }

    static int readExact(void* ctx, uint8_t* buf, size_t n)
    {
        auto self = static_cast<StreamReader*>(ctx);
        try {
            std::string s = self->m_in.read(static_cast<int>(n));
            memcpy(buf, s.data(), s.size() < n ? s.size() : n);
            return 0;
        } catch (...) {
            self->m_ex = std::current_exception();
            return -1;
        }
    }

    static int readSome(void* ctx, uint8_t* buf, size_t n, size_t* got)
    {
        auto self = static_cast<StreamReader*>(ctx);
        try {
            int r = self->m_in.read(buf, static_cast<int>(n));
            *got = r > 0 ? static_cast<size_t>(r) : 0;
            return 0;
        } catch (...) {
            self->m_ex = std::current_exception();
            return -1;
        }
    }

    static int eof(void* ctx, bool* out)
    {
        auto self = static_cast<StreamReader*>(ctx);
        try {
            *out = self->m_in.eof();
            return 0;
        } catch (...) {
            self->m_ex = std::current_exception();
            return -1;
        }
    }

    Stream& m_in;
    pcrs_reader m_reader;
    std::exception_ptr m_ex;
};

// Rust の AMF0 デシリアライザから通知を受けて amf0::Value を組み立てる。
class Amf0Builder
{
public:
    Amf0Builder()
    {
        m_builder.ctx = this;
        m_builder.number = [](void* c, double v) { self(c)->leaf([=]() { return amf0::Value::number(v); }); };
        m_builder.string = [](void* c, const uint8_t* s, size_t n) {
            std::string str(reinterpret_cast<const char*>(s), n);
            self(c)->leaf([&]() { return amf0::Value::string(str); });
        };
        m_builder.boolean = [](void* c, bool b) { self(c)->leaf([=]() { return amf0::Value::boolean(b); }); };
        m_builder.null = [](void* c) { self(c)->leaf([]() { return amf0::Value(nullptr); }); };
        m_builder.date = [](void* c, double t, uint16_t tz) { self(c)->leaf([=]() { return amf0::Value::date(t, tz); }); };
        m_builder.begin_object = [](void* c, int kind) { self(c)->guard([&]() { self(c)->m_stack.push_back(Frame(kind == 0 ? Frame::kObject : Frame::kArray)); }); };
        m_builder.key = [](void* c, const uint8_t* s, size_t n) { self(c)->guard([&]() { self(c)->m_stack.back().key.assign(reinterpret_cast<const char*>(s), n); }); };
        m_builder.end_object = [](void* c) { self(c)->guard([&]() { self(c)->close(); }); };
        m_builder.begin_strict_array = [](void* c) { self(c)->guard([&]() { self(c)->m_stack.push_back(Frame(Frame::kStrict)); }); };
        m_builder.end_strict_array = [](void* c) { self(c)->guard([&]() { self(c)->close(); }); };
    }

    const pcrs_amf0_builder* get() const { return &m_builder; }

    // Rust の結果 (pcrs_amf0_read_value の返り値) に応じて、C++ 版と同じ例外を投げる。
    void check(int code, int8_t unknownType, StreamReader& reader)
    {
        if (m_ex)
            std::rethrow_exception(m_ex);
        switch (code)
        {
        case 0: return;
        case 1: reader.rethrowIfAborted(); throw StreamException("AMF0: read aborted");
        case 2: throw std::runtime_error("AMF0: nesting too deep");
        case 3: throw std::runtime_error("AMF0: too many values");
        default: throw std::runtime_error("unknown AMF value type " + std::to_string(static_cast<int>(unknownType)));
        }
    }

    amf0::Value result;
    std::vector<amf0::KeyValuePair> topPairs; // いちばん外側のオブジェクトの中身 (readObject 用)

private:
    struct Frame
    {
        enum Kind { kObject, kArray, kStrict };
        explicit Frame(Kind k) : kind(k) {}
        Kind kind;
        std::string key;
        std::vector<amf0::KeyValuePair> pairs;
        std::vector<amf0::Value> elems;
    };

    static Amf0Builder* self(void* c) { return static_cast<Amf0Builder*>(c); }

    // コールバックから例外を外に出さない。最初の例外を保存し、それ以降の通知は無視する。
    template <typename F> void guard(F f)
    {
        if (m_ex)
            return;
        try { f(); } catch (...) { m_ex = std::current_exception(); }
    }

    template <typename F> void leaf(F make)
    {
        guard([&]() { attach(make()); });
    }

    void attach(const amf0::Value& v)
    {
        if (m_stack.empty())
            result = v;
        else if (m_stack.back().kind == Frame::kStrict)
            m_stack.back().elems.push_back(v);
        else
            m_stack.back().pairs.push_back({m_stack.back().key, v});
    }

    void close()
    {
        Frame f = std::move(m_stack.back());
        m_stack.pop_back();
        amf0::Value v;
        if (f.kind == Frame::kStrict)
            v = amf0::Value::strictArray(f.elems);
        else if (f.kind == Frame::kObject)
            v = amf0::Value::object(f.pairs);
        else
            v = amf0::Value::array(f.pairs);
        if (m_stack.empty())
            topPairs = std::move(f.pairs);
        attach(v);
    }

    pcrs_amf0_builder m_builder;
    std::vector<Frame> m_stack;
    std::exception_ptr m_ex;
};

template <typename T>
T readPrimitive(Stream& in, int (*f)(const pcrs_reader*, T*))
{
    StreamReader reader(in);
    T v{};
    if (f(reader.get(), &v) != 0)
    {
        reader.rethrowIfAborted();
        throw StreamException("AMF0: read aborted");
    }
    return v;
}


// Dechunker::getNextChunk の本体。1 チャンク読んで buffer の後ろに足す。
inline void nextChunk(Stream& in, size_t maxChunkSize, std::deque<char>& buffer, bool& eof)
{
    StreamReader reader(in);
    pcrs_buf data = {nullptr, 0};
    int code = pcrs_dechunk_next(reader.get(), maxChunkSize, &data);
    {
        // C++ 版と同じく、後ろの CRLF が不正でも、読めた中身はバッファに入れてから例外を投げる。
        RustBuf owner(data);
        std::string s = owner.str();
        buffer.insert(buffer.end(), s.begin(), s.end());
    }
    switch (code)
    {
    case 0: return;
    case 1: reader.rethrowIfAborted(); throw StreamException("Dechunker: read aborted");
    case 2: throw StreamException("Protocol error");
    case 3: throw StreamException("Chunk size too large");
    case 4: eof = true; throw StreamException("Closed on read");
    default: throw StreamException("Premature end");
    }
}

// ------------------------------------------------ XML

// XML::Node::setAttributes の本体。attrData (malloc したもの)、attr (new[] したもの)、numAttr を作る。
// C++ 版と同じ例外 (StreamException) を投げる。
inline void parseXmlAttributes(const char* n, char*& attrData, XML::Node::Attribute*& attr, int& numAttr)
{
    size_t len = strlen(n);
    std::vector<size_t> positions(2 * (len + 1));
    size_t count = 0;
    pcrs_buf data = {nullptr, 0};
    int code = pcrs_xml_parse_attributes(reinterpret_cast<const uint8_t*>(n), len, &data, positions.data(), &count);
    if (code == 1)
        throw StreamException("Too many attributes");
    if (code != 0)
        throw StreamException("Bad tag value");

    RustBuf owner(data);
    std::string s = owner.str();
    char* d = static_cast<char*>(malloc(s.size() + 1)); // ~Node が free() で解放する
    if (!d)
        throw std::bad_alloc();
    memcpy(d, s.data(), s.size());
    d[s.size()] = '\0';

    attr = new XML::Node::Attribute[count];
    for (size_t i = 0; i < count; i++)
    {
        attr[i].namePos = static_cast<int>(positions[2 * i]);
        attr[i].valuePos = static_cast<int>(positions[2 * i + 1]);
    }
    attrData = d;
    numAttr = static_cast<int>(count);
}

// XML::read の本体。Rust が読んだ要素を受け取り、C++ 版と同じ形の木を組み立てる。
class XmlReader
{
public:
    explicit XmlReader(XML& xml) : m_xml(xml)
    {
        m_builder.ctx = this;
        m_builder.content = content;
        m_builder.start_tag = startTag;
        m_builder.end_tag = endTag;
    }

    void read(Stream& in)
    {
        StreamReader reader(in);
        int code = pcrs_xml_read(reader.get(), &m_builder);
        switch (code)
        {
        case 0: return;
        case 1: reader.rethrowIfAborted(); throw StreamException("XML: read aborted");
        case 2: std::rethrow_exception(m_ex);
        case 3: throw StreamException("Tag too long");
        case 4: throw StreamException("Content too big");
        case 5: throw StreamException("Not XML document");
        default: throw StreamException("Unexpected end tag");
        }
    }

private:
    template <typename F>
    static int guard(void* ctx, F f)
    {
        auto self = static_cast<XmlReader*>(ctx);
        try {
            f(*self);
            return 0;
        } catch (...) {
            self->m_ex = std::current_exception();
            return -1;
        }
    }

    // Rust 側は、開いているノードがあるときだけ内容と閉じタグを通知する。
    static int content(void* ctx, const uint8_t* s, size_t n)
    {
        return guard(ctx, [&](XmlReader& self) {
            self.m_curr->setContent(std::string(reinterpret_cast<const char*>(s), n).c_str());
        });
    }

    static int startTag(void* ctx, const uint8_t* s, size_t n, bool single)
    {
        return guard(ctx, [&](XmlReader& self) {
            std::string tag(reinterpret_cast<const char*>(s), n);
            XML::Node* node = new XML::Node("%s", tag.c_str());
            if (self.m_curr)
                self.m_curr->add(node);
            else
                self.m_xml.setRoot(node);
            if (!single)
                self.m_curr = node;
        });
    }

    static int endTag(void* ctx)
    {
        return guard(ctx, [&](XmlReader& self) { self.m_curr = self.m_curr->parent; });
    }

    XML& m_xml;
    XML::Node* m_curr = nullptr;
    pcrs_xml_builder m_builder;
    std::exception_ptr m_ex;
};

} // namespace rustbridge

#endif
