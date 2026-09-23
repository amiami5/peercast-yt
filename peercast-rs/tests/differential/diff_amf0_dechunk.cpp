// AMF0 デシリアライザ (core/common/amf0.cpp) と Dechunker (core/common/dechunker.cpp) の、
// C++ 版と Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) のもの。Rust 版は、本番と同じ
// 橋渡しのコード (core/common/rustbridge.h) を通して呼ぶ。どちらも同じ種類のストリーム
// (MemoryStream: データが尽きると 0 を返す / StringStream: 尽きると例外) から読み、
// 結果の値 (または例外の種類とメッセージ) と、読み終えた位置を比べる。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <random>
#include <string>

#include "amf0.h"
#include "dechunker.h"
#include "rustbridge.h"
#include "sstream.h"
#include "stream.h"
#include "../../../tests/mocksys.h" // Stream の統計が sys->getDTime() を使う

static long g_bad = 0, g_cases = 0;
static std::mt19937 rng(20260923);

static std::string hex(const std::string& s)
{
    std::string r;
    char b[4];
    for (unsigned char c : s) { snprintf(b, sizeof b, "%02x", c); r += b; }
    return r;
}

// Value を比較用の文字列にする (inspect は不正な UTF-8 で例外を投げるので使わない)
static std::string dump(const amf0::Value& v)
{
    char b[64];
    switch (v.m_type)
    {
    case amf0::Value::kNumber: { uint64_t u; memcpy(&u, &v.m_number, 8); snprintf(b, sizeof b, "n%016llx", (unsigned long long)u); return b; }
    case amf0::Value::kString: return "s" + hex(v.m_string);
    case amf0::Value::kBool: return v.m_bool ? "T" : "F";
    case amf0::Value::kNull: return "N";
    case amf0::Value::kDate: { uint64_t u; memcpy(&u, &v.m_date.unixTime, 8); snprintf(b, sizeof b, "d%016llx/%u", (unsigned long long)u, v.m_date.timezone); return b; }
    case amf0::Value::kObject:
    case amf0::Value::kArray:
    {
        std::string r = v.m_type == amf0::Value::kObject ? "{" : "A{";
        for (auto& p : v.m_object) r += hex(p.first) + ":" + dump(p.second) + ",";
        return r + "}";
    }
    case amf0::Value::kStrictArray:
    {
        std::string r = "[";
        for (auto& e : v.m_strict_array) r += dump(e) + ",";
        return r + "]";
    }
    }
    return "?";
}

template <typename F>
static std::string guarded(F f)
{
    try { return f(); }
    catch (StreamException& e) { return std::string("StreamException:") + e.what(); }
    catch (std::runtime_error& e) { return std::string("runtime_error:") + e.what(); }
    catch (std::exception& e) { return std::string("exception:") + e.what(); }
}

// ---------------------------------------------------------------- AMF0

static std::string rustReadValue(Stream& in)
{
    rustbridge::StreamReader reader(in);
    rustbridge::Amf0Builder builder;
    int8_t t = 0;
    int code = pcrs_amf0_read_value(reader.get(), builder.get(), &t);
    builder.check(code, t, reader);
    return dump(builder.result);
}

static std::string rustReadObject(Stream& in)
{
    rustbridge::StreamReader reader(in);
    rustbridge::Amf0Builder builder;
    int8_t t = 0;
    int code = pcrs_amf0_read_object(reader.get(), builder.get(), &t);
    builder.check(code, t, reader);
    std::string r;
    for (auto& p : builder.topPairs) r += hex(p.first) + ":" + dump(p.second) + ",";
    return r;
}

static std::string cxxReadObject(Stream& in)
{
    amf0::Deserializer d;
    std::string r;
    for (auto& p : d.readObject(in)) r += hex(p.first) + ":" + dump(p.second) + ",";
    return r;
}

// 同じ入力を、両方の種類のストリームで比べる
template <typename C, typename R>
static void compare(const char* what, const std::string& data, C cxx, R rust)
{
    for (int kind = 0; kind < 2; kind++)
    {
        g_cases++;
        std::string a, b;
        int pa, pb;
        if (kind == 0)
        {
            std::string copy1 = data, copy2 = data;
            MemoryStream m1(&copy1[0], copy1.size()), m2(&copy2[0], copy2.size());
            a = guarded([&]() { return cxx(m1); }); pa = m1.getPosition();
            b = guarded([&]() { return rust(m2); }); pb = m2.getPosition();
        }
        else
        {
            StringStream s1(data), s2(data);
            a = guarded([&]() { return cxx(s1); }); pa = s1.getPosition();
            b = guarded([&]() { return rust(s2); }); pb = s2.getPosition();
        }
        a += " @" + std::to_string(pa);
        b += " @" + std::to_string(pb);
        if (a != b && g_bad++ < 20)
            printf("  [違い] %s (%s) 入力=%s\n    C++ =%s\n    Rust=%s\n", what, kind ? "StringStream" : "MemoryStream",
                   hex(data.substr(0, 200)).c_str(), a.substr(0, 300).c_str(), b.substr(0, 300).c_str());
    }
}

// それらしい AMF0 の値を作る
static std::string genString(int maxlen)
{
    std::string s;
    int len = rng() % (maxlen + 1);
    for (int i = 0; i < len; i++) s += (rng() % 4) ? (char)('a' + rng() % 4) : (char)(rng() % 256);
    std::string r;
    r += (char)(s.size() >> 8);
    r += (char)(s.size() & 0xff);
    return r + s;
}

static std::string genValue(int depth)
{
    std::string r;
    int t = rng() % 12;
    if (depth > 4 && t >= 2 && t <= 5) t = 0;
    switch (t)
    {
    case 0: r += '\x00'; for (int i = 0; i < 8; i++) r += (char)(rng() % 256); break;
    case 1: r += '\x02'; r += genString(6); break;
    case 2: case 3:
    {
        r += (t == 2) ? '\x03' : '\x08';
        if (t == 3) for (int i = 0; i < 4; i++) r += (char)(rng() % 256);
        int n = rng() % 4;
        for (int i = 0; i < n; i++) { r += genString(3); r += genValue(depth + 1); }
        r += std::string("\x00\x00\x09", 3);
        break;
    }
    case 4:
    {
        r += '\x0a';
        int n = rng() % 4;
        r += std::string("\x00\x00\x00", 3); r += (char)n;
        for (int i = 0; i < n; i++) r += genValue(depth + 1);
        break;
    }
    case 5: r += '\x01'; r += (char)(rng() % 3); break;
    case 6: r += '\x05'; break;
    case 7: r += '\x0b'; for (int i = 0; i < 10; i++) r += (char)(rng() % 256); break;
    default: r += (char)(rng() % 256); break; // 不明な型も混ぜる
    }
    return r;
}

static std::string mutateBytes(std::string s)
{
    int n = rng() % 4;
    for (int i = 0; i < n && !s.empty(); i++)
    {
        size_t pos = rng() % s.size();
        switch (rng() % 4)
        {
        case 0: s[pos] = (char)(rng() % 256); break;
        case 1: s.erase(pos, 1); break;
        case 2: s.insert(pos, 1, (char)(rng() % 256)); break;
        case 3: s.resize(pos); break; // 途中で切る
        }
    }
    return s;
}

static void amf0Case(const std::string& data)
{
    compare("readValue", data,
            [](Stream& in) { amf0::Deserializer d; return dump(d.readValue(in)); },
            rustReadValue);
    compare("readObject", data, cxxReadObject, rustReadObject);
    compare("readInt32", data,
            [](Stream& in) { amf0::Deserializer d; return std::to_string(d.readInt32(in)); },
            [](Stream& in) { amf0::Deserializer d; (void)d; return std::to_string(rustbridge::readPrimitive<int32_t>(in, pcrs_amf0_read_int32)); });
    compare("readString", data,
            [](Stream& in) { amf0::Deserializer d; return hex(d.readString(in)); },
            [](Stream& in) {
                rustbridge::StreamReader reader(in);
                pcrs_buf buf = {nullptr, 0};
                if (pcrs_amf0_read_string(reader.get(), &buf) != 0) { reader.rethrowIfAborted(); throw StreamException("aborted"); }
                return hex(rustbridge::RustBuf(buf).str());
            });
}

// ---------------------------------------------------------------- Dechunker

// C++ 版の Dechunker::read をそのまま写し、getNextChunk だけ Rust 版にしたもの。
class RustDechunker
{
public:
    explicit RustDechunker(Stream& s) : m_stream(s) {}
    int read(void* buf, int aSize)
    {
        if (m_eof)
            throw StreamException("Closed on read");
        if (aSize < 0)
            throw GeneralException("Bad argument");
        size_t size = aSize;
        char* p = (char*)buf;
        while (true)
        {
            if (m_buffer.size() >= size)
            {
                while (size > 0) { *p++ = m_buffer.front(); m_buffer.pop_front(); size--; }
                return p - (char*)buf;
            }
            rustbridge::nextChunk(m_stream, Dechunker::MAX_CHUNK_SIZE, m_buffer, m_eof);
        }
    }
    bool m_eof = false;
    std::deque<char> m_buffer;
    Stream& m_stream;
};

template <typename D>
static std::string transcript(D& d, const std::vector<int>& sizes)
{
    std::string r;
    for (int n : sizes)
    {
        std::string buf(n, '\0');
        std::string res = guarded([&]() { int got = d.read(&buf[0], n); return hex(buf.substr(0, got)); });
        r += res + "|";
        if (res.find(':') != std::string::npos) break; // 例外が起きたら終わり
    }
    return r + (d.m_eof ? "EOF" : "");
}

static void dechunkCase(const std::string& data, const std::vector<int>& sizes)
{
    compare("Dechunker", data,
            [&](Stream& in) { Dechunker d(in); return transcript(d, sizes); },
            [&](Stream& in) { RustDechunker d(in); return transcript(d, sizes); });
}

static std::string genChunked()
{
    std::string r;
    int n = rng() % 4;
    for (int i = 0; i < n; i++)
    {
        int len = 1 + rng() % 20;
        char b[16];
        snprintf(b, sizeof b, rng() % 2 ? "%x" : "%X", len);
        r += b;
        r += "\r\n";
        for (int j = 0; j < len; j++) r += (char)('a' + rng() % 26);
        r += "\r\n";
    }
    if (rng() % 4) r += "0\r\n\r\n";
    return r;
}

int main(int argc, char** argv)
{
    sys = new MockSys();
    long iterations = argc > 1 ? atol(argv[1]) : 100000;

    // 決まった例
    amf0Case("");
    amf0Case(std::string("\x05", 1));
    amf0Case(std::string("\x02\x00\x05" "ab", 5));
    amf0Case(std::string("\x0a\x7f\xff\xff\xff", 5) + std::string(200000, '\x05')); // 値が多すぎる
    {
        std::string deep;
        for (int i = 0; i < 40; i++) deep += std::string("\x0a\x00\x00\x00\x01", 5);
        amf0Case(deep + "\x05");
    }
    for (int t = 0; t < 256; t++) amf0Case(std::string(1, (char)t) + std::string(16, '\x00'));

    dechunkCase("", {1});
    dechunkCase("123456789\r\n", {1});
    dechunkCase("FFFFFFF\r\n", {1});
    dechunkCase("2\r\nabXY", {1, 1, 1});

    for (long it = 0; it < iterations; it++)
    {
        std::string v = genValue(0);
        if (rng() % 2) v += genValue(0);
        amf0Case(rng() % 3 ? mutateBytes(v) : v);

        std::string c = genChunked();
        std::vector<int> sizes;
        int k = 1 + rng() % 6;
        for (int i = 0; i < k; i++) sizes.push_back(rng() % 30);
        dechunkCase(rng() % 3 ? mutateBytes(c) : c, sizes);
    }

    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
