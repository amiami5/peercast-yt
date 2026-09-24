// 段階9a: 正規表現 (core/common/regexp.cpp の Regexp。C++ 版は std::regex の ECMAScript) の、C++ 版と
// Rust 版 (src/server/regex.rs) の差分テスト。
//
// C++ 版で実際に使っている正規表現 (ini、ホスト名、フィルター、日付、テンプレートの UI の条件など)
// には、それらしい入力とその変異を与える。ほかに、記号を組み合わせた乱数の正規表現と入力を与え、
// 正規表現の誤りかどうか、一致するか、各グループの文字列を比べる。
#include <cstdio>
#include <cstdlib>
#include <random>
#include <regex>
#include <string>
#include <vector>

#include "peercast_rs.h"
#include "regexp.h"
#include "rustbridge.h"

static std::mt19937 rng(20260929);
static unsigned R(unsigned n) { return rng() % n; }
static long g_cases = 0, g_bad = 0, g_errors = 0, g_matches = 0;

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

static std::string cxx(const std::string& pattern, const std::string& subject)
{
    try {
        Regexp r(pattern);
        auto v = r.exec(subject);
        if (v.empty())
            return "0";
        std::string s = "1";
        for (auto& g : v) s += "|" + g;
        return s;
    } catch (std::regex_error&) {
        return "E";
    }
}

static std::string rust(const std::string& pattern, const std::string& subject)
{
    pcrs_vec out;
    int r = pcrs_regex_exec(reinterpret_cast<const uint8_t*>(pattern.data()), pattern.size(),
                            reinterpret_cast<const uint8_t*>(subject.data()), subject.size(), &out);
    auto v = rustbridge::takeVec(out);
    if (r < 0) return "E";
    if (r == 0) return "0";
    std::string s = "1";
    for (auto& g : v) s += "|" + g;
    return s;
}

static void compare(const std::string& p, const std::string& s)
{
    if (getenv("DR_TRACE")) { fprintf(stderr, "%s | %s\n", show(p).c_str(), show(s).c_str()); }
    std::string a = cxx(p, s), b = rust(p, s);
    g_cases++;
    if (a == "E") g_errors++;
    if (a[0] == '1') g_matches++;
    if (a != b && g_bad++ < 30)
        printf("  [違い] /%s/ %s\n    C++ =%s\n    Rust=%s\n", show(p).c_str(), show(s).c_str(), show(a).c_str(), show(b).c_str());
}

#define IPV6 "(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|[fF][eE]80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::([fF][fF][fF][fF](:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))"

// C++ 版のコードが使っている正規表現と、その見本の入力
static const std::vector<std::pair<std::string, std::vector<std::string>>> g_real = {
    { "^\\s*\\[(\\w+)\\]\\s*$", { "[Server]", " [End] ", "[a b]", "[]", "[Server1]x" } },
    { "\\s*(\\w+)\\s*=\\s*(.*)$", { "serverPort = 7144", "a=", " x = y = z", "=1", "name" } },
    { "^\\d+\\.\\d+\\.\\d+\\.\\d+$", { "127.0.0.1", "1.2.3", "1.2.3.4.5", "a.b.c.d" } },
    { "^\\d+\\.\\d+\\.\\d+\\.\\d+:\\d+$", { "127.0.0.1:7144", "1.2.3.4:", "1.2.3.4:x" } },
    { "^\\d+\\.\\d+\\.\\d+\\.\\d+/\\d+$", { "10.0.0.0/8", "10.0.0.0/", "10.0.0/8" } },
    { "^" IPV6 "$", { "::1", "::", "2001:db8::1", "fe80::1%eth0", "::ffff:1.2.3.4", "1:2:3:4:5:6:7:8", "1.2.3.4", ":::", "1::2::3" } },
    { "^\\[" IPV6 "\\]:\\d+$", { "[::1]:7144", "[::1]", "[2001:db8::1]:80" } },
    { "^" IPV6 "/\\d+$", { "::/0", "2001:db8::/32", "::1/" } },
    { "^([A-z]{3}), (\\d+) ([A-z]{3}) (\\d+) (\\d+):(\\d+):(\\d+) (GMT|UTC)$", { "Sun, 06 Nov 1994 08:49:37 GMT", "Sun, 6 Nov 1994 8:49:37 UTC", "Sun 06 Nov" } },
    { "^([A-z]+)day, (\\d+)-([A-z]{3})-(\\d{2}) (\\d+):(\\d+):(\\d+) (GMT|UTC)$", { "Sunday, 06-Nov-94 08:49:37 GMT", "Sunday, 06-Nov-1994 08:49:37 GMT" } },
    { "^([A-z]{3}) ([A-z]{3}) +(\\d+) (\\d+):(\\d+):(\\d+) (\\d+)$", { "Sun Nov  6 08:49:37 1994", "Sun Nov 6 08:49:37 1994" } },
    { "/[^/]*$", { "http://yp/index.txt", "index.txt", "a/b/" } },
    { "^PeerCastStation/([0-9.]+)$", { "PeerCastStation/2.9.1", "PeerCastStation/", "PeerCastStation/1.0 x" } },
    { "^PeerCast/0.1218 \\(YT(\\d+)\\)$", { "PeerCast/0.1218 (YT33)", "PeerCast/0.1218(YT33)", "PeerCast/0x1218 (YT1)" } },
    { "^PeerCast/0.1218\\(IM(\\d+)\\)$", { "PeerCast/0.1218(IM0051)" } },
    { "^https?://[a-zA-z\\-\\.]+(?::\\d+)?/test/read\\.cgi\\/(\\w+)/(\\d+)/?$", { "http://jbbs.example/test/read.cgi/game/123/", "https://a.b:8080/test/read.cgi/x/1", "http://a/test/read.cgi/x/y" } },
    { "/index.html$", { "/html/ja/index.html", "/index.html?x", "" } },
    { "/console.html", { "/html/en/console.html", "/console.htm" } },
};

static std::string mutate(std::string s)
{
    static const char* pieces[] = { "", "a", "1", ":", ".", "/", " ", "[", "]", "::", "%", "x", "\n", "\xff", "0", "GMT" };
    int n = 1 + R(3);
    for (int i = 0; i < n; i++)
    {
        size_t pos = s.empty() ? 0 : R(s.size() + 1);
        switch (R(3))
        {
        case 0: s.insert(pos, pieces[R(sizeof pieces / sizeof pieces[0])]); break;
        case 1: if (pos < s.size()) s.erase(pos, 1 + R(3)); break;
        default: if (pos < s.size()) s[pos] = "a1:.[]/ x"[R(9)]; break;
        }
    }
    return s;
}

static std::string randPattern()
{
    static const char* toks[] = { "a", "b", "c", ".", "^", "$", "*", "+", "?", "*?", "+?", "??", "{2}", "{1,2}", "{0,}", "{2,1}",
                                  "(", ")", "(?:", "(?=", "(?!", "|", "[ab]", "[^a]", "[a-c]", "[c-a]", "[", "]", "\\d", "\\w",
                                  "\\s", "\\D", "\\W", "\\S", "\\b", "\\B", "\\.", "\\1", "\\2", "\\\\", "-", ",", "{", "}",
                                  "[\\d-]", "[\\w.]", "[.]", "\\x41", "\\t", "(a)", "(b|c)", "(a*)", "(a|ab)", "x", ":" };
    // 量指定子を重ねたもの (a*+ など) は、libstdc++ が終わらなくなることがあるので作らない
    // (Rust 版は重ねたものも受け付ける。src/server/regex.rs の単体テスト)
    auto isQuant = [](const std::string& t) { return t[0] == '*' || t[0] == '+' || t[0] == '?' || (t[0] == '{' && t.size() > 1); };
    std::string p, prev;
    int n = R(8);
    for (int i = 0; i < n; i++)
    {
        std::string t = toks[R(sizeof toks / sizeof toks[0])];
        if (!prev.empty() && isQuant(prev) && isQuant(t))
            continue;
        p += t;
        prev = t;
    }
    return p;
}

static std::string randSubject()
{
    static const char alpha[] = "abcab1 .-:\t_A";
    std::string s;
    int n = R(10);
    for (int i = 0; i < n; i++) s += alpha[R(sizeof alpha - 1)];
    return s;
}

int main()
{
    for (auto& r : g_real)
        for (auto& s : r.second)
        {
            compare(r.first, s);
            for (int k = 0; k < 2000; k++)
                compare(r.first, mutate(s));
        }
    for (int i = 0; i < 300000; i++)
        compare(randPattern(), randSubject());
    printf("比較件数 %ld (C++ 版が誤りとしたもの %ld、一致したもの %ld)、説明のつかない違い %ld\n", g_cases, g_errors, g_matches, g_bad);
    return g_bad ? 1 : 0;
}
