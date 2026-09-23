// 段階5b: PublicController::acceptableLanguages / formatUptime (core/common/public.cpp) と、
// コンソールの引数の解釈 parse_options (core/common/commands.cpp) の、C++ 版と Rust 版の差分テスト。
//
// parse_options は commands.cpp の static 関数で外から呼べないので、元のコードをこのファイルに
// そのまま写したものを C++ 版とする。
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <map>
#include <random>
#include <string>
#include <vector>

#include "common.h"
#include "public.h"
#include "rustbridge.h"
#include "str.h"
#include "../../../tests/mocksys.h"

static std::mt19937 rng(20260925);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }
static long g_cases = 0, g_bad = 0, g_knownNul = 0;

// ---------------------------------------------------------------- parse_options (commands.cpp から写したもの)

static std::pair< std::map<std::string, std::string>,
                  std::vector<std::string> >
cxx_parse_options(const std::vector<std::string>& args,
                  const std::vector<std::string>& option_names)
{
    std::map<std::string, std::string> options;
    std::vector<std::string> positionals;

    for (size_t i = 0; i < args.size(); i++) {
        if (args[i] == "--") { // end of options
            for (size_t j = i + 1; j < args.size(); j++) {
                positionals.push_back(args[j]);
            }
            break;
        } else if (args[i][0] == '-') {
            if (args[i][1] == '-') { // long option
                auto vec = str::split(args[i], "=", 2);
                if (std::find(option_names.begin(), option_names.end(), vec[0]) == option_names.end()) {
                    throw FormatException(str::format("Unknown long option: %s", vec[0].c_str()));
                } else if (vec.size() == 2) { // assignment
                    options[vec[0]] = vec[1];
                } else {
                    options[vec[0]] = "";
                }
            } else { // short option
                if (std::find(option_names.begin(), option_names.end(), args[i]) == option_names.end()) {
                    throw FormatException(str::format("Unknown short option: %s", args[i].c_str()));
                } else {
                    options[args[i]] = "";
                }
            }
        } else {
            positionals.push_back(args[i]);
        }
    }
    return { options, positionals };
}

static std::string show(const std::pair<std::map<std::string, std::string>, std::vector<std::string>>& r)
{
    std::string s = "{";
    for (auto& p : r.first) s += p.first + "=" + p.second + ";";
    s += "}[";
    for (auto& p : r.second) s += p + "|";
    return s + "]";
}

static std::string rust_parse_options(const std::vector<std::string>& args, const std::vector<std::string>& names)
{
    rustbridge::JoinedParts a(args), n(names);
    pcrs_vec out{};
    size_t num = 0;
    rustbridge::RustBuf err;
    int r = pcrs_commands_parse_options(a.data(), a.joined.size(), a.lens.data(), a.lens.size(),
                                        n.data(), n.joined.size(), n.lens.data(), n.lens.size(), &out, &num, err.out());
    if (r == -1)
        return "FormatException:" + err.str();
    if (r != 0)
        return "internal error";
    auto flat = rustbridge::takeVec(out);
    std::pair<std::map<std::string, std::string>, std::vector<std::string>> res;
    for (size_t i = 0; i < num; i++)
        res.first[flat[2 * i]] = flat[2 * i + 1];
    res.second.assign(flat.begin() + 2 * num, flat.end());
    return show(res);
}

static void testParseOptions()
{
    const std::vector<std::string> pieces = { "-", "--", "-n", "--help", "--count", "=", "x", "", "a=b", "--count=", "-z", "--zz", std::string("-\0n", 3), "\xe3\x81\x82" };
    const std::vector<std::string> names = { "--help", "-n", "--count", "-" };
    for (int i = 0; i < 300000; i++)
    {
        std::vector<std::string> args;
        int k = rnd(6);
        for (int j = 0; j < k; j++)
        {
            std::string s;
            int m = 1 + rnd(3);
            for (int t = 0; t < m; t++)
                s += pieces[rnd((int) pieces.size())];
            args.push_back(rnd(4) ? pieces[rnd((int) pieces.size())] : s);
        }
        std::string a;
        try { a = show(cxx_parse_options(args, names)); }
        catch (FormatException& e) { a = std::string("FormatException:") + e.msg; }
        std::string b = rust_parse_options(args, names);
        g_cases++;
        bool nul = false;
        for (auto& s : args) nul |= s.find('\0') != std::string::npos;
        // C++ 版の str::split は NUL で切れる (段階1 の既知の違い。Rust 版は切れない)
        if (a != b && nul)
        {
            g_knownNul++;
            continue;
        }
        if (a != b && ++g_bad <= 5)
            printf("parse_options: C++ %s / Rust %s\n", a.c_str(), b.c_str());
    }
}

// ---------------------------------------------------------------- PublicController

static std::string join(const std::vector<std::string>& v)
{
    std::string s;
    for (auto& x : v) s += x + "|";
    return s;
}

static std::string someQ()
{
    const std::vector<std::string> qs = { "1", "0.5", "0.8", "0", "0.123456789", "1e-1", ".5", "5.", "0x1p-1", "0X.8", "inf", "-INFINITY",
                                          "nan", "nan(x)", "-1", "1e400", "", "  0.3", "0.3abc", "q", "0x", "0xg", "+0.25" };
    return qs[rnd((int) qs.size())];
}

static void testAcceptLanguage()
{
    const std::vector<std::string> tags = { "ja", "en", "en-US", "fr", "*", "", " de", "zh-Hant-TW" };
    for (int i = 0; i < 300000; i++)
    {
        std::string h;
        if (rnd(10))
        {
            // 16 個以下: C++ 版の std::sort は挿入ソートになり、順序が決まる (NaN も比べられる)
            int n = rnd(17);
            for (int j = 0; j < n; j++)
            {
                if (j) h += rnd(10) ? "," : ", ";
                h += tags[rnd((int) tags.size())];
                if (rnd(3))
                    h += std::string(rnd(5) ? ";q=" : rnd(2) ? "; q=" : ";") + someQ();
                if (rnd(20) == 0) h += ";x=1";
            }
            if (rnd(30) == 0) h += std::string(1, '\0') + "zz";
        }
        else
        {
            // 17 個以上: C++ 版は同じ q 値の順序が決まらず、NaN では配列の外を読むことがあるので、
            // q 値がすべて違うものだけを比べる
            int n = 17 + rnd(40);
            std::vector<int> q(n);
            for (int j = 0; j < n; j++) q[j] = j;
            std::shuffle(q.begin(), q.end(), rng);
            for (int j = 0; j < n; j++)
            {
                char b[32];
                snprintf(b, sizeof b, "%st%d;q=0.%03d", j ? "," : "", j, q[j]);
                h += b;
            }
        }
        std::string a = join(PublicController::acceptableLanguages(h));
        std::string b = join(rustbridge::takeVec(pcrs_public_acceptable_languages(reinterpret_cast<const uint8_t*>(h.data()), h.size())));
        g_cases++;
        // C++ 版の str::split は NUL で切れる (段階1 の既知の違い)
        if (a != b && h.find('\0') != std::string::npos)
        {
            g_knownNul++;
            continue;
        }
        if (a != b && ++g_bad <= 5)
            printf("acceptableLanguages(%s): C++ %s / Rust %s\n", h.c_str(), a.c_str(), b.c_str());
    }
}

static void testUptime()
{
    std::vector<unsigned> edge = { 0, 1, 59, 60, 3599, 3600, 359999, 360000, 0x7fffffffu, 0x80000000u, 0xffffffffu };
    for (int i = 0; i < 200000; i++)
        edge.push_back(i < 100000 ? (unsigned) i * 37 : (unsigned) rng());
    for (unsigned t : edge)
    {
        std::string a = PublicController::formatUptime(t);
        std::string b = rustbridge::RustBuf(pcrs_public_format_uptime(t)).str();
        g_cases++;
        if (a != b && ++g_bad <= 5)
            printf("formatUptime(%u): C++ %s / Rust %s\n", t, a.c_str(), b.c_str());
    }
}

int main()
{
    sys = new MockSys();
    testParseOptions();
    testAcceptLanguage();
    testUptime();
    printf("比較件数 %ld、既知の違い (str::split の NUL、段階1) %ld、説明のつかない違い %ld\n", g_cases, g_knownNul, g_bad);
    return g_bad ? 1 : 0;
}
