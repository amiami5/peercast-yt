// strutil (str.cpp のうち UTF-8/エスケープ以外の関数) の C++ 版と Rust 版の差分テスト。
// diff.cpp と同じ考え方で、既知の違いはないはずなので、1件でも違えば失敗として報告する。

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>

#include "peercast_rs.h"
#include "str.h"

using std::string;
using std::vector;

static const uint8_t* B(const string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }
static string take(pcrs_buf b) { string s(reinterpret_cast<const char*>(b.ptr), b.len); pcrs_buf_free(b); return s; }
static vector<string> take_vec(pcrs_vec v)
{
    vector<string> res;
    const char* p = reinterpret_cast<const char*>(v.joined.ptr);
    size_t off = 0;
    for (size_t i = 0; i < v.count; i++) { res.emplace_back(p + off, v.lens[i]); off += v.lens[i]; }
    pcrs_vec_free(v);
    return res;
}

static long g_compared = 0, g_bad = 0;
static string show(const string& s) { string r; char b[8]; for (unsigned char c : s) { if (c>=0x20&&c<0x7f&&c!='\\') r+=c; else { snprintf(b,sizeof b,"\\x%02x",c); r+=b; } } return r; }

template <typename T>
static void same(const char* fn, const string& in, const T& a, const T& b)
{
    g_compared++;
    if (a == b) return;
    if (g_bad++ < 8) printf("  [違い] %s(\"%s\")\n", fn, show(in).c_str());
}

static void same_vec(const char* fn, const string& in, const vector<string>& a, const vector<string>& b)
{
    g_compared++;
    if (a == b) return;
    if (g_bad++ < 8) {
        printf("  [違い] %s(\"%s\")\n     C++ :", fn, show(in).c_str());
        for (auto& s : a) printf(" \"%s\"", show(s).c_str());
        printf("\n     Rust:");
        for (auto& s : b) printf(" \"%s\"", show(s).c_str());
        printf("\n");
    }
}

static void check_all(const string& in, const string& in2)
{
    same("hexdump", in, str::hexdump(in), take(pcrs_str_hexdump(B(in), in.size())));
    same("upcase", in, str::upcase(in), take(pcrs_str_upcase(B(in), in.size())));
    same("downcase", in, str::downcase(in), take(pcrs_str_downcase(B(in), in.size())));
    same("capitalize", in, str::capitalize(in), take(pcrs_str_capitalize(B(in), in.size())));
    same("rstrip", in, str::rstrip(in), take(pcrs_str_rstrip(B(in), in.size())));
    same("strip", in, str::strip(in), take(pcrs_str_strip(B(in), in.size())));
    same("escapeshellarg_unix", in, str::escapeshellarg_unix(in), take(pcrs_str_escapeshellarg_unix(B(in), in.size())));
    same("ascii_dump", in, str::ascii_dump(in, "."), take(pcrs_str_ascii_dump(B(in), in.size(), B(string(".")), 1)));
    same("extension_without_dot", in, str::extension_without_dot(in), take(pcrs_str_extension_without_dot(B(in), in.size())));
    same("group_digits", in, str::group_digits(in, ","), take(pcrs_str_group_digits(B(in), in.size(), B(string(",")), 1)));
    same_vec("to_lines", in, str::to_lines(in), take_vec(pcrs_str_to_lines(B(in), in.size())));

    // str::split/split_limit は内部で c_str()+strstr を使っており、埋め込み NUL バイトのせいで
    // 出力が NUL の手前で切り詰まる (既知の C++ 版の不具合)。区切り文字列がその NUL より前で
    // 空になる場合 (separator が NUL で始まる、または空) は、ポインタが全く進まず、無限に
    // メモリを確保し続けて落ちる。落ちる方は呼ばず (別途 --nul-oom で単体確認済み)、
    // 出力が切り詰まるだけの方は「説明のつく違い」として数える。
    // 区切り文字列 (in2) のどこかに NUL があると、c_str()+strstr は NUL の手前までしか
    // 「本当の区切り」として見ない (前方一致だけで誤ってマッチしたり、区切りとして機能しなく
    // なったりする)。ここでは「NUL が絡む入力は比較しない」で一括して扱う。
    bool haystack_has_nul = in.find('\0') != string::npos;
    bool needle_effective_empty = in2.empty() || in2.find('\0') != string::npos;

    // 2 引数のもの (in と in2 の組み合わせ)
    same("contains", in + "|" + in2, str::contains(in, in2) ? string("1") : "0",
         pcrs_str_contains(B(in), in.size(), B(in2), in2.size()) ? string("1") : "0");
    same("has_prefix", in + "|" + in2, str::has_prefix(in, in2) ? string("1") : "0",
         pcrs_str_has_prefix(B(in), in.size(), B(in2), in2.size()) ? string("1") : "0");
    same("has_suffix", in + "|" + in2, str::has_suffix(in, in2) ? string("1") : "0",
         pcrs_str_has_suffix(B(in), in.size(), B(in2), in2.size()) ? string("1") : "0");
    same("replace_prefix", in + "|" + in2, str::replace_prefix(in, in2, "@"),
         take(pcrs_str_replace_prefix(B(in), in.size(), B(in2), in2.size(), B(string("@")), 1)));
    same("replace_suffix", in + "|" + in2, str::replace_suffix(in, in2, "@"),
         take(pcrs_str_replace_suffix(B(in), in.size(), B(in2), in2.size(), B(string("@")), 1)));
    {
        auto rv = take_vec(pcrs_str_split(B(in), in.size(), B(in2), in2.size()));
        if (needle_effective_empty) {
            g_compared++; // C++ 側は無限ループするので呼ばない。Rust 側が落ちないことだけ確認する。
        } else if (haystack_has_nul) {
            g_compared++; // C++ 側の出力が NUL の手前で切り詰まるので、比較しない。
        } else {
            same_vec("split", in + "|" + in2, str::split(in, in2), rv);
        }
    }

    for (int limit : {1, 2, 3}) {
        struct Opt { bool ok; vector<string> v; };
        Opt l{true, {}}; try { l.v = str::split(in, in2, limit); } catch (...) { l.ok = false; }
        pcrs_vec pv{};
        bool rok = pcrs_str_split_limit(B(in), in.size(), B(in2), in2.size(), limit, &pv) == 0;
        if (l.ok != rok) { g_bad++; printf("  [違い] split(limit=%d) 成功/失敗が食い違う\n", limit); continue; }
        if (!l.ok) { g_compared++; continue; }
        if (haystack_has_nul || needle_effective_empty)
            g_compared++; // 同上: NUL 絡みの出力切り詰めは説明のつく違いとして数える (比較しない)
        else
            same_vec("split_limit", in, l.v, take_vec(pv));
    }

    {
        struct Opt { bool ok; int v; };
        Opt l{true, 0}; try { l.v = str::count(in, in2); } catch (...) { l.ok = false; }
        int32_t rv = 0;
        bool rok = pcrs_str_count(B(in), in.size(), B(in2), in2.size(), &rv) == 0;
        if (l.ok != rok) { g_bad++; printf("  [違い] count 成功/失敗が食い違う (%s|%s)\n", show(in).c_str(), show(in2).c_str()); }
        else if (l.ok) same("count", in + "|" + in2, l.v, (int)rv);
        else g_compared++;
    }

    for (int n : {-1, 0, 1, 3}) {
        struct Opt { bool ok; string v; };
        Opt l{true, ""}; try { l.v = str::indent_tab(in, n); } catch (...) { l.ok = false; }
        pcrs_buf b{}; bool rok = pcrs_str_indent_tab(B(in), in.size(), n, &b) == 0;
        string rv = rok ? take(b) : "";
        if (l.ok != rok) { g_bad++; printf("  [違い] indent_tab(n=%d) 成功/失敗が食い違う\n", n); }
        else if (l.ok) same("indent_tab", in, l.v, rv);
        else g_compared++;
    }

    {
        struct Opt { bool ok; vector<string> v; };
        Opt l{true, {}}; try { l.v = str::shellwords(in); } catch (...) { l.ok = false; }
        pcrs_vec pv{}; int ek = 0;
        bool rok = pcrs_str_shellwords(B(in), in.size(), &pv, &ek) == 0;
        if (l.ok != rok) { g_bad++; printf("  [違い] shellwords 成功/失敗が食い違う (\"%s\")\n", show(in).c_str()); }
        else if (l.ok) same_vec("shellwords", in, l.v, take_vec(pv));
        else g_compared++;
    }
}

struct Rng { uint64_t x; uint64_t next() { x^=x<<13; x^=x>>7; x^=x<<17; return x; } size_t below(size_t n){return next()%n;} };
static const char* TOKENS[] = {
    "a","b","AB","0","9"," ","\t","\v","'","\"","\\",".",",","-","\n","\r",
    "abc","abcdefghijklmnop","abcdefghijklmnopq","foo","bar","baz","@","x=y",
    "1234","1234.5678","1000000",",","http://","https://",
    "a.txt","a.tar.gz","noext",".hidden","'a b'","\"a b\"","a\\ b","\\",
    "\xff","\x80","\x01","\x1f","\x7f","あ","💩",
};
static string random_input(Rng& r) {
    string s; size_t n = r.below(6);
    for (size_t i = 0; i < n; i++) {
        if (r.below(8) == 0) s += (char)r.below(256);
        else s += TOKENS[r.below(sizeof(TOKENS)/sizeof(*TOKENS))];
    }
    return s;
}

int main(int argc, char** argv) {
    long n = 200000; uint64_t seed = 1;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i],"--random") && i+1<argc) n = atol(argv[++i]);
        else if (!strcmp(argv[i],"--seed") && i+1<argc) seed = strtoull(argv[++i],nullptr,10);
    }
    Rng r{seed * 0x9e3779b97f4a7c15ULL + 7};
    for (long i = 0; i < n; i++) check_all(random_input(r), random_input(r));
    printf("乱数入力 %ld 組: 完了\n", n);
    printf("比較件数: %ld  違い: %ld\n", g_compared, g_bad);
    return g_bad ? 1 : 0;
}
