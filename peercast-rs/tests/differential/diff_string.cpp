// String (core/common/_string.cpp) の変換関数の、C++ 版と Rust 版の差分テスト。
//
// C++ 版は、入力の末尾で途中までしかない %XX や UTF-8 の文字に出会うと、終端の NUL を
// 越えて読み進める。ここでは入力を 0 で埋めた大きなバッファに置き、越えて読んだ先が
// 0 になるようにして比べる (Rust 版は「入力の後ろは 0」とみなして入力の終わりで止まる)。
//
// C++ 版の UNKNOWN2UNICODE は str::codepoint_to_utf8 を呼ぶ。製品のビルドでは、これは
// 段階1で Rust 版 (U+0080〜U+07FF の誤りを直したもの) に置き換わっているので、ここでも
// 同じく Rust 版を使う (str.o はリンクしない)。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <stdexcept>
#include <string>

#include "_string.h"
#include "peercast_rs.h"
#include "str.h"

namespace str {
std::string codepoint_to_utf8(uint32_t cp)
{
    pcrs_buf b;
    if (pcrs_str_codepoint_to_utf8(cp, &b) != 0)
        throw std::out_of_range("codepoint");
    std::string s(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return s;
}
}

static long g_bad = 0, g_cases = 0;

static std::string hex(const uint8_t* p, size_t n)
{
    std::string s;
    char b[4];
    for (size_t i = 0; i < n; i++) { snprintf(b, sizeof b, "%02x", p[i]); s += b; }
    return s;
}

// C++ の data と Rust の出力 (MAX_LEN - 1 バイト以下 + NUL) を比べる。
static void check(const char* name, const std::string& input, const String& s, pcrs_buf r)
{
    g_cases++;
    bool ok = r.len < String::MAX_LEN
        && memcmp(s.data, r.ptr, r.len) == 0
        && s.data[r.len] == '\0';
    if (!ok && g_bad++ < 20)
    {
        printf("  [違い] %s 入力=%s\n    C++ =%s\n    Rust=%s\n", name,
               hex(reinterpret_cast<const uint8_t*>(input.data()), input.size()).c_str(),
               hex(reinterpret_cast<const uint8_t*>(s.data), strnlen(s.data, String::MAX_LEN)).c_str(),
               hex(r.ptr, r.len).c_str());
    }
    pcrs_buf_free(r);
}

// 入力を 0 埋めしたバッファに置く (終端越えの読み出し先を 0 にする)
struct Padded
{
    char buf[4096];
    explicit Padded(const std::string& s)
    {
        memset(buf, 0, sizeof buf);
        memcpy(buf, s.data(), s.size());
    }
};

static void fresh(String& s)
{
    memset(s.data, 0x55, sizeof s.data); // 書かれなかった部分が分かるように
}

static void run(const std::string& in)
{
    Padded p(in);
    const uint8_t* u = reinterpret_cast<const uint8_t*>(p.buf);
    size_t n = in.size();
    String s;

    for (int safe = 0; safe < 2; safe++)
    {
        fresh(s); s.ASCII2ESC(p.buf, safe);
        check(safe ? "ASCII2ESC(safe)" : "ASCII2ESC", in, s, pcrs_string_ascii_to_esc(u, n, safe));
        fresh(s); s.ASCII2META(p.buf, safe);
        check(safe ? "ASCII2META(safe)" : "ASCII2META", in, s, pcrs_string_ascii_to_meta(u, n, safe));
        if (n < String::MAX_LEN) // C++ 版は data を越えて書きうるので、実際に来る長さだけ
        {
            fresh(s); s.UNKNOWN2UNICODE(p.buf, safe);
            check(safe ? "UNKNOWN2UNICODE(safe)" : "UNKNOWN2UNICODE", in, s,
                  pcrs_string_unknown_to_unicode(u, n, safe));
        }
    }
    fresh(s); s.ESC2ASCII(p.buf);
    check("ESC2ASCII", in, s, pcrs_string_esc_to_ascii(u, n));
    if (n < String::MAX_LEN)
    {
        fresh(s); s.BASE642ASCII(p.buf);
        check("BASE642ASCII", in, s, pcrs_string_base64_to_ascii(u, n));
    }
    fresh(s); s.setFromString(p.buf);
    check("setFromString", in, s, pcrs_string_from_string(u, n));
    fresh(s); s.setUnquote(p.buf);
    check("setUnquote", in, s, pcrs_string_unquote(u, n));

    if (n >= 4)
    {
        g_cases++;
        char a[3] = {0, 0, 0};
        uint8_t b[3] = {0, 0, 0};
        int ra = String::base64WordToChars(a, p.buf);
        int rb = pcrs_base64_word_to_chars(u, b);
        if ((ra != rb || memcmp(a, b, ra) != 0) && g_bad++ < 20)
            printf("  [違い] base64WordToChars 入力=%s C++=%d Rust=%d\n",
                   hex(u, 4).c_str(), ra, rb);
    }
}

int main(int argc, char** argv)
{
    long iterations = argc > 1 ? atol(argv[1]) : 300000;

    // 長さ 0〜2 の全バイト列 (NUL を除く)
    run("");
    for (int a = 1; a < 256; a++)
    {
        run(std::string(1, (char)a));
        for (int b = 1; b < 256; b++)
            run(std::string{(char)a, (char)b});
    }

    // 長さ 3〜4 は、意味のあるバイトに絞って全通り
    const std::string alpha = std::string("%+;\"' &<>=/Aa0zZ9") + "\x80\x81\x9f\xa1\xc3\xe0\xe3\xfc\xfe\xff";
    for (char a : alpha) for (char b : alpha) for (char c : alpha)
    {
        run(std::string{a, b, c});
        for (char d : alpha)
            run(std::string{a, b, c, d});
    }

    // 乱数 (長さ 0〜600、偏ったアルファベットと全バイトを混ぜる)
    std::mt19937 rng(20260923);
    for (long it = 0; it < iterations; it++)
    {
        size_t len = rng() % 2 ? rng() % 40 : rng() % 600;
        std::string in;
        for (size_t i = 0; i < len; i++)
        {
            char c = (rng() % 3) ? alpha[rng() % alpha.size()] : (char)(1 + rng() % 255);
            in += c;
        }
        run(in);
    }

    // stopwatch
    for (unsigned t : {0u, 1u, 59u, 60u, 61u, 3599u, 3600u, 86399u, 86400u, 90061u, 4294967295u})
    {
        String s;
        fresh(s); s.setFromStopwatch(t);
        check("setFromStopwatch", std::to_string(t), s, pcrs_string_from_stopwatch(t));
    }
    for (long it = 0; it < 100000; it++)
    {
        unsigned t = rng();
        String s;
        fresh(s); s.setFromStopwatch(t);
        check("setFromStopwatch", std::to_string(t), s, pcrs_string_from_stopwatch(t));
    }

    printf("比較件数 %ld、違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
