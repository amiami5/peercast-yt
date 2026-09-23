// XML パーサー (core/common/xml.cpp) の、C++ 版と Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の XML クラス。Rust 版は、本番と
// 同じ橋渡しのコード (core/common/rustbridge.h の XmlReader と parseXmlAttributes) を通して呼ぶ。
// ただし Rust 版の木を作るときに呼ばれる XML::Node のコンストラクタは cxxcore.a のもの
// (属性の解析が C++ 版) なので、属性の解析は別に parseXmlAttributes 同士で比べる。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>

#include "rustbridge.h"
#include "sstream.h"
#include "stream.h"
#include "xml.h"
#include "../../../tests/mocksys.h"

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

static void report(const char* what, const std::string& in, const std::string& a, const std::string& b)
{
    if (g_bad++ < 20)
        printf("  [違い] %s 入力=\"%s\"\n    C++ =%s\n    Rust=%s\n", what, show(in.substr(0, 300)).c_str(),
               show(a.substr(0, 400)).c_str(), show(b.substr(0, 400)).c_str());
}

template <typename F>
static std::string guarded(F f)
{
    try { return f(); }
    catch (StreamException& e) { return std::string("StreamException:") + e.what(); }
    catch (std::exception& e) { return std::string("exception:") + e.what(); }
}

static std::string dumpNode(XML::Node* n)
{
    std::string r = "<";
    for (int i = 0; i < n->numAttr; i++)
        r += std::string(n->getAttrName(i)) + "=" + n->getAttrValue(i) + ";";
    if (n->contData) r += "|" + std::string(n->contData);
    for (XML::Node* c = n->child; c; c = c->sibling) r += dumpNode(c);
    return r + ">";
}

static std::string dumpXml(XML& xml)
{
    return xml.root ? dumpNode(xml.root) : "(no root)";
}

// ---------------------------------------------------------------- XML::read

static void readCase(const std::string& data)
{
    for (int kind = 0; kind < 2; kind++)
    {
        g_cases++;
        XML x1, x2;
        std::string a, b;
        int pa, pb;
        if (kind == 0)
        {
            std::string c1 = data, c2 = data;
            MemoryStream m1(&c1[0], c1.size()), m2(&c2[0], c2.size());
            a = guarded([&]() { x1.read(m1); return std::string("ok"); }); pa = m1.getPosition();
            b = guarded([&]() { rustbridge::XmlReader(x2).read(m2); return std::string("ok"); }); pb = m2.getPosition();
        }
        else
        {
            StringStream s1(data), s2(data);
            a = guarded([&]() { x1.read(s1); return std::string("ok"); }); pa = s1.getPosition();
            b = guarded([&]() { rustbridge::XmlReader(x2).read(s2); return std::string("ok"); }); pb = s2.getPosition();
        }
        a += " @" + std::to_string(pa) + " " + dumpXml(x1);
        b += " @" + std::to_string(pb) + " " + dumpXml(x2);
        if (a != b)
            report(kind ? "XML::read (StringStream)" : "XML::read (MemoryStream)", data, a, b);
    }
}

// ---------------------------------------------------------------- setAttributes

static void attrCase(const std::string& tag)
{
    g_cases++;
    std::string ctag = tag.c_str(); // Node("%s", ...) に渡るのは NUL の手前まで
    std::string a = guarded([&]() {
        XML::Node n("%s", ctag.c_str());
        std::string r;
        for (int i = 0; i < n.numAttr; i++)
            r += std::string(n.getAttrName(i)) + "=" + n.getAttrValue(i) + ";";
        return r;
    });
    std::string b = guarded([&]() {
        char* data = nullptr;
        XML::Node::Attribute* attr = nullptr;
        int num = 0;
        rustbridge::parseXmlAttributes(ctag.c_str(), data, attr, num);
        std::string r;
        for (int i = 0; i < num; i++)
            r += std::string(&data[attr[i].namePos]) + "=" + &data[attr[i].valuePos] + ";";
        free(data);
        delete[] attr;
        return r;
    });
    if (a != b)
        report("setAttributes", tag, a, b);
}

// ---------------------------------------------------------------- getBinaryContent

// C++ 版が終端を越えて読むかどうか (16 進数字の組の 2 文字目が終端の NUL になるか)
static bool cxxOverreads(const std::string& s)
{
    size_t i = 0;
    while (i < s.size())
    {
        char c = s[i];
        if (c == ' ' || c == '\r' || c == '\n' || c == '\t') { i++; continue; }
        if (i + 1 >= s.size()) return true;
        i += 2;
    }
    return false;
}

static void binaryCase(const std::string& content, int size)
{
    g_cases++;
    std::string c = content.c_str();
    if (cxxOverreads(c)) { g_known++; return; } // C++ 版は未定義の動作 (Rust 版は NUL とみなして止まる)

    auto run = [&](bool rust) {
        return guarded([&]() {
            XML::Node n("x");
            n.setContent(c.c_str());
            std::string buf(size > 0 ? size : 0, '\0');
            int got;
            if (rust)
            {
                pcrs_buf out = {nullptr, 0};
                if (pcrs_xml_binary_content(reinterpret_cast<const uint8_t*>(c.data()), c.size(), size > 0 ? size : 0, &out) != 0)
                    throw StreamException("Too much binary data");
                rustbridge::RustBuf owner(out);
                memcpy(&buf[0], out.ptr, out.len);
                got = out.len;
            }
            else
                got = n.getBinaryContent(&buf[0], size);
            return show(buf.substr(0, got));
        });
    };
    std::string a = run(false), b = run(true);
    if (a != b)
        report("getBinaryContent", content, a, b);
}

// ---------------------------------------------------------------- 入力の生成

static const char* kSamples[] = {
    "<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n<peercast><servent uptime=\"10\"/>\n<bandwidth out=\"1\" in=\"2\"/></peercast>",
    "<ASX version=\"3.0\"><Entry><Ref href=\"http://example.com/a.asf\"/></Entry></ASX>",
    "<a>text<b>inner</b>tail</a>",
    "<!-- comment --><x y=\"1\" z=\"2\">c</x>",
    "<yp name=\"test\"><host ip=\"1.2.3.4\" port_open=\"1\" speed=\"100\" over=\"0\"/><uptest checkable=\"1\" remain=\"0\"/></yp>",
    "</a>", "<>", "<?html?>", "<a", "<a/", "x<a>", "<a x=1>", "<a x=\"1>", "<a b c>",
};

static const std::string kAlpha = "<>/?!=\" \t\r\nabcxyzXML-";

static std::string mutate(std::string s)
{
    int n = 1 + rng() % 4;
    for (int i = 0; i < n; i++)
    {
        size_t pos = s.empty() ? 0 : rng() % (s.size() + 1);
        switch (rng() % 5)
        {
        case 0: if (pos < s.size()) s.erase(pos, 1); break;
        case 1: s.insert(pos, 1, kAlpha[rng() % kAlpha.size()]); break;
        case 2: if (pos < s.size()) s[pos] = kAlpha[rng() % kAlpha.size()]; break;
        case 3: s.insert(pos, 1, (char)(rng() % 256)); break;
        case 4: s.resize(pos); break;
        }
    }
    return s;
}

static std::string randomText(const std::string& alpha, size_t maxlen)
{
    std::string s;
    size_t len = rng() % maxlen;
    for (size_t i = 0; i < len; i++)
        s += (rng() % 5) ? alpha[rng() % alpha.size()] : (char)(rng() % 256);
    return s;
}

int main(int argc, char** argv)
{
    sys = new MockSys();
    long iterations = argc > 1 ? atol(argv[1]) : 100000;
    const size_t nSamples = sizeof(kSamples) / sizeof(kSamples[0]);

    for (auto s : kSamples) readCase(s);
    readCase(std::string("<a>") + std::string(50000, 'x') + "</a>");

    const char* attrSamples[] = {"host ip=\"1.2.3.4\" port = \"7144\"", "tag", "tag a b", "tag a=1", "t a=\"xyz",
                                 "t a=\"1\"b=\"2\"", "t  a =  \"\" ", "t a=\"=\"", "t\ta\n=\"x\"", ""};
    for (auto s : attrSamples) attrCase(s);

    for (long it = 0; it < iterations; it++)
    {
        readCase(rng() % 3 ? mutate(kSamples[rng() % nSamples]) : randomText(kAlpha, 60));
        attrCase(rng() % 2 ? mutate(attrSamples[rng() % 10]) : randomText("ab=\" \t\r\n", 30));
        binaryCase(randomText("0123456789ABCDEFabcdef \n", 30), rng() % 20);
    }

    printf("比較件数 %ld、既知の違い (C++ 版が未定義の動作になる入力) %ld、説明のつかない違い %ld\n", g_cases, g_known, g_bad);
    return g_bad ? 1 : 0;
}
