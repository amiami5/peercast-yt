// イエローページのチャンネル一覧 (core/common/chandir.cpp) の、C++ 版と Rust 版の差分テスト。
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の ChannelEntry。chandir.cpp の中の static な
// directoryUrlOf と formatTime は、元のコードをここに写して比べる。
#include <cstdio>
#include <cstdlib>
#include <random>
#include <string>
#include <vector>

#include "chandir.h"
#include "peercast_rs.h"
#include "regexp.h"
#include "str.h"

static long g_bad = 0, g_cases = 0, g_known = 0, g_known_nul = 0;
static std::mt19937 rng(20260924);

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
static std::string take(pcrs_buf b)
{
    std::string s(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return s;
}

// ---- 元のコード (chandir.cpp の static 関数) ----
static std::string directoryUrlOf(const std::string& url)
{
    auto matches = Regexp("/[^/]*$").exec(url);

    if (matches.size() > 0) {
        return str::replace_suffix(url, matches[0], "/");
    } else {
        return url;
    }
}

static std::string formatTime(unsigned int diff)
{
    auto min = diff / 60;
    auto sec = diff % 60;
    if (min == 0) {
        return str::format("%ds", sec);
    } else {
        return str::format("%dm %ds", min, sec);
    }
}

// ---- 比べる形 ----
static std::string dumpEntry(const ChannelEntry& e)
{
    std::string r;
    for (auto* f : { &e.name, &e.tip, &e.url, &e.genre, &e.desc, &e.contentTypeStr, &e.trackArtist, &e.trackAlbum,
                     &e.trackName, &e.trackContact, &e.encodedName, &e.uptime, &e.status, &e.comment })
        r += *f + "|";
    r += e.id.str() + "|" + std::to_string(e.numDirects) + "|" + std::to_string(e.numRelays) + "|" +
         std::to_string(e.bitrate) + "|" + std::to_string(e.direct);
    return r;
}

static std::string bs(pcrs_bytes b) { return std::string(reinterpret_cast<const char*>(b.ptr), b.len); }

static std::string dumpC(const pcrs_chan_entry* c)
{
    std::string r;
    for (auto f : { c->name, c->tip, c->url, c->genre, c->desc, c->content_type, c->track_artist, c->track_album,
                    c->track_name, c->track_contact, c->encoded_name, c->uptime, c->status, c->comment })
        r += bs(f) + "|";
    char hex[33];
    for (int i = 0; i < 16; i++) snprintf(hex + i * 2, 3, "%02X", c->id[i]);
    r += std::string(hex) + "|" + std::to_string(c->num_directs) + "|" + std::to_string(c->num_relays) + "|" +
         std::to_string(c->bitrate) + "|" + std::to_string(c->direct);
    return r;
}

static void onEntry(void* ctx, const pcrs_chan_entry* c) { static_cast<std::vector<std::string>*>(ctx)->push_back("E:" + dumpC(c)); }
static void onError(void* ctx, int32_t lineno)
{
    static_cast<std::vector<std::string>*>(ctx)->push_back("X:" + str::format("Parse error at line %d.", (int)lineno));
}

// 数の欄が int に収まらない (atoi の桁あふれ。既知の違い)
static bool overflows(const std::string& f)
{
    // strtod は 16 進数や inf も読むので、atoi が読む 10 進数の部分だけで判断する
    std::string digits;
    const char* q = f.c_str();
    while (*q == ' ' || (*q >= '\t' && *q <= '\r')) q++;
    if (*q == '+' || *q == '-') digits += *q++;
    while (*q >= '0' && *q <= '9') digits += *q++;
    double d = strtod(digits.c_str(), nullptr);
    return d > 2147483647.0 || d < -2147483648.0;
}

static bool lineOverflows(const std::string& line)
{
    auto f = str::split(line, "<>");
    if (f.size() < 19) return false;
    return overflows(f[6]) || overflows(f[7]) || overflows(f[8]) || overflows(f[18]);
}

// ---- 入力を作る ----
static const char* PIECES[] = {
    "", "0", "1", "-1", "+7", " 42", "\t-3x", "2147483647", "2147483648", "-2147483649", "99999999999", "0x10",
    "http://a/", "HTTP://X", "https://y/z", "HtTpS://", "http:/", "javascript:alert(1)", "data:text/html,x",
    "97968780D09CC97BB98D4A2BF221EDE7", "0123456789abcdef0123456789ABCDEF", "0123456789ABCDEF0123456789ABCDE",
    "zz968780D09CC97BB98D4A2BF221EDE7", "0G968780D09CC97BB98D4A2BF221EDE7FF", "予定地", "%E4%BA%88", "<", ">", "<>",
    "\r", "a\rb", " ", "1:14", "click",
};

static std::string randField()
{
    std::string s;
    int n = rng() % 3;
    for (int i = 0; i < n; i++)
    {
        switch (rng() % 6)
        {
        case 0: { int k = rng() % 6; for (int j = 0; j < k; j++) s += (char)(rng() % 256); break; }
        case 1: s += std::string(rng() % 16 == 0 ? 1 : 0, '\0') + PIECES[rng() % (sizeof PIECES / sizeof *PIECES)]; break;
        default: s += PIECES[rng() % (sizeof PIECES / sizeof *PIECES)]; break;
        }
    }
    return s;
}

static std::string randLine()
{
    int n = 19;
    switch (rng() % 5) { case 0: n = rng() % 23; break; case 1: n = 18 + rng() % 3; break; default: break; }
    std::string line;
    for (int i = 0; i < n; i++)
    {
        if (i) line += "<>";
        line += randField();
    }
    if (rng() % 8 == 0) line += '\r';
    return line;
}

static std::string randText(std::vector<std::string>& lines)
{
    std::string text;
    int n = rng() % 6;
    for (int i = 0; i < n; i++)
    {
        std::string l = randLine();
        lines.push_back(l);
        text += l;
        if (i + 1 < n || rng() % 2) text += '\n';
    }
    if (rng() % 10 == 0) text += "\n\n";
    return text;
}

static void textCase()
{
    g_cases++;
    std::vector<std::string> lines;
    std::string text = randText(lines);

    std::vector<std::string> errors;
    auto entries = ChannelEntry::textToChannelEntries(text, "http://yp/index.txt", errors);
    // C++ 版はエラーと結果を別々に返すので、行番号から並べ直す
    std::vector<std::string> a;
    {
        std::vector<std::string> ex;
        for (auto& e : entries) ex.push_back("E:" + dumpEntry(e));
        size_t ei = 0, xi = 0;
        // 行ごとに、欄が 19 個ならエントリ、そうでなければエラー
        std::vector<std::string> ls;
        {
            std::string cur;
            for (char c : text) { if (c == '\n') { ls.push_back(cur); cur.clear(); } else cur += c; }
            if (!cur.empty()) ls.push_back(cur);
        }
        for (auto& l : ls)
        {
            if (str::split(l, "<>").size() == 19) { if (ei < ex.size()) a.push_back(ex[ei++]); }
            else if (xi < errors.size()) a.push_back("X:" + errors[xi++]);
        }
        if (ei != ex.size() || xi != errors.size()) a.push_back("(count mismatch)");
    }

    std::vector<std::string> b;
    pcrs_chandir_parse(u(text), text.size(), &b, onEntry, onError);

    if (a != b)
    {
        // 欄の中の乱数バイトに改行が入ることがあるので、実際の行で判断する
        bool known = false;
        std::string cur;
        for (char c : text + "\n")
        {
            if (c == '\n') { if (lineOverflows(cur)) known = true; cur.clear(); }
            else cur += c;
        }
        if (known) { g_known++; return; }
        // str::split は、C++ 版は NUL で切れる (段階 1 の既知の違い)
        if (text.find('\0') != std::string::npos) { g_known_nul++; return; }
        if (g_bad++ < 20)
        {
            printf("  [違い] textToChannelEntries 入力=\"%s\"\n", show(text).c_str());
            for (auto& s : a) printf("    C++ =%s\n", show(s).c_str());
            for (auto& s : b) printf("    Rust=%s\n", show(s).c_str());
        }
    }
}

static void fieldsCase()
{
    g_cases++;
    std::vector<std::string> fields;
    int n = rng() % 2 ? 19 + rng() % 3 : rng() % 22;
    for (int i = 0; i < n; i++) fields.push_back(randField());
    // GnuID と数の欄は c_str() で読むので、NUL の手前までになる
    for (int i : { 1, 6, 7, 8, 18 })
        if (i < n && rng() % 4 == 0) fields[i].insert(rng() % (fields[i].size() + 1), 1, '\0');
    if (n > 1 && rng() % 4 == 0) fields[1] = std::string("0123456789ABCDEF0123456789ABCDEF").insert(rng() % 33, 1, '\0');

    std::string a;
    try { a = "E:" + dumpEntry(ChannelEntry(fields, "")); }
    catch (std::runtime_error& e) { a = std::string("T:") + e.what(); }

    std::vector<std::string> out;
    std::vector<pcrs_bytes> v;
    for (auto& f : fields) v.push_back({ u(f), f.size() });
    int rc = pcrs_chandir_entry(v.data(), v.size(), &out, onEntry);
    std::string b = rc != 0 ? "T:too few fields" : out.size() == 1 ? out[0] : "(no entry)";

    if (a != b)
    {
        std::string line;
        for (size_t i = 0; i < fields.size(); i++) line += (i ? "<>" : "") + fields[i];
        if (fields.size() >= 19 && (overflows(fields[6]) || overflows(fields[7]) || overflows(fields[8]) || overflows(fields[18])))
        { g_known++; return; }
        if (g_bad++ < 20) printf("  [違い] ChannelEntry 欄=\"%s\"\n    C++ =%s\n    Rust=%s\n", show(line).c_str(), show(a).c_str(), show(b).c_str());
    }
}

static std::string randUrl()
{
    static const char* P[] = { "", "/", "//", "http:", "index.txt", "a", "?x=/", "\n", "yp", ".", "%2F", "\xe3\x81\x82" };
    std::string s;
    int n = rng() % 8;
    for (int i = 0; i < n; i++)
    {
        if (rng() % 10 == 0) s += '\0';
        else s += P[rng() % (sizeof P / sizeof *P)];
    }
    return s;
}

static void urlCase()
{
    std::string feed = randUrl(), name = rng() % 3 ? randField() : "";
    ChannelEntry e({ "", "", "", "", "", "", "", "", "", "", "", "", "", "", name, "", "", "", "" }, feed);

    g_cases++;
    std::string a = e.chatUrl(), b = take(pcrs_chandir_side_url(u(feed), feed.size(), u(e.encodedName), e.encodedName.size(), 0));
    if (a != b && g_bad++ < 20) printf("  [違い] chatUrl feed=\"%s\" name=\"%s\"\n    C++ =%s\n    Rust=%s\n", show(feed).c_str(), show(name).c_str(), show(a).c_str(), show(b).c_str());

    g_cases++;
    a = e.statsUrl(), b = take(pcrs_chandir_side_url(u(feed), feed.size(), u(e.encodedName), e.encodedName.size(), 1));
    if (a != b && g_bad++ < 20) printf("  [違い] statsUrl feed=\"%s\" name=\"%s\"\n    C++ =%s\n    Rust=%s\n", show(feed).c_str(), show(name).c_str(), show(a).c_str(), show(b).c_str());

    g_cases++;
    a = directoryUrlOf(feed), b = take(pcrs_chandir_directory_url(u(feed), feed.size()));
    if (a != b && g_bad++ < 20) printf("  [違い] directoryUrlOf \"%s\"\n    C++ =%s\n    Rust=%s\n", show(feed).c_str(), show(a).c_str(), show(b).c_str());
}

static void timeCase(unsigned int t)
{
    g_cases++;
    std::string a = formatTime(t), b = take(pcrs_chandir_format_time(t));
    if (a != b && g_bad++ < 20) printf("  [違い] formatTime %u C++=%s Rust=%s\n", t, a.c_str(), b.c_str());
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 200000;
    for (long i = 0; i < n; i++)
    {
        textCase();
        fieldsCase();
        urlCase();
        timeCase(rng() % 4 == 0 ? rng() : rng() % 7200);
    }
    for (unsigned int t : { 0u, 59u, 60u, 3599u, 3600u, 0x7fffffffu, 0x80000000u, 0xffffffffu }) timeCase(t);

    printf("%ld 件 (既知の違い: atoi の桁あふれ %ld 件、split の NUL %ld 件)、説明のつかない違い %ld 件\n",
           g_cases, g_known, g_known_nul, g_bad);
    return g_bad ? 1 : 0;
}
