// テンプレートエンジン (core/common/template.cpp) の、C++ 版と Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の Template。Rust 版は、本番と同じ
// 橋渡し (core/common/rusttemplate.h の rustbridge::templateCall) を通して、同じ設定の別の
// Template で動かす。スコープ (変数) はどちらも C++ の GenericScope。
//
// 入力は、UI の実際のテンプレート (引数で渡したファイル)、式の断片を組み合わせたもの、
// それらの変異。出力、戻り値、例外の種類とメッセージ、読み終えた位置、define で変わった
// スコープの中身を比べる。変数の値 (servMgr など) は件ごとに乱数で変える。
//
// 使い方: ./diff_template [件数 (既定 20000)] [テンプレートのファイル...]
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <random>
#include <sstream>
#include <string>
#include <vector>

#include "rusttemplate.h"
#include "sstream.h"
#include "template.h"
#include "../../../tests/mockpeercast.h"

static std::mt19937 rng(20260924);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }
static bool coin(int percent) { return rnd(100) < percent; }

template <typename T>
static const T& pick(const std::vector<T>& v) { return v[rnd((int) v.size())]; }

// ---------------------------------------------------------------- 変数の値

static amf0::Value someScalar()
{
    switch (rnd(9))
    {
    case 0: return amf0::Value();
    case 1: return amf0::Value(true);
    case 2: return amf0::Value(false);
    case 3: return amf0::Value(rnd(5));
    case 4: return amf0::Value(rnd(1000) / 7.0);
    case 5: return amf0::Value("");
    case 6: return amf0::Value("0");
    case 7: return amf0::Value(pick(std::vector<std::string>{ "dark", "light", "system", "orange", "cyan", "ja", "en", "UP", "DOWN", "SERVER", "FREE", "Success" }));
    default: return amf0::Value("<a href=\"x\">'&'</a> 日本語");
    }
}

static amf0::Value someObject(int depth);

static amf0::Value someValue(int depth)
{
    int r = rnd(10);
    if (depth < 2 && r == 0)
        return someObject(depth + 1);
    if (depth < 2 && r == 1)
    {
        std::vector<amf0::Value> a;
        int n = rnd(4);
        for (int i = 0; i < n; i++)
            a.push_back(coin(50) ? someObject(depth + 1) : someScalar());
        return a;
    }
    return someScalar();
}

static const std::vector<std::string> FIELDS = {
    "serverIP", "serverPort", "serverName", "version", "upgradeURL", "id", "type", "status", "name", "authToken",
    "uptime", "bitrate", "url", "contactURL", "preferredTheme", "accentColor", "lang", "filters", "chat", "isRoot",
    "rtmpServerMonitor", "track", "title", "genre", "path", "ssl", "isPrivate", "hasUnsafeFilterSettings", "rtmpPort",
};

static amf0::Value someObject(int depth)
{
    std::map<std::string, amf0::Value> m;
    int n = rnd(12);
    for (int i = 0; i < n; i++)
        m[pick(FIELDS)] = someValue(depth);
    return m;
}

static void fillScope(GenericScope& s)
{
    s.vars["servMgr"] = someObject(0);
    s.vars["channel"] = coin(70) ? someObject(0) : amf0::Value();
    s.vars["request"] = amf0::Value::object({ { "path", pick(std::vector<std::string>{ "/index.html", "/channels.html", "/settings.html", "/console.html", "/x" }) } });
    s.vars["this"] = someObject(0);
    std::vector<amf0::Value> list;
    int n = rnd(4);
    for (int i = 0; i < n; i++)
        list.push_back(someObject(1));
    s.vars["list"] = list;
    s.vars["n"] = pick(std::vector<amf0::Value>{ amf0::Value(2), amf0::Value(0), amf0::Value("3"), amf0::Value("x"), amf0::Value(-1), amf0::Value() });
    s.vars["f"] = amf0::Value::strictArray({ "lambda", amf0::Value::strictArray({ "array", "x" }), amf0::Value::strictArray({ "str", "x", "x" }) });
}

// ---------------------------------------------------------------- テンプレートを作る

static const std::vector<std::string> EXPRS = {
    "servMgr.version", "servMgr.preferredTheme == \"dark\"", "request.path =~ \"/index.html$\"", "request.path !~ \"^/c\"",
    "!servMgr.chat", "length(list)", "length(servMgr.filters)", "inspect(this)", "inspect(servMgr)", "str(this.id, \"-\", n)",
    "channel", "channel.track.title", "this.status != \"FREE\"", "f(\"ab\")", "(lambda([a,b], str(b,a)))(1, 2)",
    "merge(servMgr, {\"k\": 1})", "keys(this)", "removeKey(this, \"id\")", "toQueryString({\"a b\": servMgr.version, \"n\": n})",
    "nth(0, [1,2])", "prop(servMgr, \"lang\")", "servMgr[\"accentColor\"]", "if(n, \"y\", \"n\")", "cond(n, 1, TRUE, 2)",
    "and(servMgr.chat, n)", "or(servMgr.isRoot, \"x\")", "replacePrefix(request.path, \"/\", \"#\")",
    "replaceSuffix(request.path, \".html\", \"\")", "evalString(\"[1,2]\")", "define(v, n)", "v", "loop.index",
    "{\"a\":[1,{\"b\":null}]}", "true", "FALSE", "null", "12345678901", "'q\\'x'", "\"<>&\\\"\"", "page.a",
    "length(1)", "prop(1, 2)", "nth(\"a\", [1])", "x ==", "(", "a b", "servMgr.version =~ \"(\"",
};

static const std::vector<std::string> BODY = { "text ", "<b>{</b>", "\\{$", "}", "\n", " x{y} ", "{@else}", "{@end}" };

static std::string genTemplate(int depth)
{
    std::string s;
    int n = 1 + rnd(6);
    for (int i = 0; i < n; i++)
    {
        int r = rnd(12);
        const std::string& e = pick(EXPRS);
        if (r < 3)
            s += std::string("{") + "$!\\"[rnd(3)] + e + "}";
        else if (r < 5 && depth < 3)
        {
            s += "{@if " + e + "}" + genTemplate(depth + 1);
            if (coin(40)) s += "{@elsif " + pick(EXPRS) + "}" + genTemplate(depth + 1);
            if (coin(40)) s += "{@else}" + genTemplate(depth + 1);
            s += "{@end}";
        }
        else if (r == 5 && depth < 3)
            s += "{@foreach " + pick(std::vector<std::string>{ "list", "servMgr.filters", "[1,2,3]", "keys(servMgr)", "n", "this" }) + "}" + genTemplate(depth + 1) + "{@end}";
        else if (r == 6 && depth < 3)
            s += "{@let v = " + e + ", w = n}" + genTemplate(depth + 1) + "{@end}";
        else if (r == 7 && depth < 3)
            s += "{@fragment " + pick(std::vector<std::string>{ "a", "b", "" }) + "}" + genTemplate(depth + 1) + "{@end}";
        else if (r == 8 && depth < 3)
            s += "{@loop " + pick(std::vector<std::string>{ "n", "3", "servMgr.chat" }) + "}" + genTemplate(depth + 1) + "{@end}";
        else if (r == 9)
            s += "{@" + pick(std::vector<std::string>{ "unknown", "if", "foreach", "let", "end", "else" }) + pick(std::vector<std::string>{ " ", "}", "\t", "" });
        else
            s += pick(BODY);
    }
    return s;
}

static std::string mutate(std::string s)
{
    int k = 1 + rnd(4);
    for (int j = 0; j < k; j++)
    {
        int i = rnd((int) s.size() + 1);
        switch (rnd(6))
        {
        case 0: if (i < (int) s.size()) s[i] = "{}$@!\\\"'()[],=~ \n"[rnd(17)]; break;
        case 1: if (i < (int) s.size()) s[i] = (char) (32 + rnd(95)); break;
        case 2: s.insert(i, 1, "{}$@\\\"'"[rnd(7)]); break;
        case 3: if (i < (int) s.size()) s.erase(i, 1); break;
        case 4: s.resize(i); break;
        default:
        {
            // 別の場所の一部を差し込む
            int a = rnd((int) s.size() + 1), len = rnd(40);
            s.insert(i, s.substr(a, len));
            break;
        }
        }
    }
    return s;
}

// ---------------------------------------------------------------- 1 件

struct Case
{
    std::string input;
    std::string args;
    std::string fragment;
    int seed;
};

static std::string dumpScope(GenericScope& s)
{
    std::string r;
    for (auto& p : s.vars)
    {
        try { r += p.first + "=" + p.second.inspect() + ";"; }
        catch (std::exception& e) { r += p.first + "=?;"; }
    }
    return r;
}

static std::string run(bool rust, const Case& c)
{
    rng.seed(c.seed);
    Template t(c.args);
    GenericScope locals;
    fillScope(locals);
    t.prependScope(locals);
    t.selectedFragment = c.fragment;

    StringStream in(c.input), out;
    std::string res;
    try {
        int code = rust ? (int) rustbridge::templateCall(&t, &in, &out, PCRS_TMPL_READ_TEMPLATE, true).number()
                        : t.readTemplate(in, &out);
        res = "code=" + std::to_string(code);
    } catch (StreamException& e) {
        res = std::string("StreamException:") + e.msg;
    } catch (GeneralException& e) {
        res = std::string("GeneralException:") + e.msg;
    } catch (std::out_of_range& e) {
        res = std::string("out_of_range:") + e.what();
    } catch (std::invalid_argument& e) {
        res = std::string("invalid_argument:") + e.what();
    } catch (std::regex_error& e) {
        res = std::string("regex_error:") + e.what();
    } catch (std::runtime_error& e) {
        res = std::string("runtime_error:") + e.what();
    } catch (std::exception& e) {
        res = std::string("exception:") + e.what();
    }
    // 例外で中断したとき、C++ 版は破棄されたスコープへのポインタを m_scopes に残す (未定義動作)。
    // Rust 版 (rusttemplate.h) は取り除くので、スコープの数は正常に終わったときだけ比べる。
    std::string scopes = res.compare(0, 5, "code=") == 0 ? std::to_string(t.m_scopes.size()) : "-";
    return res + "\npos=" + std::to_string(in.getPosition()) + " frag=" + t.currentFragment +
           " scopes=" + scopes + "\nout=" + out.str() + "\nlocals=" + dumpScope(locals) + "\n";
}

// Rust 版だけで確かめる (C++ 版は未定義動作や、スタックを使い果たして落ちることがある)
static std::string rustOnlyReason(const std::string& rs)
{
    if (rs.find("GeneralException:nth: index out of range") != std::string::npos)
        return "nth の範囲外 (C++ 版は未定義動作)";
    if (rs.find("GeneralException:Template: nesting too deep") != std::string::npos)
        return "入れ子が深すぎる (C++ 版は上限なし)";
    return "";
}

int main(int argc, char** argv)
{
    int count = argc > 1 ? atoi(argv[1]) : 20000;
    std::vector<std::string> corpus;
    for (int i = 2; i < argc; i++)
    {
        std::ifstream f(argv[i], std::ios::binary);
        std::stringstream ss;
        ss << f.rdbuf();
        corpus.push_back(ss.str());
    }

    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();

    long same = 0, bad = 0, withOutput = 0, withError = 0;
    std::map<std::string, long> explained;
    std::mt19937 gen(4242);
    for (int i = 0; i < count; i++)
    {
        rng.seed(gen());
        Case c;
        int kind = rnd(10);
        if (kind < 3 && !corpus.empty())
            c.input = pick(corpus);
        else if (kind < 5 && !corpus.empty())
            c.input = mutate(pick(corpus));
        else if (kind < 8)
            c.input = genTemplate(0);
        else
            c.input = mutate(genTemplate(0));
        c.args = pick(std::vector<std::string>{ "", "a=1", "fragment=a", "a=%E3%81%82&b" });
        c.fragment = pick(std::vector<std::string>{ "", "", "", "a", "b" });
        c.seed = (int) gen();

        std::string b = run(true, c);
        std::string reason = rustOnlyReason(b);
        if (!reason.empty())
        {
            explained[reason]++;
            continue;
        }
        std::string a = run(false, c);
        if (a == b)
        {
            same++;
            if (a.find("\nout=\n") == std::string::npos) withOutput++;
            if (a.find("code=") != 0) withError++;
            continue;
        }
        if (++bad <= 3)
        {
            char fn[64];
            snprintf(fn, sizeof fn, "/tmp/diff_template_%ld.txt", bad);
            FILE* f = fopen(fn, "wb");
            if (f) { fwrite(c.input.data(), 1, c.input.size(), f); fclose(f); }
            printf("--- 違い (入力 %s, args=%s fragment=%s)\nC++:\n%.2000s\nRust:\n%.2000s\n", fn, c.args.c_str(), c.fragment.c_str(), a.c_str(), b.c_str());
        }
    }
    printf("template: %d 件、一致 %ld (うち出力あり %ld、例外 %ld)", count, same, withOutput, withError);
    for (auto& e : explained)
        printf("、%s %ld", e.first.c_str(), e.second);
    printf("、説明のつかない違い %ld\n", bad);
    return bad ? 1 : 0;
}
