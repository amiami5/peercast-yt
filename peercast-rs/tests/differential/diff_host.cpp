// 段階9a: IP アドレスとホスト (core/common/ip.h の IP、host.cpp の Host)、フィルター (servfilter.cpp の
// ServFilter) の、C++ 版と Rust 版 (src/server/host.rs、servfilter.rs) の差分テスト。
//
// 名前を引く (DNS) ものは結果が環境によるので比べない: Host::fromStrName で名前を引く入力と、
// ServFilter のホスト名と名前の終わりのパターンの matches。
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>
#include <vector>

#include "host.h"
#include "ip.h"
#include "peercast_rs.h"
#include "regexp.h"
#include "rustbridge.h"
#include "servfilter.h"
#include "str.h"
#include "usys.h"

static std::mt19937 rng(20260930);
static unsigned R(unsigned n) { return rng() % n; }
static long g_cases = 0, g_bad = 0, g_dns = 0, g_throw = 0, g_atoi = 0;

extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char*, ...) {}

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

static void report(const char* what, const std::string& in, const std::string& a, const std::string& b)
{
    g_cases++;
    if (a != b && g_bad++ < 30)
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", what, show(in).c_str(), show(a).c_str(), show(b).c_str());
}

// atoi の値が int に収まらないか (C++ 版は glibc の atoi で切り詰め、Rust 版は端の値にする。段階3a)
static bool overflowsInt(const std::string& s, size_t from)
{
    if (from >= s.size()) return false;
    long v = strtol(s.c_str() + from, nullptr, 10);
    return v > INT_MAX || v < INT_MIN;
}

static const uint8_t* u8(const std::string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }

static std::string randStr()
{
    static const char* toks[] = { "1", "2", "12", "123", "255", "256", "0", "00", "01", "999", "1234", ".", ":", "::", "[", "]", "/",
                                  "a", "f", "ff", "ffff", "FFFF", "db8", "2001", "fe80", "%eth0", "-1", "+1", " ", "x", "8",
                                  "32", "64", "128", "129", "7144", "65535", "65536", "localhost", "::ffff:", "1.2.3.4" };
    std::string s;
    int n = R(9);
    for (int i = 0; i < n; i++) s += toks[R(sizeof toks / sizeof toks[0])];
    return s;
}

static std::string ipStr(const IP& ip)
{
    try { return ip.str(); } catch (std::exception& e) { return std::string("E:") + e.what(); }
}

static std::string rustIpStr(const uint8_t* a)
{
    return rustbridge::RustBuf(pcrs_ip_str(a)).str();
}

static void testIp(const std::string& s)
{
    IP ip;
    bool a = IP::tryParse(s, ip);
    uint8_t out[16] = {};
    bool b = pcrs_ip_parse(u8(s), s.size(), out);
    std::string sa = a ? ipStr(ip) : "-", sb = b ? rustIpStr(out) : "-";
    report("IP::tryParse", s, sa, sb);
}

// fromStrName が名前を引くか (C++ 版と同じ順に調べる)
static bool needsDns(const std::string& s)
{
    static const Regexp v4("^\\d+\\.\\d+\\.\\d+\\.\\d+$"), v4p("^\\d+\\.\\d+\\.\\d+\\.\\d+:\\d+$");
    if (s.empty() || v4.matches(s) || v4p.matches(s))
        return false;
    auto v = str::split(s, ":");
    if (v.size() > 2)
        return false;
    IP ip;
    // IPv6 の正規表現に合うものは tryParse だけ
    if (s.find(':') != std::string::npos && v.size() > 2) return false;
    return !IP::tryParse(v[0], ip);
}

static void testHost(const std::string& s)
{
    uint8_t ip[16];
    uint16_t port;
    {
        Host h;
        h.fromStrIP(s.c_str(), 7144);
        pcrs_host_from_str(u8(s), s.size(), 7144, false, ip, &port);
        IP rip; memcpy(rip.addr, ip, 16);
        report("Host::fromStrIP", s, h.str(), Host(rip, port).str());
    }
    if (s.find('[') == std::string::npos && needsDns(s))
    {
        g_dns++;
        return;
    }
    if (s.find('[') != std::string::npos)
    {
        // [ で始まる IPv6 の書き方でなければ、split(":") で名前を引くことがある
        static const Regexp bracket("^\\[.*\\](:\\d+)?$");
        if (!bracket.matches(s) && needsDns(s)) { g_dns++; return; }
    }
    Host h;
    try {
        h.fromStrName(s.c_str(), 7144);
    } catch (FormatException& e) {
        g_throw++;
        return;
    } catch (std::exception& e) {
        g_dns++;
        return;
    }
    pcrs_host_from_str(u8(s), s.size(), 7144, true, ip, &port);
    IP rip; memcpy(rip.addr, ip, 16);
    if (rip == h.ip && port != h.port && overflowsInt(s, s.rfind(':') + 1))
    {
        g_atoi++;
        return;
    }
    report("Host::fromStrName", s, h.str(), Host(rip, port).str());
}

static void testFilter(const std::string& pattern)
{
    ServFilter f;
    try {
        f.setPattern(pattern.c_str());
    } catch (std::exception&) {
        // C++ 版は IPv6 の正規表現に合うが inet_pton で読めないもの (%scope など) で例外を投げる。
        // Rust 版は :: として続ける
        g_throw++;
        return;
    }
    unsigned flags = R(16);
    f.flags = flags;
    // 比べるホスト
    IP ip;
    std::string hs;
    switch (R(4))
    {
    case 0: hs = str::format("%u.%u.%u.%u", R(3) ? 10 : R(256), R(256), R(256), R(256)); break;
    case 1: hs = pattern.substr(0, pattern.find('/')); break;
    case 2: hs = str::format("2001:db8:%x::%x", R(3), R(3)); break;
    default: hs = R(2) ? "::1" : "127.0.0.1"; break;
    }
    if (!IP::tryParse(hs, ip)) ip = IP(0x0a000001);
    Host h(ip, 7144);
    unsigned fl = 1u << R(4);

    std::string a, b;
    try {
        a = f.getPattern();
    } catch (std::exception& e) {
        a = "E";
    }
    rustbridge::RustBuf out;
    bool global = false, set = false;
    bool m = pcrs_servfilter_probe(u8(pattern), pattern.size(), flags, ip.addr, 7144, fl, out.out(), &global, &set);
    b = out.str();
    if (a != b && overflowsInt(pattern, pattern.rfind('/') + 1))
    {
        g_atoi++;
        return;
    }
    report("ServFilter::getPattern", pattern, a, b);
    report("ServFilter::isGlobal", pattern, std::to_string(f.isGlobal()), std::to_string(global));
    report("ServFilter::isSet", pattern, std::to_string(f.isSet()), std::to_string(set));
    if (f.type == ServFilter::T_HOSTNAME || f.type == ServFilter::T_SUFFIX)
    {
        g_dns++;
        return;
    }
    bool cm;
    try { cm = f.matches(fl, h); } catch (std::exception&) { return; }
    // /0 は Rust 版で直した (C++ 版は x86 でずらさないので、同じアドレスにしか一致しない)
    if (f.type == ServFilter::T_IPV4_WITH_NETMASK && str::has_suffix(a, "/0") && cm != m && m)
        return;
    report("ServFilter::matches", pattern + " " + hs, std::to_string(cm), std::to_string(m));
}

int main()
{
    sys = new USys();
    std::vector<std::string> fixed = { "", "127.0.0.1", "::1", "[::1]:7144", "[::1]", "1.2.3.4:80", "::", "::ffff:1.2.3.4",
                                       "2001:db8::1", "fe80::1%eth0", "255.255.255.255", "0.0.0.0/0", "::/0", "10.0.0.0/8",
                                       "2001:db8::/32", "1.2.3.4/33", "::/129", ".example.jp", "01.2.3.4", "1.2.3.4.5" };
    for (auto& s : fixed)
    {
        testIp(s);
        testHost(s);
        for (int i = 0; i < 20; i++) testFilter(s);
    }
    for (int i = 0; i < 200000; i++)
    {
        std::string s = randStr();
        testIp(s);
        testHost(s);
        testFilter(s);
    }
    printf("比較件数 %ld (名前を引くので比べなかったもの %ld)、既知の違い (C++ 版が例外を投げたもの %ld、atoi の桁あふれ %ld)、"
           "説明のつかない違い %ld\n", g_cases, g_dns, g_throw, g_atoi, g_bad);
    return g_bad ? 1 : 0;
}
