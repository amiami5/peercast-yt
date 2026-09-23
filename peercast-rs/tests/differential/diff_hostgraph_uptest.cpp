// リレーツリー (core/common/hostgraph.cpp) と帯域測定の通信しない部分 (core/common/uptest.cpp) の、
// C++ 版と Rust 版の差分テスト。C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) のクラス。
// Rust 版は C ABI を直接呼び、WITH_RUST_CORE のビルドの橋渡しと同じやり方で結果を当てはめる。
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <random>
#include <string>
#include <vector>

#include "chanhit.h"
#include "hostgraph.h"
#include "peercast_rs.h"
#include "uptest.h"
#include "../../../tests/mockpeercast.h"

using json = nlohmann::json;

static long g_bad = 0, g_cases = 0;
static std::mt19937 rng(20260925);
static std::string g_log;

static void logv(const char* k, const char* f, va_list ap)
{
    char b[1024];
    vsnprintf(b, sizeof b, f, ap);
    g_log += std::string(k) + b + "\n";
}
extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("D:", f, ap); va_end(ap); }
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("W:", f, ap); va_end(ap); }
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

static std::string take(pcrs_buf b)
{
    std::string s(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return s;
}

static void report(const char* what, const std::string& a, const std::string& b, const std::string& ctx = "")
{
    g_cases++;
    if (a != b && g_bad++ < 20)
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", what, show(ctx.substr(0, 3000)).c_str(),
               show(a.substr(0, 3000)).c_str(), show(b.substr(0, 3000)).c_str());
}

static unsigned R(unsigned n) { return rng() % n; }
static MockSys* msys() { return static_cast<MockSys*>(sys); }

// ---------------------------------------------------------------- hostgraph

static IP randIp()
{
    switch (R(6))
    {
    case 0: return IP();
    case 1: return IP(0u);
    case 2: return IP((10u << 24) | R(3));
    case 3: return IP((192u << 24) | (168u << 16) | R(3));
    case 4: { in6_addr a = {}; a.s6_addr[0] = 0x20; a.s6_addr[1] = 0x01; a.s6_addr[15] = R(3); return IP(a); }
    default: return IP(rng());
    }
}

static Host randHost(const std::vector<Host>& pool)
{
    static const unsigned short ports[] = { 0, 7144, 7145 };
    switch (R(6))
    {
    case 0: return Host();
    case 1: return Host(randIp(), ports[R(3)]);
    case 2: return Host(pool[R(pool.size())].ip, ports[R(3)]);
    default: return pool[R(pool.size())];
    }
}

static ChanHit randHit(const std::vector<Host>& pool)
{
    ChanHit h;
    h.rhost[0] = randHost(pool);
    h.rhost[1] = R(2) ? h.rhost[0] : randHost(pool);
    h.host = h.rhost[0];
    h.uphost = R(3) == 0 ? Host() : randHost(pool);
    h.numListeners = R(10);
    h.numRelays = R(10);
    h.version = R(3) ? 0 : 1200 + R(300);
    h.versionVP = R(2) ? 0 : R(30);
    h.versionExNumber = R(2) ? 0 : R(1000);
    static const char* prefixes[] = { "  ", "YT", "ST" };
    memcpy(h.versionExPrefix, prefixes[R(3)], 2);
    for (auto& b : h.sessionID.id) b = rng();
    h.firewalled = R(2); h.tracker = R(3) == 0; h.recv = R(2);
    h.direct = R(2); h.relay = R(2); h.cin = R(2);
    return h;
}

static std::string dumpHost(const Host& h) { return h.ip.str() + ":" + std::to_string(h.port); }

static std::string dumpGraph(HostGraph& g)
{
    std::string s = "hit:";
    for (auto& p : g.m_hit)
        s += " " + dumpHost(p.first.first) + "/" + dumpHost(p.first.second) + "=" + (std::string) p.second.sessionID
             + (p.second.next ? "+" : "");
    s += "\nroots:";
    for (auto& r : g.m_roots) s += " " + dumpHost(r.first) + "/" + dumpHost(r.second);
    s += "\nchildren:";
    for (auto& p : g.m_children)
    {
        s += " [" + dumpHost(p.first.first) + "/" + dumpHost(p.first.second) + ":";
        for (auto& c : p.second) s += " " + dumpHost(c.first) + "/" + dumpHost(c.second);
        s += "]";
    }
    return s;
}

static pcrs_host rsHost(const Host& h)
{
    pcrs_host r;
    in6_addr a = h.ip.serialize();
    memcpy(r.ip, a.s6_addr, 16);
    r.port = h.port;
    return r;
}

// hostgraph.cpp の WITH_RUST_CORE のコンストラクターと同じ (結果を g に入れる)
static void rsBuild(HostGraph& g, const ChanHit& self, ChanHitList* hitList)
{
    g.m_hit.clear();
    g.m_children.clear();
    g.m_roots.clear();

    std::vector<const ChanHit*> hits = { &self };
    for (auto p = hitList->hit; p; p = p->next)
    {
        LOG_DEBUG("HostGraph: %s", p->rhost[0].str().c_str());
        hits.push_back(p.get());
    }

    std::vector<pcrs_graph_node> nodes;
    for (auto h : hits)
        nodes.push_back({ { rsHost(h->rhost[0]), rsHost(h->rhost[1]) }, rsHost(h->uphost) });

    std::vector<size_t> index(hits.size());
    std::vector<ptrdiff_t> parent(hits.size());
    size_t count = pcrs_hostgraph_build(nodes.data(), nodes.size(), index.data(), parent.data());

    std::vector<std::pair<Host, Host>> ids;
    for (size_t k = 0; k < count; k++)
    {
        ids.push_back(g.id(*hits[index[k]]));
        g.m_hit[ids[k]] = *hits[index[k]];
        if (index[k] != 0)
            g.m_hit[ids[k]].next = nullptr;
    }
    for (size_t k = 0; k < count; k++)
    {
        if (parent[k] < 0)
            g.m_roots.push_back(ids[k]);
        else
            g.m_children[ids[parent[k]]].push_back(ids[k]);
    }
}

static void graphCase()
{
    std::vector<Host> pool;
    int np = 1 + R(6);
    for (int i = 0; i < np; i++) pool.push_back(Host(randIp(), R(2) ? 7144 : R(3)));

    ChanHit self = randHit(pool);
    if (R(4) == 0) self.next = std::make_shared<ChanHit>();

    int n = R(14);
    auto list = std::make_shared<ChanHitList>();
    for (int i = 0; i < n; i++)
    {
        auto c = std::make_shared<ChanHit>(randHit(pool));
        c->next = list->hit;
        list->hit = c;
    }
    auto empty = std::make_shared<ChanHitList>();

    g_log.clear();
    HostGraph a(self, list.get());
    std::string da = dumpGraph(a);
    std::string ja = json(a.getRelayTree()).dump();
    std::string la = g_log;

    HostGraph b(self, empty.get());
    g_log.clear();
    rsBuild(b, self, list.get());
    std::string db = dumpGraph(b);
    std::string jb = json(b.getRelayTree()).dump();
    std::string lb = g_log;

    report("HostGraph", da, db);
    report("getRelayTree", ja, jb, da);
    report("HostGraph ログ", la, lb);
}

// ---------------------------------------------------------------- uptest

static std::string randBytes(size_t n)
{
    std::string s;
    for (size_t i = 0; i < n; i++) s += (char) rng();
    return s;
}

static std::string randValue()
{
    static const char* vals[] = { "", "1", "0", "YP", "999", "example.com", "a b", "\xe6\x97\xa5", "/yp/uptest.cgi",
                                  "x=y", "'", "&amp;" };
    switch (R(8))
    {
    case 0: return randBytes(R(6));
    case 1: return std::string("a\0b", 3);
    default: return vals[R(sizeof vals / sizeof vals[0])];
    }
}

static std::string randAttrName()
{
    static const char* names[] = { "name", "nameX", "NAME", "Name", "nam", "ip", "ipv6", "IP", "port_open",
                                   "port_openz", "port", "port_x", "speed", "over", "checkable", "remain",
                                   "addr", "object", "post_size", "limit", "interval", "enabled", "x", "" };
    return names[R(sizeof names / sizeof names[0])];
}

static std::string randAttrs()
{
    std::string s;
    int n = R(9);
    for (int i = 0; i < n; i++)
    {
        s += R(10) ? " " : (R(2) ? "\t" : "  ");
        s += randAttrName();
        switch (R(12))
        {
        case 0: s += "=" + randValue(); break;               // 引用符なし
        case 1: break;                                       // 値なし
        case 2: s += "=\"" + randValue(); break;             // 閉じていない
        case 3: s += " = \"" + randValue() + "\""; break;
        case 4: s += "==\"" + randValue() + "\""; break;
        default: s += "=\"" + randValue() + "\""; break;
        }
    }
    return s;
}

static std::string randTag()
{
    static const char* tags[] = { "yp", "YP", "Yp", "host", "HOST", "uptest", "Uptest", "uptest_srv", "uptest_srvx",
                                  "yp4g", "x", "ypx", "hos" };
    return tags[R(sizeof tags / sizeof tags[0])];
}

// uptestendpoint_unittest の文書に近いものを、ところどころ壊して作る
static std::string randDoc()
{
    static const char* good[] = {
        "<yp name=\"YP\"/>",
        "<host ip=\"192.168.0.1\" port_open=\"1\" speed=\"999\" over=\"0\"/>",
        "<uptest checkable=\"1\" remain=\"0\"/>",
        "<uptest_srv addr=\"example.com\" port=\"443\" object=\"/yp/uptest.cgi\" post_size=\"250\" limit=\"3000\" "
        "interval=\"15\" enabled=\"1\"/>",
    };
    std::string s;
    if (R(3) == 0) s += R(4) ? "<?xml version=\"1.0\"?>" : "<?foo?>";
    bool wrap = R(5) != 0;
    if (wrap) s += "<yp4g>";
    int n = R(9);
    int opened = 0;
    for (int i = 0; i < n; i++)
    {
        switch (R(14))
        {
        case 0: s += "<" + randTag() + randAttrs() + ">"; opened++; break;
        case 1: s += "</" + randTag() + ">"; opened--; break;
        case 2: s += "<" + randTag() + randAttrs() + "/>"; break;
        case 3: s += "<!-- c > -->"; break;
        case 4: s += randValue(); break;
        case 5: s += std::string(R(2) ? "<" : ">"); break;
        case 6: s += "<" + randTag() + " " + std::string(8150 + R(100), 'a') + "=\"v\"/>"; break;
        case 7: s += "<" + std::string(1, '\0') + "yp name=\"z\"/>"; break;
        default: s += good[R(4)]; break;
        }
    }
    for (int i = 0; i < 4; i++)
        if (R(4)) s += good[i];
    if (wrap && R(4)) s += "</yp4g>";
    if (R(20) == 0 && !s.empty()) s.resize(R(s.size()));
    (void) opened;
    return s;
}

static std::string infoStr(const UptestInfo& i)
{
    return i.name + "|" + i.ip + "|" + i.port_open + "|" + i.speed + "|" + i.over + "|" + i.checkable + "|" + i.remain
           + "|" + i.addr + "|" + i.port + "|" + i.object + "|" + i.post_size + "|" + i.limit + "|" + i.interval + "|"
           + i.enabled;
}

// uptest.cpp の WITH_RUST_CORE の readInfo と同じ (例外は文字列にする)
static std::string rsReadInfo(const std::string& body)
{
    pcrs_buf out = { nullptr, 0 };
    switch (pcrs_uptest_read_info(reinterpret_cast<const uint8_t*>(body.data()), body.size(), &out))
    {
    case 0: break;
    case 3: return "E:Tag too long";
    case 4: return "E:Content too big";
    case 5: return "E:Not XML document";
    case 6: return "E:Unexpected end tag";
    case 7: return "E:Too many attributes";
    case 8: return "E:Bad tag value";
    default: return "E:NULL";
    }
    UptestInfo info;
    std::string* fields[] = {
        &info.name, &info.ip, &info.port_open, &info.speed, &info.over, &info.checkable, &info.remain,
        &info.addr, &info.port, &info.object, &info.post_size, &info.limit, &info.interval, &info.enabled,
    };
    std::string s = take(out);
    size_t pos = 0;
    for (auto f : fields)
    {
        size_t end = s.find('\0', pos);
        *f = s.substr(pos, end - pos);
        pos = end + 1;
    }
    return infoStr(info);
}

static std::string cxxReadInfo(const std::string& body)
{
    try
    {
        return infoStr(UptestEndpoint::readInfo(body));
    }
    catch (std::exception& e)
    {
        std::string w = e.what();
        if (w.find("non-null assertion failed on line ") == 0)
            return "E:NULL";
        return "E:" + w;
    }
}

static std::map<std::string, long> g_kinds;

static void uptestCase()
{
    std::string doc = randDoc();
    std::string a = cxxReadInfo(doc), b = rsReadInfo(doc);
    g_kinds[a.compare(0, 2, "E:") == 0 ? a : "成功"]++;
    report("readInfo", a, b, doc);

    // postURL
    UptestInfo i;
    i.addr = randValue();
    i.port = randValue();
    i.object = randValue();
    std::string u = take(pcrs_uptest_post_url(reinterpret_cast<const uint8_t*>(i.addr.data()), i.addr.size(),
                                              reinterpret_cast<const uint8_t*>(i.port.data()), i.port.size(),
                                              reinterpret_cast<const uint8_t*>(i.object.data()), i.object.size()));
    report("postURL", i.postURL(), u);

    // isReady
    UptestEndpoint e("http://x/");
    e.status = (UptestEndpoint::Status) R(3);
    static const unsigned edges[] = { 0, 1, 59, 60, 61, 62, 0x7fffffffu, 0x80000000u, 0xffffffffu, 0xffffffc4u };
    e.lastTriedAt = R(2) ? edges[R(10)] : rng();
    msys()->time = R(2) ? e.lastTriedAt + edges[R(10)] : rng();
    report("isReady", std::to_string(e.isReady()),
           std::to_string(pcrs_uptest_is_ready(e.status, e.lastTriedAt, msys()->time)));

    // addURL (URL の妥当性は C++ の URI が決める)
    static const char* urls[] = { "http://a/", "http://a/", "https://a/", "ftp://x", "http://bayonet.ddo.jp/sp/yp4g.xml",
                                  "not a url", "", "http://", "HTTP://a/", "http://a:80/x?y#z", "http://a", "http:a",
                                  "http://[::1]:7144/" };
    UptestServiceRegistry reg;
    std::vector<std::string> mine;
    std::string ra, rb;
    int n = R(8);
    for (int k = 0; k < n; k++)
    {
        std::string url = R(10) ? urls[R(sizeof urls / sizeof urls[0])] : std::string("http://a/\0b", 11);
        auto r = reg.addURL(url);
        ra += std::to_string(r.first) + r.second + ";";

        URI uri(url);
        bool valid = uri.isValid();
        std::string scheme = valid ? uri.scheme() : "";
        std::vector<pcrs_bytes> existing;
        for (auto& m : mine)
            existing.push_back({ reinterpret_cast<const uint8_t*>(m.data()), m.size() });
        const char* err = pcrs_uptest_check_add_url(valid, reinterpret_cast<const uint8_t*>(scheme.data()),
                                                    scheme.size(), reinterpret_cast<const uint8_t*>(url.data()),
                                                    url.size(), existing.data(), existing.size());
        if (err)
            rb += "0" + std::string(err) + ";";
        else
        {
            mine.push_back(url);
            rb += "1;";
        }
    }
    for (auto& s : reg.getURLs()) ra += "|" + s;
    for (auto& s : mine) rb += "|" + s;
    report("addURL", ra, rb);
}

static void textStatusCase()
{
    // textStatus は uptest.cpp の static 関数なので getState を通して比べる
    UptestServiceRegistry reg;
    for (int s = 0; s < 3; s++)
    {
        reg.addURL("http://a" + std::to_string(s) + "/");
        reg.m_providers.back().status = (UptestEndpoint::Status) s;
    }
    std::string a = reg.getState().inspect();
    std::string b;
    for (int s = 0; s < 3; s++) b += std::string(pcrs_uptest_text_status(s)) + ";";
    b += pcrs_uptest_text_status(3) ? "?" : "null";
    std::string want;
    for (auto s : { "Untried", "Success", "Error" })
        if (a.find(s) != std::string::npos) want += std::string(s) + ";";
    report("textStatus", want + "null", b, a);
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 100000;
    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();

    textStatusCase();
    for (long i = 0; i < n; i++)
    {
        graphCase();
        uptestCase();
    }
    for (auto& k : g_kinds) printf("readInfo の結果 %s: %ld 件\n", k.first.c_str(), k.second);
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
