// HTTP の行の解析 (core/common/http.cpp) と cgi::parseHttpDate の、C++ 版と Rust 版の差分テスト。
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の HTTP クラスそのもの。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <stdexcept>
#include <string>

#include "cgi.h"
#include "http.h"
#include "peercast_rs.h"
#include "sstream.h"
#include "sys.h"

static long g_bad = 0, g_cases = 0, g_known = 0;

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

static void bad(const char* what, const std::string& in, const std::string& cxx, const std::string& rs)
{
    if (g_bad++ < 20)
        printf("  [違い] %s 入力=\"%s\"\n    C++ =%s\n    Rust=%s\n", what, show(in).c_str(), show(cxx).c_str(), show(rs).c_str());
}

static std::string take(pcrs_buf b)
{
    std::string s(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return s;
}

static const uint8_t* u(const std::string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }

// readLine が返しうる行にする (\r と \n を含まない。NUL は含みうる)
static std::string asLine(const std::string& s)
{
    std::string r;
    for (char c : s)
        if (c != '\r' && c != '\n')
            r += c;
    return r;
}

static void statusLine(const std::string& rawLine)
{
    const std::string line = asLine(rawLine);
    g_cases++;
    StringStream mem;
    mem.str(line + "\n");
    HTTP http(mem);
    int a = http.readResponse();
    std::string cxxLine = http.cmdLine;

    // Rust 版に渡すのは、C++ 版の readLine が cmdLine に入れた内容 (NUL の手前まで)
    std::string cline = line.substr(0, strnlen(line.c_str(), line.size()));
    size_t cut = 0;
    int b = pcrs_http_parse_status_line(u(cline), cline.size(), &cut);
    std::string rsLine = cline.substr(0, cut);
    if (a != b || cxxLine != rsLine)
    {
        // atoi の桁あふれは既知の違い (C++ は未定義、Rust は丸める)
        if (cxxLine == rsLine && std::string(cline).find_first_of("0123456789") != std::string::npos
            && (b == INT32_MAX || b == INT32_MIN))
        { g_known++; return; }
        bad("readResponse", line, std::to_string(a) + " / " + cxxLine, std::to_string(b) + " / " + rsLine);
    }
}

static void headerLine(const std::string& rawLine)
{
    const std::string line = asLine(rawLine);
    if (line.empty())
        return; // 空行はヘッダーの終わり
    g_cases++;
    StringStream mem;
    mem.str(line + "\n");
    HTTP http(mem);
    http.nextHeader();
    std::string cxxArg = http.arg ? std::to_string(http.arg - http.cmdLine) : "null";
    std::string cxxHeaders;
    for (const auto& p : http.headers)
        cxxHeaders += p.first + "=" + p.second + ";";

    std::string cline = line.substr(0, strnlen(line.c_str(), line.size()));
    size_t off = 0;
    pcrs_buf name = {nullptr, 0}, value = {nullptr, 0};
    std::string rsArg = "null", rsHeaders;
    if (pcrs_http_parse_header_line(u(cline), cline.size(), &off, &name, &value))
    {
        rsArg = std::to_string(off);
        HTTPHeaders h;
        h.set(take(name), take(value));
        for (const auto& p : h)
            rsHeaders += p.first + "=" + p.second + ";";
    }
    if (cxxArg != rsArg || cxxHeaders != rsHeaders)
        bad("nextHeader", line, cxxArg + " " + cxxHeaders, rsArg + " " + rsHeaders);
}

static void basicAuth(const std::string& arg)
{
    g_cases++;
    char u1[64] = "", p1[64] = "";
    HTTP::parseAuthorizationHeader(arg.c_str(), u1, p1, sizeof u1, sizeof p1);

    std::string carg = arg.c_str();
    std::string u2, p2;
    pcrs_buf ub = {nullptr, 0}, pb = {nullptr, 0};
    if (pcrs_http_parse_basic_auth(u(carg), carg.size(), &ub, &pb))
    {
        u2 = take(ub).substr(0, 63);
        p2 = take(pb).substr(0, 63);
        u2 = u2.c_str(); // C++ 側で char[] に写したときと同じく、NUL の手前まで
        p2 = p2.c_str();
    }
    if (u1 != u2 || p1 != p2)
        bad("parseAuthorizationHeader", arg, std::string(u1) + " : " + p1, u2 + " : " + p2);
}

static void crossOrigin(const std::string& site, const std::string& origin, const std::string& host)
{
    g_cases++;
    bool a = HTTP::isCrossOriginRequest(site, origin, host);
    bool b = pcrs_http_is_cross_origin_request(u(site), site.size(), u(origin), origin.size(), u(host), host.size());
    if (a != b)
        bad("isCrossOriginRequest", site + " | " + origin + " | " + host, a ? "true" : "false", b ? "true" : "false");
}

static void loopback(const std::string& host)
{
    g_cases++;
    bool a = HTTP::isLoopbackHostHeader(host);
    bool b = pcrs_http_is_loopback_host_header(u(host), host.size());
    if (a != b)
        bad("isLoopbackHostHeader", host, a ? "true" : "false", b ? "true" : "false");
}

static void httpDate(const std::string& s)
{
    g_cases++;
    long long a;
    bool threw = false;
    try { a = cgi::parseHttpDate(s); }
    catch (std::out_of_range&) { a = -1; threw = true; }
    long long b = pcrs_cgi_parse_http_date(u(s), s.size());
    if (a != b)
        bad("parseHttpDate", s, std::to_string(a), std::to_string(b));
    else if (threw)
        g_known++; // C++ 版は例外、Rust 版は -1 (既知の違い)
}

static std::mt19937 rng(20260923);

static std::string mutate(std::string s, const std::string& alpha)
{
    int n = 1 + rng() % 3;
    for (int i = 0; i < n; i++)
    {
        size_t pos = s.empty() ? 0 : rng() % (s.size() + 1);
        switch (rng() % 4)
        {
        case 0: if (!s.empty() && pos < s.size()) s.erase(pos, 1); break;
        case 1: s.insert(pos, 1, alpha[rng() % alpha.size()]); break;
        case 2: if (pos < s.size()) s[pos] = alpha[rng() % alpha.size()]; break;
        case 3: s.insert(pos, 1, (char)(1 + rng() % 255)); break;
        }
    }
    return s;
}

static std::string randomLine(const std::string& alpha, size_t maxlen)
{
    std::string s;
    size_t len = rng() % maxlen;
    for (size_t i = 0; i < len; i++)
        s += (rng() % 4) ? alpha[rng() % alpha.size()] : (char)(rng() % 256);
    return s;
}

int main(int argc, char** argv)
{
    long iterations = argc > 1 ? atol(argv[1]) : 200000;

    const std::string lineAlpha = "HTTP/1.0 200 OK:Host-xX \t+-9";
    const std::string authAlpha = "Basic dXNlcjpwYXNz=+/: bAsIc";
    const std::string hostAlpha = "[]:.0123456789localhostLOCALHOST/";
    const std::string dateAlpha = "SunMonTueWedThuFriSatJanNovDec, -:0123456789 GMTUTCday";

    const char* statusSamples[] = {"HTTP/1.0 200 OK", "HTTP/1.1   404 Not Found", "", "X", " 200", "HTTP/1.0 -5", "ICY 200 OK", "HTTP/1.0 99999999999"};
    const char* headerSamples[] = {"Host: localhost", "Content-Length:10", "X:", ":", "no colon", "a:  b  ", "A:B:C"};
    const char* authSamples[] = {"Basic dXNlcjpwYXNz", "basic dXNlcjpwYXNz", "Basic", "Basic ", "xBasicy dXNlcjpwYXNz", "Basic dXNlcg==", "Basic OnBhc3M="};
    const char* dateSamples[] = {"Sun, 06 Nov 1994 08:49:37 GMT", "Sunday, 06-Nov-94 08:49:37 GMT", "Sun Nov  6 08:49:37 1994",
                                 "Tuesday, 08-Nov-94 08:49:37 UTC", "Fri, 31 Feb 2021 25:61:61 UTC", "Sun, 99999999999 Nov 1994 08:49:37 GMT"};

    for (auto s : statusSamples) statusLine(s);
    for (auto s : headerSamples) headerLine(s);
    for (auto s : authSamples) basicAuth(s);
    for (auto s : dateSamples) httpDate(s);

    for (long it = 0; it < iterations; it++)
    {
        statusLine(rng() % 2 ? randomLine(lineAlpha, 40) : mutate(statusSamples[rng() % 8], lineAlpha));
        headerLine(rng() % 2 ? randomLine(lineAlpha, 40) : mutate(headerSamples[rng() % 7], lineAlpha));
        basicAuth(rng() % 2 ? randomLine(authAlpha, 400) : mutate(authSamples[rng() % 7], authAlpha));
        crossOrigin(randomLine("same-originonecross-siteNONE", 16), randomLine("http://localhost:7144null", 30), randomLine("localhost:7144LOCAL", 20));
        loopback(randomLine(hostAlpha, 20));
        httpDate(rng() % 2 ? randomLine(dateAlpha, 40) : mutate(dateSamples[rng() % 6], dateAlpha));
    }

    printf("比較件数 %ld、既知の違い %ld、説明のつかない違い %ld\n", g_cases, g_known, g_bad);
    return g_bad ? 1 : 0;
}
