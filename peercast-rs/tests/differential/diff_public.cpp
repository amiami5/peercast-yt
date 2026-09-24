// 段階5b: PublicController::acceptableLanguages / formatUptime (core/common/public.cpp) と、
// コンソールの引数の解釈 parse_options (core/common/commands.cpp) の、C++ 版と Rust 版の差分テスト。
// 段階8c: FileSystemMapper (mapper.cpp)、public.cpp と assets.cpp の MIMEType と振り分け、
// If-Modified-Since の判断、HTTPRequest の URL の分割。
//
// parse_options と MIMEType は static 関数で外から呼べないので、元のコードをこのファイルに
// そのまま写したものを C++ 版とする。
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <unistd.h>
#include <map>
#include <random>
#include <string>
#include <vector>

#include "cgi.h"
#include "common.h"
#include "http.h"
#include "mapper.h"
#include "public.h"
#include "rustbridge.h"
#include "str.h"
#include "../../../tests/mocksys.h"

static std::mt19937 rng(20260925);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }
static long g_cases = 0, g_bad = 0, g_knownNul = 0, g_knownSibling = 0;

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

// ---------------------------------------------------------------- FileSystemMapper (段階8c)

static std::string rust_toLocalFilePath(const FileSystemMapper& m, const std::string& vpath, const std::vector<std::string>& langs)
{
    // mapper.cpp の WITH_RUST_CORE のときの toLocalFilePath と resolvePath を写したもの
    auto u = [](const std::string& x) { return reinterpret_cast<const uint8_t*>(x.data()); };
    rustbridge::RustBuf fp;
    if (!pcrs_mapper_local_path(u(m.virtualPath), m.virtualPath.size(), u(m.documentRoot), m.documentRoot.size(),
                                u(vpath), vpath.size(), fp.out()))
        return "|";
    auto filePath = fp.str();
    rustbridge::JoinedParts l(langs);
    auto cands = rustbridge::takeVec(pcrs_mapper_candidates(u(filePath), filePath.size(), l.data(), l.joined.size(),
                                                            l.lens.data(), l.lens.size()));
    std::string resolved, lang;
    for (size_t i = 0; i + 1 < cands.size(); i += 2)
    {
        auto r = FileSystemMapper::realPath(cands[i]);
        if (r != "") { resolved = r; lang = cands[i + 1]; break; }
    }
    if (resolved == "")
        return "|";
    if (!pcrs_mapper_inside(u(m.documentRoot), m.documentRoot.size(), u(resolved), resolved.size()))
        return "|";
    return resolved + "|" + lang;
}

static void testMapper()
{
    char tmpl[] = "/tmp/diff_public_XXXXXX";
    std::string base = mkdtemp(tmpl);
    auto sh = [](const std::string& c) { if (system(c.c_str()) != 0) { printf("失敗: %s\n", c.c_str()); exit(2); } };
    sh("cd " + base + " && mkdir -p public/sub public2 other && touch public/a.html public/a.html.ja public/b.txt.en "
       "public/sub/c.css public/index.html public/d.fr public2/secret other/x && ln -s ../public2 public/link && "
       "ln -s .. public/up && ln -s ../public2/secret public/s2");
    FileSystemMapper m("/public", base + "/public");
    const std::vector<std::string> toks = { "/public", "/public/", "/publicx", "/", "a.html", "b.txt", "d", "index.html",
                                            "sub", "c.css", "..", ".", "link", "up", "s2", "secret", "public2", "other",
                                            "x", "//", "%2e", "", std::string("a\0b", 3), "\xe3\x81\x82" };
    const std::vector<std::string> langTok = { "ja", "fr", "en", "", "../x", "html", "..", "/a" };
    for (int i = 0; i < 200000; i++)
    {
        std::string v;
        int n = rnd(6);
        for (int j = 0; j < n; j++)
        {
            if (j && rnd(3)) v += "/";
            v += toks[rnd((int) toks.size())];
        }
        if (rnd(3)) v = "/public/" + v;
        std::vector<std::string> langs;
        for (int j = rnd(4); j > 0; j--) langs.push_back(langTok[rnd((int) langTok.size())]);

        auto r = m.toLocalFilePath(v, langs);
        std::string a = r.first + "|" + r.second;
        std::string b = rust_toLocalFilePath(m, v, langs);
        g_cases++;
        // C++ 版は先頭が一致するかだけを見るので、隣のディレクトリ (public2) の中も通す (Rust 版で直した)
        if (a != b && b == "|" && r.first.compare(0, m.documentRoot.size(), m.documentRoot) == 0 &&
            r.first.size() > m.documentRoot.size() && r.first[m.documentRoot.size()] != '/')
        {
            g_knownSibling++;
            continue;
        }
        if (a != b && ++g_bad <= 5)
            printf("toLocalFilePath(%s): C++ %s / Rust %s\n", v.c_str(), a.c_str(), b.c_str());
    }
    sh("rm -rf " + base);
}

// ---------------------------------------------------------------- MIMEType、振り分け、304、URL (段階8c)

// public.cpp の MIMEType を写したもの
static std::string cxx_public_mime(const std::string& path)
{
    using namespace str;

    if (contains(path, ".htm"))
    {
        return MIME_HTML;
    }else if (contains(path, ".css"))
    {
        return MIME_CSS;
    }else if (contains(path, ".jpg"))
    {
        return MIME_JPEG;
    }else if (contains(path, ".gif"))
    {
        return MIME_GIF;
    }else if (contains(path, ".png"))
    {
        return MIME_PNG;
    }else if (contains(path, ".js"))
    {
        return MIME_JS;
    }else
    {
        return "application/octet-stream";
    }
}

// assets.cpp の MIMEType を写したもの
static std::string cxx_assets_mime(const std::string& path)
{
    using namespace str;

    if (has_suffix(path, ".htm") || has_suffix(path, ".html"))
    {
        return MIME_HTML;
    }else if (has_suffix(path, ".css"))
    {
        return MIME_CSS;
    }else if (has_suffix(path, ".jpg"))
    {
        return MIME_JPEG;
    }else if (has_suffix(path, ".gif"))
    {
        return MIME_GIF;
    }else if (has_suffix(path, ".png"))
    {
        return MIME_PNG;
    }else if (has_suffix(path, ".js"))
    {
        return MIME_JS;
    }else if (has_suffix(path, ".svg"))
    {
        return "image/svg+xml";
    }else if (has_suffix(path, ".ico"))
    {
        return "image/vnd.microsoft.icon";
    }else
    {
        return "application/octet-stream";
    }
}

// PublicController::operator() の振り分けを写したもの
static int cxx_route(const std::string& path)
{
    if (path == "/public") return 0;
    else if (path == "/public/") return 1;
    else if (path == "/public/index.txt") return 2;
    else if (path == "/public/play.html") return 3;
    else return 4;
}

// AssetsController::operator() の 304 の判断を写したもの
static bool cxx_not_modified(time_t last_modified, const std::string& ims)
{
    time_t if_modified_since = -1;

    if (ims.size())
        if_modified_since = cgi::parseHttpDate(ims);

    if (last_modified != -1 && if_modified_since != -1)
        if (last_modified <= if_modified_since)
            return true;
    return false;
}

static void testPaths()
{
    const std::vector<std::string> toks = { "/public", "/public/", "index.txt", "play.html", "/", "?", "??", "a", ".htm",
                                            ".html", ".HTML", ".css", ".jpg", ".gif", ".png", ".js", ".json", ".svg",
                                            ".ico", ".map", ".", "id=1", "&", "=", "", std::string("\0", 1), "\xff" };
    for (int i = 0; i < 300000; i++)
    {
        std::string v;
        for (int j = rnd(6); j > 0; j--) v += toks[rnd((int) toks.size())];
        auto u = reinterpret_cast<const uint8_t*>(v.data());

        g_cases += 3;
        std::string a = cxx_public_mime(v), b = pcrs_public_mime_type(u, v.size());
        // C++ 版の str::contains は NUL で切れない (std::string::find) ので、NUL の違いはない
        if (a != b && ++g_bad <= 5) printf("public MIMEType(%s): C++ %s / Rust %s\n", v.c_str(), a.c_str(), b.c_str());
        a = cxx_assets_mime(v); b = pcrs_assets_mime_type(u, v.size());
        if (a != b && ++g_bad <= 5) printf("assets MIMEType(%s): C++ %s / Rust %s\n", v.c_str(), a.c_str(), b.c_str());
        int ra = cxx_route(v), rb = pcrs_public_route(u, v.size());
        if (ra != rb && ++g_bad <= 5) printf("route(%s): C++ %d / Rust %d\n", v.c_str(), ra, rb);

        HTTPRequest req("GET", v, "HTTP/1.0", {});
        pcrs_buf p, q;
        pcrs_http_split_url(u, v.size(), &p, &q);
        a = req.path + "|" + req.queryString;
        b = rustbridge::RustBuf(p).str() + "|" + rustbridge::RustBuf(q).str();
        g_cases++;
        // C++ 版の str::split は NUL で切れる (段階1 の既知の違い)
        if (a != b && v.find('\0') != std::string::npos) { g_knownNul++; continue; }
        if (a != b && ++g_bad <= 5) printf("HTTPRequest(%s): C++ %s / Rust %s\n", v.c_str(), a.c_str(), b.c_str());
    }

    const std::vector<std::string> dates = { "Sun, 06 Nov 1994 08:49:37 GMT", "Sunday, 06-Nov-94 08:49:37 GMT",
                                             "Sun Nov  6 08:49:37 1994", "Thu, 01 Jan 1970 00:00:00 GMT",
                                             "Tue, 19 Jan 2038 03:14:08 GMT", "Fri, 31 Dec 9999 23:59:59 GMT",
                                             "yesterday", "", " ", "Sun, 06 Nov 1994 08:49:37", "Sun, 32 Nov 1994 08:49:37 GMT",
                                             "Sun, 06 Nov 1994 25:49:37 GMT", "Sun, 06 Foo 1994 08:49:37 GMT" };
    const time_t base = 784111777; // 1994-11-06 08:49:37
    for (int i = 0; i < 200000; i++)
    {
        std::string ims = dates[rnd((int) dates.size())];
        if (rnd(4) == 0 && !ims.empty()) ims[rnd((int) ims.size())] = "0123456789 ,:-GMTx"[rnd(18)];
        time_t lm;
        switch (rnd(5))
        {
        case 0: lm = -1; break;
        case 1: lm = base + rnd(3) - 1; break;
        case 2: lm = 0; break;
        case 3: lm = (time_t) rng() * 3; break;
        default: lm = base + rnd(1000000) - 500000; break;
        }
        bool a = cxx_not_modified(lm, ims);
        bool b = pcrs_assets_not_modified(lm, reinterpret_cast<const uint8_t*>(ims.data()), ims.size());
        g_cases++;
        if (a != b && ++g_bad <= 5) printf("not modified(%ld, %s): C++ %d / Rust %d\n", (long) lm, ims.c_str(), a, b);
    }
}

int main()
{
    sys = new MockSys();
    testParseOptions();
    testAcceptLanguage();
    testUptime();
    testMapper();
    testPaths();
    printf("比較件数 %ld、既知の違い (str::split の NUL、段階1) %ld、(隣のディレクトリへのトラバーサル、段階8c) %ld、"
           "説明のつかない違い %ld\n", g_cases, g_knownNul, g_knownSibling, g_bad);
    return g_bad ? 1 : 0;
}
