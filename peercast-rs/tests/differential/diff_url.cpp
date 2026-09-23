// URL の解析 (core/common/LUrlParser.cpp と URLSource::getSourceProtocol) の、C++ 版と Rust 版の差分テスト。
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) のもの。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>

#include "LUrlParser.h"
#include "peercast_rs.h"
#include "url.h"

static long g_bad = 0, g_cases = 0, g_known = 0;
static std::mt19937 rng(20260923);

static std::string show(const std::string& s)
{
    std::string r;
    char b[8];
    for (unsigned char c : s)
    {
        if (c >= 0x20 && c < 0x7f && c != '\\') r += (char)c;
        else { snprintf(b, sizeof b, "\\x%02x", c); r += b; }
    }
    return r;
}

static const uint8_t* u(const std::string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }

static void urlCase(const std::string& in)
{
    g_cases++;
    using LUrlParser::clParseURL;
    clParseURL c = clParseURL::ParseURL(in);
    std::string a = std::to_string(c.m_ErrorCode) + "|" + c.m_Scheme + "|" + c.m_Host + "|" + c.m_Port + "|" + c.m_Path +
                    "|" + c.m_Query + "|" + c.m_Fragment + "|" + c.m_UserName + "|" + c.m_Password;
    int cport = 0;
    bool cok = c.GetPort(&cport);

    pcrs_vec v{};
    int code = pcrs_url_parse(u(in), in.size(), &v);
    std::string b = std::to_string(code == 0 ? 0 : code);
    std::string rport;
    if (code == 0)
    {
        const char* p = reinterpret_cast<const char*>(v.joined.ptr);
        size_t off = 0;
        for (size_t i = 0; i < v.count; i++)
        {
            std::string part(p + off, v.lens[i]);
            off += v.lens[i];
            b += "|" + part;
            if (i == 2) rport = part;
        }
        pcrs_vec_free(v);
    }
    else
        b += "||||||||";

    if (a != b && g_bad++ < 20)
        printf("  [違い] ParseURL 入力=\"%s\"\n    C++ =%s\n    Rust=%s\n", show(in).c_str(), show(a).c_str(), show(b).c_str());

    if (c.IsValid())
    {
        g_cases++;
        int rp = pcrs_url_port_number(u(rport), rport.size());
        bool rok = rp != 0;
        if (cok != rok || (cok && cport != rp))
        {
            // atoi の桁あふれ (C++ 版は 2^32 を超える数が一周する) は既知の違い
            bool overflow = strtod(c.m_Port.c_str(), nullptr) > 2147483647.0 || strtod(c.m_Port.c_str(), nullptr) < -2147483648.0;
            if (overflow) g_known++;
            else if (g_bad++ < 20)
                printf("  [違い] GetPort 入力=\"%s\" C++=%d/%d Rust=%d\n", show(c.m_Port).c_str(), cok, cport, rp);
        }
    }

    g_cases++;
    std::string buf1 = in.c_str(), buf2 = buf1;
    char* f1 = &buf1[0];
    ChanInfo::PROTOCOL p1 = URLSource::getSourceProtocol(f1);
    size_t skip = 0;
    int p2 = pcrs_url_source_protocol(u(buf2), buf2.size(), &skip);
    if ((int)p1 != p2 || (size_t)(f1 - &buf1[0]) != skip)
        if (g_bad++ < 20)
            printf("  [違い] getSourceProtocol 入力=\"%s\" C++=%d/%zu Rust=%d/%zu\n", show(in).c_str(), (int)p1,
                   (size_t)(f1 - &buf1[0]), p2, skip);
}

int main(int argc, char** argv)
{
    long iterations = argc > 1 ? atol(argv[1]) : 500000;

    const char* samples[] = {
        "http://user:pw@example.com:8080/a/b?x=1#frag", "https://[::1]:7144/", "http://host", "HTTP://Host/Path",
        "pcp://1.2.3.4:7144/channel/0123456789ABCDEF", "rtmp://live/stream", "pipe:ffmpeg -i x", "file:///tmp/a.flv",
        "mms://x", "http://h:4294967376/", "http://h:99999999999/", "http://h:-1/", "http://a@b@c/d", "x:", "",
    };
    for (auto s : samples) urlCase(s);

    // 短い入力は、意味のある文字だけで全通り
    const std::string alpha = ":/@[]?#hH1.+-";
    for (char a : alpha) for (char b : alpha) for (char c : alpha) for (char d : alpha)
        urlCase(std::string{a, b, c, d});

    const std::string rich = ":/@[]?#httpHTTPspipe0123456789.+-: \x80\xff";
    for (long it = 0; it < iterations; it++)
    {
        std::string s;
        if (rng() % 2)
        {
            s = samples[rng() % (sizeof(samples) / sizeof(samples[0]))];
            int n = 1 + rng() % 4;
            for (int i = 0; i < n; i++)
            {
                size_t pos = s.empty() ? 0 : rng() % (s.size() + 1);
                switch (rng() % 4)
                {
                case 0: if (pos < s.size()) s.erase(pos, 1); break;
                case 1: s.insert(pos, 1, rich[rng() % rich.size()]); break;
                case 2: if (pos < s.size()) s[pos] = rich[rng() % rich.size()]; break;
                case 3: s.insert(pos, 1, (char)(rng() % 256)); break;
                }
            }
        }
        else
        {
            size_t len = rng() % 30;
            for (size_t i = 0; i < len; i++) s += rich[rng() % rich.size()];
        }
        urlCase(s);
    }

    printf("比較件数 %ld、既知の違い (ポート番号の桁あふれ) %ld、説明のつかない違い %ld\n", g_cases, g_known, g_bad);
    return g_bad ? 1 : 0;
}
