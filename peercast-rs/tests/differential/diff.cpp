// C++ 版 (core/common の cgi.cpp と str.cpp) と Rust 版 (peercast-rs) に同じ入力を与えて、出力を比べる。
//
// 使い方: make && ./diff [--exhaustive3] [--random N] [--seed S]
//
// 結果が違ってもよいのは、docs/rust-migration.md と各関数の説明に書いた「C++ 版との違い」だけ。
// それに当たる場合は理由ごとに数えて表示し、どれにも当たらない違いは「説明のつかない違い」として
// 表示して、終了コードを 1 にする。

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

#include "cgi.h"
#include "peercast_rs.h"
#include "str.h"

using std::string;

// ---------------------------------------------------------------- Rust 呼び出し
static const uint8_t* B(const string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }

static string take(pcrs_buf b)
{
    string s(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return s;
}

struct Opt { bool ok; string v; };

static Opt take_opt(int rc, pcrs_buf b)
{
    if (rc != 0) return {false, ""};
    return {true, take(b)};
}

static Opt legacy_opt(const std::function<string()>& f)
{
    try { return {true, f()}; } catch (const std::exception&) { return {false, ""}; }
}

// ---------------------------------------------------------------- 独立した参照実装
// RFC 3629 の表どおりの厳密な UTF-8 検査。Rust の標準ライブラリとは別に、ここで書き直している。
static bool strict_utf8(const string& s)
{
    size_t i = 0, n = s.size();
    auto b = [&](size_t k) { return static_cast<unsigned char>(s[k]); };
    while (i < n) {
        unsigned c = b(i);
        if (c <= 0x7f) { i++; continue; }
        size_t len; unsigned lo = 0x80, hi = 0xbf;
        if (c >= 0xc2 && c <= 0xdf) len = 2;
        else if (c == 0xe0) { len = 3; lo = 0xa0; }
        else if (c >= 0xe1 && c <= 0xec) len = 3;
        else if (c == 0xed) { len = 3; hi = 0x9f; }
        else if (c >= 0xee && c <= 0xef) len = 3;
        else if (c == 0xf0) { len = 4; lo = 0x90; }
        else if (c >= 0xf1 && c <= 0xf3) len = 4;
        else if (c == 0xf4) { len = 4; hi = 0x8f; }
        else return false;
        if (i + len > n) return false;
        unsigned c1 = b(i + 1);
        if (c1 < lo || c1 > hi) return false;
        for (size_t k = 2; k < len; k++)
            if ((b(i + k) & 0xc0) != 0x80) return false;
        i += len;
    }
    return true;
}

// ---------------------------------------------------------------- 集計
struct Stats {
    long compared = 0;                         // 完全一致を確かめた数
    std::map<string, long> explained;          // 説明のつく違い (理由 → 件数)
    long unexplained = 0;
};

static std::map<string, Stats> g_stats;
static string show(const string& s)
{
    string r;
    char buf[8];
    for (unsigned char c : s) {
        if (c >= 0x20 && c < 0x7f && c != '\\') r += c;
        else { snprintf(buf, sizeof buf, "\\x%02x", c); r += buf; }
    }
    return r;
}

static void same(const char* fn, const string& in, const string& a, const string& b)
{
    Stats& st = g_stats[fn];
    if (a == b) { st.compared++; return; }
    if (st.unexplained++ < 5)
        printf("  [説明のつかない違い] %s(\"%s\")\n     C++ : \"%s\"\n     Rust: \"%s\"\n", fn, show(in).c_str(), show(a).c_str(), show(b).c_str());
}

static void explained(const char* fn, const char* reason) { g_stats[fn].explained[reason]++; }
static void unexplained(const char* fn, const string& in, const char* what)
{
    Stats& st = g_stats[fn];
    if (st.unexplained++ < 5) printf("  [説明のつかない違い] %s(\"%s\"): %s\n", fn, show(in).c_str(), what);
}

// ---------------------------------------------------------------- 説明のつく違いの判定
// %XX でない % を含むか (C++ 版の unescape は、この場合 sscanf の結果を確かめず未初期化の値を使う)。
static bool has_malformed_percent(const string& s)
{
    for (size_t i = 0; i < s.size(); i++) {
        if (s[i] != '%') continue;
        if (i + 2 >= s.size() + 0 && i + 2 > s.size() - 1) return true;
        if (!isxdigit((unsigned char)s[i + 1]) || !isxdigit((unsigned char)s[i + 2])) return true;
        i += 2;
    }
    return false;
}

// &#数字; が 2^32 以上か (C++ 版は uint32_t に切り詰めて別の文字になる)。
static bool has_wrapping_numeric_ref(const string& s)
{
    for (size_t i = 0; i + 2 < s.size(); i++) {
        if (s[i] != '&' || s[i + 1] != '#') continue;
        size_t j = i + 2;
        int radix = 10;
        if (j < s.size() && (s[j] == 'x' || s[j] == 'X')) { radix = 16; j++; }
        unsigned __int128 v = 0; size_t digits = 0;
        while (j < s.size() && isxdigit((unsigned char)s[j])) {
            int d = isdigit((unsigned char)s[j]) ? s[j] - '0' : (tolower(s[j]) - 'a' + 10);
            if (d >= radix) break;
            v = v * radix + d; digits++; j++;
            if (v > ((unsigned __int128)1 << 100)) break;
        }
        if (digits && j < s.size() && s[j] == ';' && v >= ((unsigned __int128)1 << 32)) return true;
    }
    return false;
}

// ---------------------------------------------------------------- 各関数の比較
static void check_all(const string& in)
{
    // --- 完全に一致するはずのもの
    same("cgi::escape", in, cgi::escape(in), take(pcrs_cgi_escape(B(in), in.size())));
    same("cgi::escape_html", in, cgi::escape_html(in), take(pcrs_cgi_escape_html(B(in), in.size())));
    same("cgi::escape_javascript", in, cgi::escape_javascript(in), take(pcrs_cgi_escape_javascript(B(in), in.size())));
    same("cgi::isSafeLocalPath", in, cgi::isSafeLocalPath(in) ? "1" : "0", pcrs_cgi_is_safe_local_path(B(in), in.size()) ? "1" : "0");
    same("str::is_http_url", in, str::is_http_url(in) ? "1" : "0", pcrs_str_is_http_url(B(in), in.size()) ? "1" : "0");

    // --- unescape: 不正な % 以外は一致するはず
    {
        string r = take(pcrs_cgi_unescape(B(in), in.size()));
        if (has_malformed_percent(in)) {
            explained("cgi::unescape", "不正な %XX (C++ 版は未初期化の値を使う)");
            // Rust 版は、不正な % をそのまま残す。それ以外は 1 対 1 に対応するので、長さで確かめる。
            if (r.size() > in.size()) unexplained("cgi::unescape", in, "出力が入力より長い");
        } else {
            same("cgi::unescape", in, cgi::unescape(in), r);
        }
    }

    // --- unescape_html
    // 生の非 ASCII バイトは、どちらの実装もそのまま写すだけで、& や ; の判定にも影響しない。
    // 「不正な UTF-8 を出力した」の判定が入力の不正なバイトに惑わされないよう、比べるときは
    // それらを '@' (どちらでもない ASCII) に置き換えた入力を使う。
    {
        string m = in;
        for (auto& c : m) if (static_cast<unsigned char>(c) >= 0x80) c = '@';

        string rm = take(pcrs_cgi_unescape_html(B(m), m.size()));
        Opt l = legacy_opt([&] { return cgi::unescape_html(m); });
        if (!strict_utf8(rm)) unexplained("cgi::unescape_html", in, "ASCII の入力から不正な UTF-8 を出力した");
        if (!l.ok) explained("cgi::unescape_html", "C++ 版が例外を投げる (無効なコードポイント)");
        else if (has_wrapping_numeric_ref(m)) explained("cgi::unescape_html", "C++ 版は 2^32 以上の数を切り詰める");
        else if (!strict_utf8(l.v)) explained("cgi::unescape_html", "C++ 版が不正な UTF-8 を出力する (U+0080〜U+07FF またはサロゲート)");
        else same("cgi::unescape_html", m, l.v, rm);

        // 元の入力そのものについても、Rust 版が落ちないこと、正しい UTF-8 なら正しい UTF-8 で返すことを確かめる
        string r = take(pcrs_cgi_unescape_html(B(in), in.size()));
        if (strict_utf8(in) && !strict_utf8(r)) unexplained("cgi::unescape_html", in, "正しい UTF-8 の入力から不正な UTF-8 を出力した");
    }

    // --- UTF-8
    {
        bool lax = str::validate_utf8(in);
        bool strict = pcrs_str_validate_utf8(B(in), in.size());
        Stats& st = g_stats["str::validate_utf8"];
        if (strict != strict_utf8(in)) unexplained("str::validate_utf8", in, "Rust 版が RFC 3629 の参照実装と違う");
        else if (lax == strict) st.compared++;
        else if (lax && !strict) explained("str::validate_utf8", "C++ 版は過長表現・サロゲート・U+10FFFF 超を許す");
        else unexplained("str::validate_utf8", in, "C++ 版が不正としたものを Rust 版が正しいとした");

        // valid_utf8 と inspect は、検査の結果が同じときだけ完全一致するはず
        string rv = take(pcrs_str_valid_utf8(B(in), in.size()));
        string ri = take(pcrs_str_inspect(B(in), in.size()));
        if (lax == strict) {
            same("str::valid_utf8", in, str::valid_utf8(in), rv);
            same("str::inspect", in, str::inspect(in), ri);
        } else {
            explained("str::valid_utf8", "検査の結果が違う入力");
            explained("str::inspect", "検査の結果が違う入力");
        }

        // json_inspect: C++ 版は不正な UTF-8 で例外
        Opt lj = legacy_opt([&] { return str::json_inspect(in); });
        pcrs_buf jb{nullptr, 0};
        Opt rj = take_opt(pcrs_str_json_inspect(B(in), in.size(), &jb), jb);
        if (lj.ok && rj.ok) same("str::json_inspect", in, lj.v, rj.v);
        else if (!lj.ok && !rj.ok) g_stats["str::json_inspect"].compared++;
        else if (lj.ok && !rj.ok && !strict_utf8(in)) explained("str::json_inspect", "C++ 版は厳密には不正な UTF-8 を通す");
        else unexplained("str::json_inspect", in, "成功と失敗が食い違う");

        // truncate_utf8
        for (size_t limit : {0u, 1u, 2u, 3u, 4u, 5u, 8u, 1000u}) {
            Opt lt = legacy_opt([&] { return str::truncate_utf8(in, limit); });
            pcrs_buf tb{nullptr, 0};
            Opt rt = take_opt(pcrs_str_truncate_utf8(B(in), in.size(), limit, &tb), tb);
            if (lt.ok && rt.ok) same("str::truncate_utf8", in, lt.v, rt.v);
            else if (!lt.ok && !rt.ok) g_stats["str::truncate_utf8"].compared++;
            else if (lt.ok && !rt.ok && !strict_utf8(lt.v)) explained("str::truncate_utf8", "C++ 版は厳密には不正な UTF-8 を通す");
            else unexplained("str::truncate_utf8", in, "成功と失敗が食い違う (または理由が説明できない)");
            if (rt.ok && (rt.v.size() > limit || !strict_utf8(rt.v) || in.compare(0, rt.v.size(), rt.v) != 0))
                unexplained("str::truncate_utf8", in, "Rust 版の出力が性質を満たさない (長さ・UTF-8・前方一致)");
        }
    }
}

static void check_codepoints()
{
    for (uint32_t cp = 0; cp < 0x110000 + 0x2000; cp++) {
        Opt l = legacy_opt([&] { return str::codepoint_to_utf8(cp); });
        pcrs_buf b{nullptr, 0};
        Opt r = take_opt(pcrs_str_codepoint_to_utf8(cp, &b), b);
        char in[16]; snprintf(in, sizeof in, "U+%04X", cp);
        const char* fn = "str::codepoint_to_utf8";
        bool scalar = cp < 0x110000 && !(cp >= 0xd800 && cp <= 0xdfff);
        if (r.ok != scalar) { unexplained(fn, in, "Rust 版が Unicode のスカラー値を判別できていない"); continue; }
        if (!r.ok) {
            if (cp >= 0x110000 && l.ok) unexplained(fn, in, "C++ 版は範囲外で例外を投げるはず");
            else if (cp >= 0xd800 && cp <= 0xdfff) explained(fn, "サロゲート (C++ 版は不正な UTF-8 を出力する)");
            else g_stats[fn].compared++;
            continue;
        }
        if (!strict_utf8(r.v)) { unexplained(fn, in, "Rust 版の出力が正しい UTF-8 でない"); continue; }
        if (cp >= 0x80 && cp <= 0x7ff) {
            // C++ 版は先頭バイトが 0xB0 | (cp >> 6) になる誤りがある。それ以外は同じ。
            bool as_documented = l.ok && l.v.size() == 2 && (unsigned char)l.v[0] == (0xb0 | (cp >> 6)) && l.v[1] == r.v[1];
            if (as_documented) explained(fn, "U+0080〜U+07FF (C++ 版は先頭バイトを 0xB0 と OR している誤り)");
            else unexplained(fn, in, "C++ 版の出力が、想定した誤りの形と違う");
        } else same(fn, in, l.v, r.v);
    }
}

// ---------------------------------------------------------------- 入力の生成
struct Rng {
    uint64_t x;
    uint64_t next() { x ^= x << 13; x ^= x >> 7; x ^= x << 17; return x; }
    size_t below(size_t n) { return next() % n; }
};

static const char* TOKENS[] = {
    "a", "Z", "0", "9", " ", "+", "%", "%4", "%41", "%zz", "%2f", "%E3%81%82", "&", ";", "#", "x", "X",
    "&lt;", "&gt;", "&amp;", "&quot;", "&apos;", "&copy;", "&nbsp;", "&yen;", "&eacute;", "&hearts;", "&AMP;", "&foo;",
    "&#65;", "&#x41;", "&#X41;", "&#169;", "&#xA9;", "&#12354;", "&#x1F4A9;", "&#0;", "&#x0;", "&#xD800;", "&#xDFFF;",
    "&#x10FFFF;", "&#x110000;", "&#4294967361;", "&#x100000041;", "&#99999999999999999999;", "&#;", "&#x;", "&#12a;",
    "<", ">", "\"", "'", "\\", "/", "//", "/\\", "\r", "\n", "\t", "\x01", "\x1f", "\x7f", "\x80", "\xbf", "\xc0", "\xc1",
    "\xc2", "\xdf", "\xe0", "\xe3", "\x81", "\x82", "\xed", "\xa0", "\xef", "\xf0", "\x9f", "\x92", "\xa9", "\xf4", "\x90",
    "\xf5", "\xff", "あ", "💩", "é", "http://", "https://", "HTTP://", "HtTpS://", "http:/", "ftp://", "javascript:",
    "/html/en/index.html", "?id=1&x=%20y", "\xe3\x81\x82", "\xf0\x9f\x92\xa9",
};

static string random_input(Rng& r)
{
    string s;
    size_t ntok = r.below(10);
    for (size_t i = 0; i < ntok; i++) {
        if (r.below(6) == 0) s += static_cast<char>(r.below(256));
        else s += TOKENS[r.below(sizeof(TOKENS) / sizeof(*TOKENS))];
    }
    return s;
}

int main(int argc, char** argv)
{
    long random_n = 300000;
    uint64_t seed = 1;
    bool exhaustive3 = false;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--random") && i + 1 < argc) random_n = atol(argv[++i]);
        else if (!strcmp(argv[i], "--seed") && i + 1 < argc) seed = strtoull(argv[++i], nullptr, 10);
        else if (!strcmp(argv[i], "--exhaustive3")) exhaustive3 = true;
    }

    // 1. 長さ 0〜2 のバイト列を全部
    check_all("");
    for (int a = 0; a < 256; a++) {
        string s1(1, static_cast<char>(a));
        check_all(s1);
        for (int b = 0; b < 256; b++) check_all(s1 + static_cast<char>(b));
    }
    printf("長さ 0〜2 の全 %d 通り: 完了\n", 1 + 256 + 65536);

    // 2. 長さ 3 のバイト列を全部 (1,677 万通り。UTF-8 と % の判定を網羅する)
    if (exhaustive3) {
        for (int a = 0; a < 256; a++)
            for (int b = 0; b < 256; b++) {
                string p; p += static_cast<char>(a); p += static_cast<char>(b);
                for (int c = 0; c < 256; c++) check_all(p + static_cast<char>(c));
            }
        printf("長さ 3 の全 16777216 通り: 完了\n");
    }

    // 3. トークンをつなげた乱数入力
    Rng r{seed * 0x9e3779b97f4a7c15ULL + 1};
    for (long i = 0; i < random_n; i++) check_all(random_input(r));
    printf("乱数入力 %ld 件 (seed=%llu): 完了\n", random_n, (unsigned long long)seed);

    // 4. コードポイント全体
    check_codepoints();
    printf("コードポイント U+0000〜U+111FFF: 完了\n\n");

    long bad = 0;
    printf("%-26s %12s  %s\n", "関数", "完全一致", "説明のつく違い / 説明のつかない違い");
    for (auto& kv : g_stats) {
        long ex = 0;
        for (auto& e : kv.second.explained) ex += e.second;
        printf("%-26s %12ld  %ld / %ld\n", kv.first.c_str(), kv.second.compared, ex, kv.second.unexplained);
        for (auto& e : kv.second.explained) printf("    - %s: %ld\n", e.first.c_str(), e.second);
        bad += kv.second.unexplained;
    }
    printf("\n説明のつかない違い: %ld\n", bad);
    return bad ? 1 : 0;
}
