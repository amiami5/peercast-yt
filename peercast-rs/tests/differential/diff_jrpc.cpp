// JSON-RPC の API (core/common/jrpc.cpp の JrpcApi) の C++ 版と Rust 版の差分テスト。C++ 版は Rust を
// 使わずにビルドしたコア一式 (cxxcore.a) の JrpcApi::call。Rust 版は WITH_RUST_CORE のビルドと同じ
// 橋渡し (core/common/rustjrpc.h) で pcrs_jrpc_call を呼ぶ。
//
// 毎回、同じ乱数の種から同じ状態 (チャンネル、サーバント、ヒットリスト、ログ、イエローページの
// 一覧、設定、状態のファイル) を作り直し、それぞれに同じ要求を与えて、応答 (か例外)、ログと横取り
// した呼び出し、あとの状態を比べる。
//
// 要求は、メソッドごとに作った引数 (型の違うもの、足りないもの、余計なものを含む) と、それを
// 数バイト変えたもの。id には乱数の JSON を使い、応答で書き出される値 (浮動小数点数など) も比べる。
// getState が読む inspect() の結果も横取りして乱数の JSON (とその変異) を返し、nlohmann の構文解析の
// 例外の文言も比べる。
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <map>
#include <memory>
#include <random>
#include <string>
#include <unistd.h>
#include <vector>

#include "chanmgr.h"
#include "channel.h"
#include "chandir.h"
#include "jrpc.h"
#include "logbuf.h"
#include "peercast_rs.h"
#include "rustjrpc.h"
#include "servent.h"
#include "servmgr.h"
#include "str.h"
#include "../../../tests/mockclientsocket.h"
#include "../../../tests/mockpeercast.h"

static long g_bad = 0, g_cases = 0, g_overflow = 0;
static std::mt19937 rng(20260927);   // 要求を作る
static std::mt19937 wrng;            // 状態を作る (両方で同じ種から)
static unsigned R(unsigned n) { return rng() % n; }
static unsigned W(unsigned n) { return wrng() % n; }

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

// ---------------------------------------------------------------- 横取り

static std::string g_rec;       // ログと横取りした呼び出し
static std::string g_inspect;   // amf0::Value::inspect の結果
static bool g_createFails = false;

static void rec(const char* kind, const char* fmt, va_list ap)
{
    va_list aq;
    va_copy(aq, ap);
    int n = vsnprintf(nullptr, 0, fmt, aq);
    va_end(aq);
    std::string s(n > 0 ? n : 0, '\0');
    if (n > 0)
        vsnprintf(&s[0], n + 1, fmt, ap);
    g_rec += std::string(kind) + ":" + s + "\n";
}

#define WRAP_LOG(sym, kind) \
    extern "C" void sym(const char* fmt, ...) { va_list ap; va_start(ap, fmt); rec(kind, fmt, ap); va_end(ap); }
WRAP_LOG(__wrap__Z9LOG_TRACEPKcz, "T")
WRAP_LOG(__wrap__Z9LOG_DEBUGPKcz, "D")
WRAP_LOG(__wrap__Z8LOG_INFOPKcz, "I")
WRAP_LOG(__wrap__Z8LOG_WARNPKcz, "W")
WRAP_LOG(__wrap__Z9LOG_ERRORPKcz, "E")

static std::string infoStr(const ChanInfo& i)
{
    return show(std::string(i.name.c_str()) + "|" + i.desc.c_str() + "|" + i.genre.c_str() + "|" + i.url.c_str() + "|" +
                i.comment.c_str() + "|" + i.track.contact.c_str() + "|" + i.track.title.c_str() + "|" +
                i.track.artist.c_str() + "|" + i.track.album.c_str() + "|" + i.track.genre.c_str() + "|" +
                std::to_string(i.bitrate) + "|" + i.contentType.c_str() + "|" + i.MIMEType.c_str() + "|" +
                i.id.str() + "|" + i.bcID.str());
}

extern "C" std::shared_ptr<Channel> __real__ZN7ChanMgr13createChannelER8ChanInfoPKc(ChanMgr*, ChanInfo&, const char*);
extern "C" std::shared_ptr<Channel> __wrap__ZN7ChanMgr13createChannelER8ChanInfoPKc(ChanMgr* self, ChanInfo& info, const char* mount)
{
    g_rec += "createChannel " + infoStr(info) + "\n";
    if (g_createFails)
        return nullptr;
    return __real__ZN7ChanMgr13createChannelER8ChanInfoPKc(self, info, mount);
}

// 本物はスレッドを作ろうとする
extern "C" void __wrap__ZN7Channel8startURLEPKc(Channel* c, const char* u)
{
    g_rec += "startURL " + show(u) + "\n";
    c->sourceURL.set(u);
}

extern "C" void __wrap__ZN7ServMgr17checkFirewallIPv6Ev(ServMgr*) { g_rec += "checkFirewallIPv6\n"; }

extern "C" void __wrap__ZN7ChanMgr18findAndPlayChannelER8ChanInfob(ChanMgr*, ChanInfo& info, bool keep)
{
    g_rec += "findAndPlayChannel " + infoStr(info) + " " + std::to_string(keep) + "\n";
}

extern "C" bool __wrap__ZN7Channel10updateInfoERK8ChanInfo(Channel* c, const ChanInfo& i)
{
    g_rec += "updateInfo " + c->info.id.str() + " " + infoStr(i) + "\n";
    return true;
}

extern "C" void __wrap__ZN7Servent5abortEv(Servent* s) { g_rec += "abort " + std::to_string(s->serventIndex) + "\n"; }

extern "C" std::string __wrap__ZNK4amf05Value7inspectB5cxx11Ev(const amf0::Value*) { return g_inspect; }

// ---------------------------------------------------------------- 状態

class RateSource : public ChannelSource
{
public:
    explicit RateSource(int r) : rate(r) {}
    void stream(std::shared_ptr<Channel>) override {}
    int getSourceRate() override { return rate; }
    int rate;
};

// C++ 版と Rust 版で str::valid_utf8 の結果が違う文字列 (段階 1 の既知の違い) は使わない
static bool sameValid(const std::string& s)
{
    pcrs_buf b = pcrs_str_valid_utf8(reinterpret_cast<const uint8_t*>(s.data()), s.size());
    std::string r(reinterpret_cast<const char*>(b.ptr), b.len);
    pcrs_buf_free(b);
    return r == str::valid_utf8(s);
}

static std::string wordW()
{
    static const char* words[] = { "", "a", "YP", "http://x/", "日本語", "\x82\xa0\x82\xa2", "'q'", "&amp;",
                                   "a;b", "=", "\"q\"\\", "\t\n", "\xe3\x81", "\xff" };
    switch (W(6))
    {
    case 0: { std::string s; int n = W(12); for (int i = 0; i < n; i++) s += (char)(1 + W(255)); return s; }
    case 1: return std::string(250 + W(20), 'x');
    default: return words[W(sizeof words / sizeof words[0])];
    }
}

// valid_utf8 を通す欄
static std::string vstrW()
{
    for (;;)
    {
        std::string s = wordW();
        if (sameValid(s))
            return s;
    }
}

// LogBuffer::write は、途中までの UTF-8 で終わる行を渡すと終わらない (docs/cpp-known-issues.md)。
// 本物のログ (ADDLOG) は不正な UTF-8 を置き換えてから書くので起きないが、ここでは直接書くので、
// write の 99 バイトずつ切り分けるループをなぞって、進まなくなるものは使わない。
static bool logSafe(const std::string& s)
{
    size_t pos = 0, len = s.size();
    while (len)
    {
        size_t buflen = len < 99 ? len : 99, p = pos;
        while (buflen && p < s.size())
        {
            unsigned char c = s[p];
            size_t charlen = (c & 0x80) == 0 ? 1 : (c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : (c & 0xF8) == 0xF0 ? 4 : 1;
            if (buflen < charlen)
                break;
            p += charlen;
            buflen -= charlen;
        }
        if (p == pos)
            return false;
        len -= p - pos;
        pos = p;
    }
    return true;
}

static GnuID g_ids[4];

static void setIds()
{
    for (int k = 0; k < 4; k++)
        for (int i = 0; i < 16; i++)
            g_ids[k].id[i] = (k == 3) ? 0 : (uint8_t)(0x11 * (k + 1) + i);
}

static Host hostW()
{
    Host h;
    if (W(4))
        h = Host(0x0a000000 + W(4), W(3) ? 7144 : W(65536));
    else if (W(2))
    {
        in6_addr a = {};
        a.s6_addr[0] = 0x20; a.s6_addr[1] = 0x01; a.s6_addr[15] = (uint8_t) W(4);
        h = Host(IP(a), 7144);
    }
    return h;
}

static void randInfoW(ChanInfo& i)
{
    i.name.set(vstrW().c_str()); i.genre.set(vstrW().c_str()); i.url.set(vstrW().c_str());
    i.desc.set(vstrW().c_str()); i.comment.set(vstrW().c_str());
    static const char* types[] = { "MP3", "FLV", "OGG", "UNKNOWN", "", "MKV", "\xff" };
    i.contentType.set(types[W(7)]);
    i.MIMEType.set(W(2) ? "" : wordW().c_str());
    i.track.title.set(vstrW().c_str()); i.track.artist.set(vstrW().c_str());
    i.track.album.set(vstrW().c_str()); i.track.contact.set(vstrW().c_str());
    i.track.genre.set(vstrW().c_str());
    i.id = g_ids[W(4)];
    for (auto& b : i.bcID.id) b = (uint8_t) wrng();
    i.bitrate = W(3) ? W(2000) : wrng();
    i.numSkips = W(3);
    i.lastPlayStart = W(2) ? 0 : 900;
    i.srcProtocol = (ChanInfo::PROTOCOL) W(6);
}

static std::vector<std::shared_ptr<Channel>> g_channels;
static std::vector<Servent*> g_servents;
static Servent* g_origServents;

static void build(unsigned seed)
{
    wrng.seed(seed);
    static_cast<MockSys*>(sys)->time = 1000 + W(100);

    servMgr->maxRelays = W(3) ? W(10) : wrng();
    servMgr->maxDirect = W(3) ? W(10) : wrng();
    servMgr->maxBitrateOut = W(3) ? W(10000) : wrng();
    chanMgr->maxRelaysPerChannel = W(3) ? W(10) : (int) wrng();
    servMgr->logLevel(1 + W(7));
    servMgr->rootHost.set(W(2) ? "" : wordW().c_str());
    servMgr->serverHost = hostW();
    servMgr->serverLocalIP = hostW().ip;
    servMgr->setFirewall(4, (ServMgr::FW_STATE) W(3));
    servMgr->flags.get("randomizeBroadcastingChannelID") = false;

    int nch = W(5);
    for (int k = 0; k < nch; k++)
    {
        auto c = std::make_shared<Channel>();
        randInfoW(c->info);
        static const Channel::STATUS st[] = { Channel::S_RECEIVING, Channel::S_BROADCASTING, Channel::S_SEARCHING,
                                              Channel::S_CONNECTING, Channel::S_ERROR, Channel::S_IDLE, Channel::S_NONE,
                                              Channel::S_WAIT, Channel::S_NOTFOUND, Channel::S_ABORT };
        c->status = st[W(10)];
        c->type = W(2) ? Channel::T_BROADCAST : Channel::T_RELAY;
        c->ipVersion = W(2) ? Channel::IP_V4 : (W(4) ? Channel::IP_V6 : (Channel::IP_VERSION) 5);
        c->sourceURL.set(W(2) ? "" : wordW().c_str());
        c->sourceHost.host = hostW();
        if (W(2))
        {
            auto s = std::make_shared<MockClientSocket>();
            s->host = hostW();
            c->sock = s;
        }
        if (W(2))
            c->sourceData = std::make_shared<RateSource>(W(2) ? W(100000) : (int) wrng());
        c->streamPos = W(2) ? W(100000) : wrng();
        c->thread.m_active = W(2);
        c->next = chanMgr->channel;
        chanMgr->channel = c;
        g_channels.push_back(c);
    }

    int nsv = W(5);
    for (int k = 0; k < nsv; k++)
    {
        Servent* s = new Servent(W(4));
        static const Servent::TYPE types[] = { Servent::T_RELAY, Servent::T_DIRECT, Servent::T_CIN, Servent::T_COUT,
                                               Servent::T_NONE, Servent::T_INCOMING };
        s->type = types[W(6)];
        s->status = (Servent::STATUS) W(13);
        s->chanID = g_ids[W(4)];
        s->agent.set(wordW().c_str());
        s->outputProtocol = (ChanInfo::PROTOCOL) W(6);
        if (W(2))
        {
            auto sock = std::make_shared<MockClientSocket>();
            sock->host = hostW();
            s->sock = sock;
        }
        s->thread.m_active = true;
        s->next = servMgr->servents;
        servMgr->servents = s;
        g_servents.push_back(s);
    }

    int nhl = W(4);
    for (int k = 0; k < nhl; k++)
    {
        ChanInfo info;
        randInfoW(info);
        auto l = chanMgr->addHitList(info);
        l->used = W(4) != 0;
        int nh = W(4);
        for (int j = 0; j < nh; j++)
        {
            auto h = std::make_shared<ChanHit>();
            h->init();
            if (W(5))
                h->host = hostW();
            h->rhost[0] = W(4) ? hostW() : h->host;
            h->rhost[1] = W(2) ? Host() : hostW();
            h->uphost = W(3) ? Host() : (W(2) ? hostW() : g_channels.empty() ? Host() : g_channels[0]->sourceHost.host);
            h->numListeners = W(5); h->numRelays = W(5); h->numHops = W(4); h->upTime = W(1000);
            h->firewalled = W(2); h->relay = W(2); h->direct = W(2); h->cin = W(2); h->stable = W(2);
            h->tracker = W(2); h->recv = W(2); h->dead = W(5) == 0;
            h->version = W(2) ? 1218 : W(2000);
            h->versionVP = W(30); h->versionExNumber = W(300);
            h->versionExPrefix[0] = W(2) ? 'Y' : (char) wrng(); h->versionExPrefix[1] = W(2) ? 'T' : (char) wrng();
            for (auto& b : h->sessionID.id) b = (uint8_t) wrng();
            h->time = 900 + W(200);
            h->next = l->hit;
            l->hit = h;
        }
    }

    int nyp = W(4);
    for (int k = 0; k < nyp; k++)
    {
        std::vector<std::string> f(19);
        for (auto& s : f) s = wordW();
        f[1] = g_ids[W(4)].str();
        ChannelEntry e(f, wordW());
        e.url = wordW(); e.trackContact = wordW();
        e.numDirects = W(3) ? W(100) : (int) wrng();
        servMgr->channelDirectory->m_channels.push_back(e);
    }

    int nlog = W(6);
    for (int k = 0; k < nlog; k++)
    {
        std::string s = wordW();
        if (logSafe(s))
            sys->logBuf->write(s.c_str(), (LogBuffer::TYPE) W(8));
    }

    unlink("channelFilters.json");
    if (W(2))
    {
        std::ofstream f("channelFilters.json", std::ios::binary);
        f << wordW();
    }
}

static void teardown()
{
    for (auto& c : g_channels)
    {
        c->thread.m_active = false;
        c->next = nullptr;
        c->sock = nullptr;
    }
    chanMgr->channel = nullptr;
    g_channels.clear();
    chanMgr->clearHitLists();
    for (auto s : g_servents)
    {
        s->thread.m_active = false;
        delete s;
    }
    g_servents.clear();
    servMgr->servents = g_origServents;
    servMgr->channelDirectory->m_channels.clear();
    sys->logBuf->clear();
}

static std::string snapshot()
{
    std::string s = "maxRelays=" + std::to_string(servMgr->maxRelays) + " maxDirect=" + std::to_string(servMgr->maxDirect) +
                    " maxBitrateOut=" + std::to_string(servMgr->maxBitrateOut) + " logLevel=" + std::to_string(servMgr->logLevel()) +
                    " maxRelaysPerChannel=" + std::to_string(chanMgr->maxRelaysPerChannel) + " root=" + show(servMgr->rootHost.c_str()) + "\n";
    for (auto c = chanMgr->channel; c; c = c->next)
        s += "ch " + c->info.id.str() + " bump=" + std::to_string(c->bump) + " active=" + std::to_string(c->thread.active()) +
             " ip=" + std::to_string(c->ipVersion) + " src=" + show(c->sourceURL.c_str()) + " " + infoStr(c->info) + "\n";
    for (auto sv = servMgr->servents; sv && sv != g_origServents; sv = sv->next)
        s += "sv " + std::to_string(sv->serventIndex) + "\n";
    int lines = sys->logBuf->toLines([](unsigned int, LogBuffer::TYPE, const char*) { return std::string(); }).size();
    s += "log=" + std::to_string(lines) + "\n";
    std::ifstream f("channelFilters.json", std::ios::binary);
    if (f)
        s += "file=" + show(std::string((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>())) + "\n";
    return s;
}

// ---------------------------------------------------------------- 要求

static std::string jstr(const std::string& s)
{
    std::string r = "\"";
    char b[8];
    for (unsigned char c : s)
    {
        if (c == '"' || c == '\\') { r += '\\'; r += (char)c; }
        else if (c < 0x20) { snprintf(b, sizeof b, "\\u%04x", c); r += b; }
        else r += (char)c;
    }
    return r + "\"";
}

static std::string strLit()
{
    static const char* lits[] = { "\"\"", "\"a\"", "\"日本語\"", "\"\\u00e9\\ud83d\\ude00\"", "\"a\\u0000b\"",
                                  "\"\\/\\b\\f\\n\\r\\t\"", "\"\\u001f\\u007f\"", "\"\xe3\x81\x82\"",
                                  "\"\\\"\\\\\"", "\"ipv6\"", "\"ipv4\"", "\"FLV\"", "\"MP3\"", "\"channelFilters\"",
                                  "\"servMgr\"", "\"chanMgr\"", "\"stats\"", "\"notificationBuffer\"", "\"sys\"", "\"ypList\"",
                                  "\"http://example.com/a.flv\"", "\"x\\u00ff\"" };
    // 構文の誤りになるもの (たまに)
    static const char* bad[] = { "\"\\ud800\"", "\"\xff\"", "\"x\x01y\"", "\"\\udc00\\ud800\"", "\"\\u12\"" };
    if (R(40) == 0)
        return bad[R(sizeof bad / sizeof bad[0])];
    if (R(4) == 0)
    {
        std::string s;
        int n = R(8);
        for (int i = 0; i < n; i++) s += (char)(0x20 + R(0x5f));
        return jstr(s);
    }
    return lits[R(sizeof lits / sizeof lits[0])];
}

static std::string numLit()
{
    static const char* nums[] = { "0", "-0", "1", "-1", "5", "6", "7", "2147483647", "2147483648", "-2147483648",
                                  "-2147483649", "4294967295", "4294967296", "9223372036854775807", "9223372036854775808",
                                  "-9223372036854775808", "-9223372036854775809", "18446744073709551615",
                                  "18446744073709551616", "1.5", "-0.5", "-0.0", "1e10", "1e19", "1e20", "1e300",
                                  "1e-400", "0.1", "1E+2", "123456789012345678901234567890", "2.5e-5", "0.0001",
                                  "1e15", "1e16", "100000000000000000000", "3.14159265358979323846" };
    char b[64];
    if (R(200) == 0)
        return R(2) ? "1e400" : "-1e400";   // 有限でない数 (C++ 版は例外が上に飛ぶ)
    switch (R(6))
    {
    case 0: snprintf(b, sizeof b, "%d", (int) rng()); return b;
    case 1:
    {
        uint64_t bits = ((uint64_t) rng() << 32) | rng();
        double d;
        memcpy(&d, &bits, 8);
        if (!std::isfinite(d)) d = 1.0;
        snprintf(b, sizeof b, "%.17g", d);
        return b;
    }
    case 2: snprintf(b, sizeof b, "%.*g", 1 + R(17), (double) (int) rng() / (1 + R(100000))); return b;
    default: return nums[R(sizeof nums / sizeof nums[0])];
    }
}

static std::string idLit()
{
    switch (R(8))
    {
    case 0: return "\"" + str::downcase(g_ids[R(4)].str()) + "\"";
    case 1: return "\"hoge\"";
    case 2: return "\"\"";
    case 3: return "\"" + g_ids[R(4)].str().substr(0, 20) + "\\u0000" + g_ids[0].str().substr(20) + "\"";
    case 4: { std::string s; for (int i = 0; i < 32; i++) s += "0123456789abcdefABCDEF +-"[R(25)]; return "\"" + s + "\""; }
    default: return "\"" + g_ids[R(4)].str() + "\"";
    }
}

static std::string value(int depth);

static std::string arr(int depth, int n)
{
    std::string s = "[";
    for (int i = 0; i < n; i++) s += (i ? "," : "") + value(depth + 1);
    return s + "]";
}

static std::string value(int depth)
{
    switch (R(depth > 3 ? 5 : 8))
    {
    case 0: return "null";
    case 1: return R(2) ? "true" : "false";
    case 2: return numLit();
    case 3: return strLit();
    case 4: return idLit();
    case 5: return arr(depth, R(4));
    default:
    {
        static const char* keys[] = { "a", "level", "name", "url", "maxRelays", "channelId", "", "\\u0000" };
        std::string s = "{";
        int n = R(4);
        for (int i = 0; i < n; i++) s += std::string(i ? "," : "") + "\"" + keys[R(8)] + "\":" + value(depth + 1);
        return s + "}";
    }
    }
}

// 各メソッドの引数らしいもの
static std::string maybe(const std::string& good, int depth = 1) { return R(6) ? good : value(depth); }

static std::string objOf(const std::vector<const char*>& keys, const std::vector<std::string>& vals)
{
    std::string s = "{";
    bool first = true;
    for (size_t i = 0; i < keys.size(); i++)
    {
        if (R(8) == 0) continue;
        s += std::string(first ? "" : ",") + "\"" + keys[i] + "\":" + vals[i];
        first = false;
    }
    if (R(10) == 0) s += std::string(first ? "" : ",") + "\"extra\":1";
    return s + "}";
}

static std::string params(const std::string& method)
{
    std::vector<const char*> names;
    std::vector<std::string> vals;
    if (method == "fetch")
    {
        names = { "url", "name", "desc", "genre", "contact", "bitrate", "type", "network" };
        vals = { maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(numLit()),
                 maybe(strLit()), R(2) ? "null" : maybe(R(2) ? "\"ipv6\"" : strLit()) };
    }
    else if (method == "getLog")
    {
        names = { "from", "maxLines" };
        vals = { R(3) ? "null" : maybe(numLit()), R(3) ? "null" : maybe(numLit()) };
    }
    else if (method == "setLogSettings")
    {
        names = { "settings" };
        vals = { maybe("{\"level\":" + maybe(numLit()) + "}") };
    }
    else if (method == "setSettings")
    {
        names = { "settings" };
        vals = { maybe(objOf({ "maxRelays", "maxRelaysPerChannel", "maxDirects", "maxUpstreamRate", "channelCleaner" },
                             { maybe(numLit()), maybe(numLit()), maybe(numLit()), maybe(numLit()), maybe(numLit()) })) };
    }
    else if (method == "getState")
    {
        names = { "objectNames" };
        std::string a = "[";
        int n = R(4);
        for (int i = 0; i < n; i++) a += std::string(i ? "," : "") + maybe(strLit());
        vals = { maybe(a + "]") };
    }
    else if (method == "setChannelInfo")
    {
        names = { "channelId", "info", "track" };
        if (R(2))
        {
            // 全部そろったもの
            auto o = [](std::vector<const char*> keys) {
                std::string s = "{";
                for (size_t i = 0; i < keys.size(); i++) s += std::string(i ? "," : "") + "\"" + keys[i] + "\":" + strLit();
                return s + "}";
            };
            vals = { "\"" + g_ids[R(4)].str() + "\"", o({ "name", "desc", "genre", "url", "comment" }),
                     o({ "url", "name", "creator", "album", "genre" }) };
        }
        else
        vals = { maybe(idLit()),
                 maybe(objOf({ "name", "desc", "genre", "url", "comment" },
                             { maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()) })),
                 maybe(objOf({ "url", "name", "creator", "album", "genre" },
                             { maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()), maybe(strLit()) })) };
    }
    else if (method == "stopChannelConnection")
    {
        names = { "channelId", "connectionId" };
        vals = { maybe(idLit()), maybe(std::to_string(R(5))) };
    }
    else if (method == "removeYellowPage")
    {
        names = { "yellowPageId" };
        vals = { maybe(R(2) ? "0" : numLit()) };
    }
    else if (method == "getServerStorageItem")
    {
        names = { "key" };
        vals = { maybe(R(2) ? "\"channelFilters\"" : strLit()) };
    }
    else if (method == "setServerStorageItem")
    {
        names = { "key", "value" };
        vals = { maybe(R(2) ? "\"channelFilters\"" : strLit()), maybe(strLit()) };
    }
    else if (method == "bumpChannel" || method == "getChannelConnections" || method == "getChannelInfo" ||
             method == "getChannelRelayTree" || method == "getChannelStatus" || method == "playChannel" ||
             method == "stopChannel")
    {
        names = { "channelId" };
        vals = { maybe(idLit()) };
    }

    switch (R(10))
    {
    case 0: return value(1);                    // 配列でもオブジェクトでもないかもしれない
    case 1: case 2: case 3:
    {
        // 名前付き (足りないもの、余計なものも)
        std::string s = "{";
        bool first = true;
        for (size_t i = 0; i < names.size(); i++)
        {
            if (R(10) == 0) continue;
            s += std::string(first ? "" : ",") + "\"" + names[i] + "\":" + vals[i];
            first = false;
        }
        if (R(8) == 0) s += std::string(first ? "" : ",") + "\"zzz\":1";
        return s + "}";
    }
    default:
    {
        std::string s = "[";
        size_t n = vals.size();
        if (R(10) == 0) n = R(n + 2);
        for (size_t i = 0; i < n; i++) s += std::string(i ? "," : "") + (i < vals.size() ? vals[i] : value(1));
        return s + "]";
    }
    }
}

static const char* g_methods[] = {
    "bumpChannel", "clearLog", "fetch", "getChannelConnections", "getChannelInfo", "getChannelRelayTree",
    "getChannelStatus", "getChannels", "getLog", "getLogSettings", "getNewVersions", "getNotificationMessages",
    "getPlugins", "getServerStorageItem", "getSettings", "getState", "getStatus", "getVersionInfo", "getYPChannels",
    "getYellowPageProtocols", "getYellowPages", "playChannel", "removeYellowPage", "setChannelInfo", "setLogSettings",
    "setServerStorageItem", "setSettings", "stopChannel", "stopChannelConnection",
    "getChannelsFound", "nonexistentMethod", "GetStatus", "",
};

static std::string mutate(std::string s)
{
    int n = 1 + R(3);
    for (int i = 0; i < n; i++)
    {
        size_t pos = R(s.size() + 1);
        switch (R(3))
        {
        case 0: if (pos < s.size()) s[pos] = (char) rng(); break;
        case 1: s.insert(s.begin() + pos, (char) (R(2) ? rng() : "{}[]\",:\\0e-."[R(12)])); break;
        default: if (pos < s.size()) s.erase(pos, 1); break;
        }
    }
    return s;
}

static std::string request()
{
    std::string method = g_methods[R(sizeof g_methods / sizeof g_methods[0])];
    std::vector<std::string> members;
    if (R(20)) members.push_back(R(20) ? "\"jsonrpc\":\"2.0\"" : "\"jsonrpc\":" + value(2));
    if (R(15)) members.push_back("\"id\":" + value(1));
    if (R(15)) members.push_back("\"method\":" + (R(20) ? jstr(method) : value(2)));
    if (R(8)) members.push_back("\"params\":" + params(method));
    if (R(20) == 0) members.push_back("\"id\":" + value(2));   // 同じキー
    for (size_t i = members.size(); i > 1; i--) std::swap(members[i - 1], members[R(i)]);
    std::string s = "{";
    for (size_t i = 0; i < members.size(); i++) s += (i ? (R(4) ? "," : " ,\n\t") : "") + members[i];
    s += "}";
    if (R(20) == 0) s = "\xef\xbb\xbf" + s;
    if (R(20) == 0) s += R(2) ? " \n" : " x";
    if (R(6) == 0) s = mutate(s);
    return s;
}

static std::string inspectText()
{
    static const char* specials[] = { "nan", "inf", "-nan", "(1234, 0)", "{\"a\":nan}", "", "{", "{\"a\":1,}",
                                      "[1 2]", "\"\\x\"", "tru", "{\"k\":\"v\"}\n{\"k\":2}", "\xef\xbb", "\xef\xbb\xbf{}" };
    switch (R(4))
    {
    case 0: return specials[R(sizeof specials / sizeof specials[0])];
    case 1: return mutate(value(0));
    default: return value(0);
    }
}

// ---------------------------------------------------------------- 実行

static std::string runSide(int side, unsigned seed, const std::string& req)
{
    build(seed);
    g_rec.clear();
    std::string res;
    try {
        if (side == 0)
            res = JrpcApi().call(req);
        else
            res = rustbridge::jrpcCall(req);
    } catch (std::exception& e) {
        res = std::string("EXC:") + e.what();
    }
    std::string out = res + "\n---\n" + g_rec + "---\n" + snapshot();
    teardown();
    return out;
}

// メソッドごとの応答の種類の数 (どこまで試せているかを見る)
static std::map<std::string, std::map<std::string, long>> g_outcomes;

static void count(const std::string& req, const std::string& res)
{
    std::string method = "(不明)";
    for (auto m : g_methods)
        if (*m && req.find(std::string("\"method\":\"") + m + "\"") != std::string::npos)
            method = m;
    std::string kind;
    size_t p;
    if (res.compare(0, 4, "EXC:") == 0)
        kind = "例外";
    else if (res.find("\"result\":") != std::string::npos && res.find("\"error\":") == std::string::npos)
        kind = "result";
    else if ((p = res.find("\"code\":")) != std::string::npos)
        kind = res.substr(p + 7, res.find(',', p) - p - 7);
    g_outcomes[method][kind]++;
}

static void oneCase()
{
    unsigned seed = rng();
    std::string req = request();
    g_inspect = inspectText();
    g_createFails = R(10) == 0;

    std::string a = runSide(0, seed, req);
    std::string b = runSide(1, seed, req);
    g_cases++;
    count(req, a.substr(0, a.find("\n---\n")));

    // 既知の違い: C++ 版は有限でない数の out_of_range を捕まえず、例外が上に飛ぶ。Rust 版は Parse error。
    auto startsWith = [](const std::string& s, const char* p) { return s.compare(0, strlen(p), p) == 0; };
    if (startsWith(a, "EXC:[json.exception.out_of_range.406] ") &&
        startsWith(b, "{\"error\":{\"code\":-32700,\"message\":\"Parse error\"},\"id\":null,\"jsonrpc\":\"2.0\"}"))
    {
        g_overflow++;
        return;
    }
    if (a != b && g_bad++ < 20)
        printf("  [違い] %s\n    C++ =%s\n    Rust=%s\n", show(req.substr(0, 1500)).c_str(),
               show(a.substr(0, 3000)).c_str(), show(b.substr(0, 3000)).c_str());
}

// ---------------------------------------------------------------- メソッドを直接呼ぶ (JrpcApi::getChannels など)

using json = nlohmann::json;

// 数の種類と文字列のバイト列をそのまま見せる (不正な UTF-8 でも dump のように例外にならない)
static std::string jshow(const json& j)
{
    char b[40];
    switch (j.type())
    {
    case json::value_t::null: return "null";
    case json::value_t::boolean: return j.get<bool>() ? "true" : "false";
    case json::value_t::number_integer: return "i" + std::to_string(j.get<int64_t>());
    case json::value_t::number_unsigned: return "u" + std::to_string(j.get<uint64_t>());
    case json::value_t::number_float: snprintf(b, sizeof b, "f%.17g", j.get<double>()); return b;
    case json::value_t::string: return "\"" + show(j.get<std::string>()) + "\"";
    case json::value_t::array: { std::string s = "["; for (auto& e : j) s += jshow(e) + ","; return s + "]"; }
    case json::value_t::object:
    {
        std::string s = "{";
        for (auto it = j.begin(); it != j.end(); ++it) s += show(it.key()) + ":" + jshow(it.value()) + ",";
        return s + "}";
    }
    default: return "?";
    }
}

// jrpc.cpp の WITH_RUST_CORE の JrpcApi::invoke と同じ
static std::string rsInvoke(const std::string& method, const json::array_t& args)
{
    rustbridge::JrpcHost host;
    rustbridge::JsonBuilder builder;
    std::string a = json(args).dump();
    int32_t code = 0;
    rustbridge::RustBuf what;
    int r = pcrs_jrpc_invoke(reinterpret_cast<const uint8_t*>(method.data()), method.size(),
                             reinterpret_cast<const uint8_t*>(a.data()), a.size(), host.get(), builder.get(), &code, what.out());
    switch (r)
    {
    case 0: return jshow(builder.result);
    case 1: return "method_not_found:" + what.str();
    case 2: return "invalid_params:" + what.str();
    case 3: return "application_error:" + std::to_string(code) + ":" + what.str();
    default: return "exception:" + what.str();
    }
}

typedef json (JrpcApi::*Method)(json::array_t);
static const std::vector<std::pair<std::string, std::pair<Method, size_t>>> g_direct = {
    { "getChannels", { &JrpcApi::getChannels, 0 } }, { "getChannelsFound", { &JrpcApi::getChannelsFound, 0 } },
    { "getVersionInfo", { &JrpcApi::getVersionInfo, 0 } }, { "bumpChannel", { &JrpcApi::bumpChannel, 1 } },
    { "clearLog", { &JrpcApi::clearLog, 0 } }, { "fetch", { &JrpcApi::fetch, 8 } },
    { "getChannelConnections", { &JrpcApi::getChannelConnections, 1 } }, { "getChannelInfo", { &JrpcApi::getChannelInfo, 1 } },
    { "getChannelRelayTree", { &JrpcApi::getChannelRelayTree, 1 } }, { "getChannelStatus", { &JrpcApi::getChannelStatus, 1 } },
    { "getLog", { &JrpcApi::getLog, 2 } }, { "getLogSettings", { &JrpcApi::getLogSettings, 0 } },
    { "getNewVersions", { &JrpcApi::getNewVersions, 0 } }, { "getNotificationMessages", { &JrpcApi::getNotificationMessages, 0 } },
    { "getPlugins", { &JrpcApi::getPlugins, 0 } }, { "getServerStorageItem", { &JrpcApi::getServerStorageItem, 1 } },
    { "getSettings", { &JrpcApi::getSettings, 0 } }, { "getState", { &JrpcApi::getState, 1 } },
    { "getStatus", { &JrpcApi::getStatus, 0 } }, { "getYPChannels", { &JrpcApi::getYPChannels, 0 } },
    { "getYellowPageProtocols", { &JrpcApi::getYellowPageProtocols, 0 } }, { "getYellowPages", { &JrpcApi::getYellowPages, 0 } },
    { "playChannel", { &JrpcApi::playChannel, 1 } }, { "removeYellowPage", { &JrpcApi::removeYellowPage, 1 } },
    { "setChannelInfo", { &JrpcApi::setChannelInfo, 3 } }, { "setLogSettings", { &JrpcApi::setLogSettings, 1 } },
    { "setServerStorageItem", { &JrpcApi::setServerStorageItem, 2 } }, { "setSettings", { &JrpcApi::setSettings, 1 } },
    { "stopChannel", { &JrpcApi::stopChannel, 1 } }, { "stopChannelConnection", { &JrpcApi::stopChannelConnection, 2 } },
};

static std::string cxxInvoke(Method m, const json::array_t& args)
{
    JrpcApi api;
    try {
        return jshow((api.*m)(args));
    } catch (JrpcApi::method_not_found& e) {
        return std::string("method_not_found:") + e.what();
    } catch (JrpcApi::invalid_params& e) {
        return std::string("invalid_params:") + e.what();
    } catch (JrpcApi::application_error& e) {
        return "application_error:" + std::to_string(e.m_errno) + ":" + e.what();
    } catch (std::exception& e) {
        return std::string("exception:") + e.what();
    }
}

static long g_directCases = 0;

static void directCase()
{
    // public.cpp が使う getChannels と getChannelsFound を多めに
    auto& d = g_direct[R(3) ? R(2) : R(g_direct.size())];
    json::array_t args;
    for (int tries = 0; tries < 20; tries++)
    {
        try {
            json p = json::parse(params(d.first));
            if (p.is_array() && p.size() == d.second.second)
            {
                args = p.get<json::array_t>();
                break;
            }
        } catch (std::exception&) {}
    }
    if (args.size() != d.second.second)
        return;

    unsigned seed = rng();
    g_inspect = inspectText();
    g_createFails = R(10) == 0;
    std::string out[2];
    for (int side = 0; side < 2; side++)
    {
        build(seed);
        g_rec.clear();
        std::string res = side == 0 ? cxxInvoke(d.second.first, args) : rsInvoke(d.first, args);
        out[side] = res + "\n---\n" + g_rec + "---\n" + snapshot();
        teardown();
    }
    g_cases++;
    g_directCases++;
    if (out[0] != out[1] && g_bad++ < 20)
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", d.first.c_str(), show(json(args).dump()).c_str(),
               show(out[0].substr(0, 3000)).c_str(), show(out[1].substr(0, 3000)).c_str());
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 100000;

    // 状態のファイル (channelFilters.json) は作業用のディレクトリに置く
    char dir[] = "/tmp/diff_jrpc.XXXXXX";
    if (!mkdtemp(dir) || chdir(dir) != 0)
        return 2;

    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    setIds();
    g_origServents = servMgr->servents;

    for (long i = 0; i < n; i++)
    {
        oneCase();
        if (i % 4 == 0)
            directCase();
    }

    unlink("channelFilters.json");
    rmdir(dir);
    if (getenv("DJ_STATS"))
    {
        for (auto& m : g_outcomes)
        {
            printf("%-24s", m.first.c_str());
            for (auto& k : m.second)
                printf(" %s:%ld", k.first.c_str(), k.second);
            printf("\n");
        }
    }
    printf("メソッドを直接呼んだもの %ld\n", g_directCases);
    printf("有限でない数で C++ 版が例外を投げたもの (既知の違い) %ld\n", g_overflow);
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
