// PCP の受け取ったパケットの処理 (core/common/pcp.cpp の PCPStream::procAtom 以下と、chaninfo.cpp の
// readInfoAtoms / readTrackAtoms) の、C++ 版と Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の PCPStream::procAtom。Rust 版は、本番と
// 同じ橋渡しのコード (core/common/rustpcp.h の rustbridge::PcpHost) を通して呼ぶ。どちらにも同じ
// パケットのバッファ (16KB 全体) と同じ状態の chanMgr、servMgr、PCPStream、BroadcastState を与え、
// 次のものを比べる。
//  - 外への働きかけ (リンク時の --wrap で横取りして記録する): ログ、中継 (broadcastPacketUp、
//    broadcastPacket)、通知、GIV の接続 (allocServent)、トラッカーへの更新、ヒットの追加と削除、
//    Channel::updateInfo
//  - 返り値か例外、処理のあとのバッファ (helo への返事を書き込む)、BroadcastState、
//    nextRootPacket、routeList、servMgr (rootMsg、downloadURL)、chanMgr (更新間隔、ヒットリスト
//    とその中身)、チャンネル (streamPos、streamIndex、headPack、rawData、info)
//
// 説明のつく違い:
//  - procAtom の入れ子 (atom の子、bcst の中の atom) が 64 段を超えると、Rust 版は例外にする
//    (C++ 版は bcst の入れ子ごとに 16KB のスタックを使い、数百段で落ちていた)
//  - NUL で終わらない文字列の atom: C++ 版は String のまだ書いていない部分 (初期化されていないメモリ)
//    を文字列の続きとして読んでいた。Rust 版はそこを 0 とみなす。比べられるように、C++ 版も
//    String::clear を横取りしてバッファ全体を 0 にする
//  - C++ 版は、バッファの終わりに着いたあとも子の数だけ (最大約 21 億回) ループし、ID 0 の atom の
//    「PCP skip」のログを出し続ける。Rust 版はそこでループを終える。比べるときは、続けて同じ
//    「PCP skip: , 0, 0」のログを 1 つにまとめる。C++ 版が長く空回りしたもの (ログが多すぎるか、
//    時間がかかりすぎたもの) は比べずに数える
//
// C++ 版が初期化していないメモリを読む箇所 (String や ChanPacket のまだ書いていない部分) は、
// Rust 版では 0 として扱う。比べられるように C++ 版も 0 にする: スタックは
// -ftrivial-auto-var-init=zero (Makefile)、ヒープはこのファイルの operator new で 0 に埋める。
//
// 使い方: ./diff_pcp [件数 (既定 100000)]
#include <csetjmp>
#include <csignal>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <pthread.h>
#include <memory>
#include <new>
#include <random>
#include <string>
#include <sys/time.h>
#include <vector>

#include "atom.h"
#include "chanmgr.h"
#include "channel.h"
#include "pcp.h"
#include "rustpcp.h"
#include "servmgr.h"
#include "../../../tests/mockpeercast.h"

// ---------------------------------------------------------------- ヒープを 0 で埋める

void* operator new(std::size_t n)
{
    void* p = std::calloc(1, n ? n : 1);
    if (!p)
        throw std::bad_alloc();
    return p;
}
void* operator new[](std::size_t n) { return operator new(n); }
void operator delete(void* p) noexcept { std::free(p); }
void operator delete[](void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }
void operator delete[](void* p, std::size_t) noexcept { std::free(p); }

// ---------------------------------------------------------------- 記録

static std::vector<std::string> g_events;
static long g_logCount = 0;
struct HarnessAbort {};
static const long LOG_LIMIT = 100000;

static std::string esc(const char* p, size_t n)
{
    std::string s;
    for (size_t i = 0; i < n; i++)
    {
        unsigned char c = p[i];
        if (c >= 0x20 && c < 0x7f && c != '\\')
            s += c;
        else
        {
            char b[8];
            snprintf(b, sizeof b, "\\x%02x", c);
            s += b;
        }
    }
    return s;
}
static std::string esc(const char* p) { return esc(p, strlen(p)); }

static uint64_t fnv(const void* p, size_t n)
{
    uint64_t h = 1469598103934665603ULL;
    const unsigned char* s = static_cast<const unsigned char*>(p);
    for (size_t i = 0; i < n; i++) { h ^= s[i]; h *= 1099511628211ULL; }
    return h;
}

static void logv(const char* level, const char* fmt, va_list ap)
{
    char buf[8192];
    vsnprintf(buf, sizeof buf, fmt, ap);
    g_events.push_back(std::string(level) + esc(buf));
    if (++g_logCount > LOG_LIMIT)
        throw HarnessAbort();
}
extern "C" void __wrap__Z9LOG_TRACEPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("T:", f, ap); va_end(ap); }
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("D:", f, ap); va_end(ap); }
extern "C" void __wrap__Z8LOG_INFOPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("I:", f, ap); va_end(ap); }
extern "C" void __wrap__Z8LOG_WARNPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("W:", f, ap); va_end(ap); }
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("E:", f, ap); va_end(ap); }

static std::string pktStr(const ChanPacket& p)
{
    char b[128];
    snprintf(b, sizeof b, "type=%d len=%u pos=%u cont=%d hash=%016llx", (int) p.type, p.len, p.pos, (int) p.cont,
             (unsigned long long) fnv(p.data, p.len));
    return b;
}

extern "C" int __wrap__ZN7ChanMgr17broadcastPacketUpER10ChanPacketRK5GnuIDS4_S4_(ChanMgr*, ChanPacket& p, const GnuID& c, const GnuID& s, const GnuID& d)
{
    g_events.push_back("broadcastPacketUp " + pktStr(p) + " " + c.str() + " " + s.str() + " " + d.str());
    return 0;
}
extern "C" int __wrap__ZN7ServMgr15broadcastPacketER10ChanPacketRK5GnuIDS4_S4_N7Servent4TYPEE(ServMgr*, ChanPacket& p, const GnuID& c, const GnuID& s, const GnuID& d, int type)
{
    g_events.push_back("broadcastPacket " + std::to_string(type) + " " + pktStr(p) + " " + c.str() + " " + s.str() + " " + d.str());
    return 0;
}
extern "C" Servent* __wrap__ZN7ServMgr12allocServentEv(ServMgr*)
{
    g_events.push_back("allocServent");
    return nullptr;
}
extern "C" void __wrap__ZN8peercast13notifyMessageEN7ServMgr11NOTIFY_TYPEERKNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEEE(int type, const std::string& msg)
{
    g_events.push_back("notify " + std::to_string(type) + " " + esc(msg.data(), msg.size()));
}
extern "C" void __wrap__ZN7ChanMgr22broadcastTrackerUpdateERK5GnuIDb(ChanMgr*, const GnuID& id, bool force)
{
    g_events.push_back("broadcastTrackerUpdate " + id.str() + " " + std::to_string(force));
}

static std::string hitStr(const ChanHit& h)
{
    char b[512];
    snprintf(b, sizeof b, "host=%s r0=%s r1=%s numl=%u numr=%u hops=%u up=%u old=%u new=%u ver=%u vp=%u vexp=%02x%02x vexn=%u "
             "recv=%d relay=%d direct=%d cin=%d tracker=%d fw=%d sid=%s cid=%s uphost=%s uphops=%u",
             h.host.str().c_str(), h.rhost[0].str().c_str(), h.rhost[1].str().c_str(), h.numListeners, h.numRelays, h.numHops,
             h.upTime, h.oldestPos, h.newestPos, h.version, h.versionVP, (unsigned char) h.versionExPrefix[0],
             (unsigned char) h.versionExPrefix[1], h.versionExNumber, h.recv, h.relay, h.direct, h.cin, h.tracker, h.firewalled,
             h.sessionID.str().c_str(), h.chanID.str().c_str(), h.uphost.str().c_str(), h.uphostHops);
    return b;
}

extern "C" void __real__ZN7ChanMgr6delHitER7ChanHit(ChanMgr*, ChanHit&);
extern "C" void __wrap__ZN7ChanMgr6delHitER7ChanHit(ChanMgr* m, ChanHit& h)
{
    g_events.push_back("delHit " + hitStr(h));
    __real__ZN7ChanMgr6delHitER7ChanHit(m, h);
}

static std::string infoStr(const ChanInfo& i)
{
    return "name=" + esc(i.name.c_str()) + " genre=" + esc(i.genre.c_str()) + " url=" + esc(i.url.c_str()) +
           " desc=" + esc(i.desc.c_str()) + " cmnt=" + esc(i.comment.c_str()) + " type=" + esc(i.contentType.c_str()) +
           " mime=" + esc(i.MIMEType.c_str()) + " ext=" + esc(i.streamExt.c_str()) + " title=" + esc(i.track.title.c_str()) +
           " artist=" + esc(i.track.artist.c_str()) + " contact=" + esc(i.track.contact.c_str()) +
           " album=" + esc(i.track.album.c_str()) + " bitrate=" + std::to_string(i.bitrate) + " id=" + i.id.str() +
           " bcid=" + i.bcID.str();
}

extern "C" bool __real__ZN7Channel10updateInfoERK8ChanInfo(Channel*, const ChanInfo&);
extern "C" bool __wrap__ZN7Channel10updateInfoERK8ChanInfo(Channel* ch, const ChanInfo& info)
{
    g_events.push_back("updateInfo " + infoStr(info));
    return __real__ZN7Channel10updateInfoERK8ChanInfo(ch, info);
}

// String() と String::clear() は data[0] だけを 0 にし、残り (NUL の後ろ) は初期化しない。C++ 版は
// NUL で終わらない文字列の atom を読むとその続きを文字列として使うので、Rust 版と同じく 0 にそろえる
// (Rust 版の newInfo は 0 で埋めた記憶領域の上に作る。core/common/rustpcp.h の resetNewInfo)。
extern "C" void __wrap__ZN6String5clearEv(::String* s)
{
    memset(s->data, 0, sizeof(s->data));
    s->type = ::String::T_UNKNOWN;
}

class TestChanMgr : public ChanMgr
{
public:
    std::shared_ptr<ChanHit> addHit(ChanHit& h) override
    {
        g_events.push_back("addHit " + hitStr(h));
        return ChanMgr::addHit(h);
    }
};

// ---------------------------------------------------------------- 状態

static GnuID gid(unsigned char b)
{
    GnuID id;
    memset(id.id, b, 16);
    return id;
}
static const unsigned char CH_A = 0xa1, HL_B = 0xb2, SID = 0x5d, REMOTE = 0x7e;

struct Case
{
    std::vector<char> buf;      // 16KB
    bool hasCh, chBroadcasting, hasHl, isRoot;
    std::string rootMsg;
    unsigned char bcsChan;
    int numHops, group;
    bool forMe;
    unsigned streamPos, nextRoot, chStreamPos;
    int depth;                  // 生成した入れ子の深さ
};

static std::string dumpState(PCPStream& pcp, BroadcastState& bcs, const std::vector<char>& buf)
{
    std::string s;
    s += "bcs chan=" + bcs.chanID.str() + " bc=" + bcs.bcID.str() + " hops=" + std::to_string(bcs.numHops) +
         " forMe=" + std::to_string(bcs.forMe) + " pos=" + std::to_string(bcs.streamPos) + " group=" + std::to_string(bcs.group) + "\n";
    s += "nextRoot=" + std::to_string(pcp.nextRootPacket) + "\n";
    char b[64];
    snprintf(b, sizeof b, "buf=%016llx\n", (unsigned long long) fnv(buf.data(), buf.size()));
    s += b;
    s += "route";
    for (int i = 0; i < pcp.routeList.maxID; i++)
        if (pcp.routeList.ids[i].isSet())
            s += " " + pcp.routeList.ids[i].str();
    s += "\n";
    s += "rootMsg=" + esc(servMgr->rootMsg.c_str()) + " download=" + esc(servMgr->downloadURL) + "\n";
    s += "updint=" + std::to_string(chanMgr->hostUpdateInterval) + "\n";
    for (auto chl = chanMgr->hitlist; chl; chl = chl->next)
    {
        s += "hitlist " + infoStr(chl->info) + "\n";
        chl->forEachHit([&](ChanHit* h) { s += "  hit " + hitStr(*h) + "\n"; });
    }
    for (auto ch = chanMgr->channel; ch; ch = ch->next)
    {
        s += "channel streamPos=" + std::to_string(ch->streamPos) + " index=" + std::to_string(ch->streamIndex) +
             " head " + pktStr(ch->headPack) + "\n  info " + infoStr(ch->info) + "\n";
        auto& rd = ch->rawData;
        s += "  raw write=" + std::to_string(rd.writePos) + " read=" + std::to_string(rd.readPos) +
             " first=" + std::to_string(rd.firstPos) + " last=" + std::to_string(rd.lastPos) + "\n";
        for (unsigned i = rd.firstPos; rd.writePos && i <= rd.lastPos; i++)
            s += "  pkt " + pktStr(rd.packets[i % ChanPacketBuffer::MAX_PACKETS]) + "\n";
    }
    return s;
}

// 続けて同じ「PCP skip: , 0, 0」のログを 1 つにまとめる
static std::string events()
{
    std::string s;
    std::string prev;
    for (auto& e : g_events)
    {
        if (e == prev && e == "D:PCP skip: , 0, 0")
            continue;
        s += e + "\n";
        prev = e;
    }
    return s;
}

static int cxxProc(PCPStream& pcp, char* buf, BroadcastState& bcs)
{
    MemoryStream mem(buf, ChanPacket::MAX_DATALEN);
    AtomStream patom(mem);
    int numc, numd;
    ID4 id = patom.read(numc, numd);
    return pcp.procAtom(patom, id, numc, numd, bcs);
}

static sigjmp_buf g_jmp;
static volatile sig_atomic_t g_armed = 0;
static void onAlarm(int)
{
    if (g_armed)
        siglongjmp(g_jmp, 1);
}
static void setTimer(int ms)
{
    struct itimerval t = {};
    t.it_value.tv_sec = ms / 1000;
    t.it_value.tv_usec = (ms % 1000) * 1000;
    setitimer(ITIMER_REAL, &t, nullptr);
}

// 空回りで比べなかったとき "" (abort) か "TIMEOUT"
static std::string run(bool rust, const Case& c)
{
    g_events.clear();
    g_logCount = 0;

    chanMgr = new TestChanMgr();
    chanMgr->hostUpdateInterval = 120;
    if (c.hasCh)
    {
        auto ch = std::make_shared<Channel>();
        ch->info.id = gid(CH_A);
        ch->info.name.set("chA");
        ch->info.url.set("http://old.example/");
        ch->streamPos = c.chStreamPos;
        if (c.chBroadcasting)
            ch->status = Channel::S_BROADCASTING;
        chanMgr->channel = ch;
    }
    if (c.hasHl)
    {
        ChanInfo i;
        i.id = gid(HL_B);
        i.name.set("hlB");
        i.desc.set("old description that is long");
        chanMgr->addHitList(i);
    }
    servMgr->isRoot = c.isRoot;
    servMgr->rootMsg.set(c.rootMsg.c_str());
    strcpy(servMgr->downloadURL, "none");
    servMgr->sessionID = gid(SID);
    servMgr->chanLog.clear();

    PCPStream* pcp = new PCPStream(gid(REMOTE));
    pcp->nextRootPacket = c.nextRoot;
    BroadcastState bcs;
    bcs.chanID = c.bcsChan ? gid(c.bcsChan) : GnuID();
    bcs.numHops = c.numHops;
    bcs.forMe = c.forMe;
    bcs.streamPos = c.streamPos;
    bcs.group = c.group;
    std::vector<char> buf = c.buf;

    std::string res;
    if (!rust)
    {
        if (sigsetjmp(g_jmp, 1))
        {
            // 空回りを打ち切った。途中の状態は使わない (解放もしない)
            g_armed = 0;
            chanMgr = nullptr;
            return "TIMEOUT";
        }
        g_armed = 1;
        setTimer(200);
    }
    try
    {
        int r = rust ? rustbridge::PcpHost(*pcp).procPacket(buf.data(), buf.size(), bcs) : cxxProc(*pcp, buf.data(), bcs);
        res = "ret " + std::to_string(r) + "\n";
    }catch (HarnessAbort&)
    {
        g_armed = 0;
        setTimer(0);
        return "";
    }catch (StreamException& e)
    {
        res = "StreamException " + esc(e.msg) + "\n";
    }catch (GeneralException& e)
    {
        res = "GeneralException " + esc(e.msg) + "\n";
    }
    if (!rust)
    {
        g_armed = 0;
        setTimer(0);
    }
    res += events() + dumpState(*pcp, bcs, buf);
    delete pcp;
    delete chanMgr;
    chanMgr = nullptr;
    return res;
}

// ---------------------------------------------------------------- パケットを作る

static std::mt19937 rng(20260923);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }

static std::string id4(const char* id)
{
    std::string s(4, '\0');
    for (int i = 0; i < 4 && id[i]; i++)
        s[i] = id[i];
    return s;
}
static std::string le32(uint32_t v)
{
    std::string s(4, '\0');
    for (int i = 0; i < 4; i++)
        s[i] = (char) (v >> (8 * i));
    return s;
}
static std::string leaf(const char* id, const std::string& data)
{
    return id4(id) + le32(data.size()) + data;
}
static std::string parent(const char* id, const std::vector<std::string>& kids)
{
    uint32_t n = kids.size();
    int r = rnd(40);
    if (r == 0)
        n += 1;                 // 子の数を実際より多く
    else if (r == 1 && n)
        n -= 1;
    else if (r == 2)
        n = 0x7fffffff;         // 最大
    std::string s = id4(id) + le32(n | 0x80000000u);
    for (auto& k : kids)
        s += k;
    return s;
}

static std::string bytes(int n)
{
    std::string s(n, '\0');
    for (auto& c : s)
        c = (char) rnd(256);
    return s;
}

static std::string strv()
{
    switch (rnd(14))
    {
    case 0: return "";
    case 1: return std::string("abc", 3);
    case 2: return std::string("abc\0", 4);
    case 3: return std::string(255, 'a') + std::string(1, '\0');
    case 4: return std::string(256, 'b');
    case 5: return std::string(257, 'c');
    case 6: return std::string("http://example.com/x\0", 21);
    case 7: return std::string("https://e.jp/?q=1\0", 18);
    case 8: return std::string("javascript:alert(1)\0", 20);
    case 9: return std::string("\xe6\x97\xa5\xe6\x9c\xac\0", 7);
    case 10: return std::string("x\0yz\0", 5);
    case 11: return std::string(40, 'Z');
    default: return bytes(rnd(24));
    }
}
static std::string intv()
{
    static const uint32_t vals[] = { 0, 1, 7, 100, 1218, 1219, 0xffffffffu, 0x7fffffffu, 0x80000000u, 60, 16384 };
    std::string v = rnd(4) ? le32(vals[rnd(11)]) : bytes(4);
    if (rnd(25) == 0)
        v = bytes(rnd(2) ? 3 : 5);  // 長さが違う
    return v;
}
static std::string shortv()
{
    std::string v = bytes(2);
    if (rnd(25) == 0)
        v = bytes(rnd(2) ? 1 : 4);
    return v;
}
static std::string charv()
{
    static const unsigned char vals[] = { 0, 1, 2, 3, 4, 7, 0x10, 0x12, 0x7f, 0x80, 0xff, 0xfe };
    std::string v(1, (char) vals[rnd(12)]);
    if (rnd(25) == 0)
        v = bytes(rnd(2) ? 0 : 2);
    return v;
}
static std::string idv()
{
    std::string s;
    switch (rnd(6))
    {
    case 0: s = std::string(16, (char) CH_A); break;
    case 1: s = std::string(16, (char) HL_B); break;
    case 2: s = std::string(16, (char) SID); break;
    case 3: s = std::string(16, '\0'); break;
    default: s = bytes(16);
    }
    if (rnd(30) == 0)
        s = bytes(rnd(2) ? 15 : 17);
    return s;
}
static std::string ipv()
{
    switch (rnd(8))
    {
    case 0: return bytes(16);
    case 1: return std::string("\0\0\0\0\0\0\0\0\0\0\xff\xff\x01\x02\x03\x04", 16);
    case 2: return bytes(rnd(2) ? 5 : 8);
    default: return le32(rnd(2) ? 0x0100007f : (uint32_t) rng());
    }
}

static int g_depth;
static std::string genAtom(int depth);

static std::string genInfo()
{
    static const char* ids[] = { "name", "gnre", "url", "desc", "cmnt", "type", "styp", "sext" };
    std::vector<std::string> k;
    int n = rnd(6);
    for (int i = 0; i < n; i++)
    {
        int r = rnd(11);
        if (r < 8)
            k.push_back(leaf(ids[r], strv()));
        else if (r == 8)
            k.push_back(leaf("bitr", intv()));
        else if (r == 9)
            k.push_back(parent("zzzz", { leaf("a", "x") }));
        else
            k.push_back(leaf("zz", intv()));
    }
    return parent("info", k);
}
static std::string genTrack()
{
    static const char* ids[] = { "titl", "crea", "url", "albm" };
    std::vector<std::string> k;
    int n = rnd(5);
    for (int i = 0; i < n; i++)
        k.push_back(rnd(8) ? leaf(ids[rnd(4)], strv()) : leaf("xx", strv()));
    return parent("trck", k);
}
static std::string genPkt()
{
    std::vector<std::string> k;
    int n = rnd(6);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(7))
        {
        case 0: k.push_back(leaf("type", rnd(3) ? id4(rnd(2) ? "head" : "data") : (rnd(2) ? id4("meta") : intv()))); break;
        case 1: k.push_back(leaf("pos", rnd(3) ? le32(rnd(3) ? 0 : (uint32_t) rnd(100000)) : intv())); break;
        case 2: k.push_back(leaf("cont", charv())); break;
        case 3: k.push_back(leaf("data", bytes(rnd(60)))); break;
        case 4: k.push_back(leaf("data", bytes(rnd(3000)))); break;
        case 5: k.push_back(leaf("qq", intv())); break;
        default: k.push_back(parent("qq", { leaf("a", "b") }));
        }
    }
    return parent("pkt", k);
}
static std::string genChan()
{
    std::vector<std::string> k;
    int n = rnd(6);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(12))
        {
        case 0: case 1: k.push_back(leaf("id", idv())); break;
        case 2: k.push_back(leaf("bcid", idv())); break;
        case 3: k.push_back(leaf("key", idv())); break;
        case 4: case 5: k.push_back(genInfo()); break;
        case 6: k.push_back(genTrack()); break;
        case 7: case 8: case 9: k.push_back(genPkt()); break;
        case 10: k.push_back(leaf("zzz", intv())); break;
        default: k.push_back(parent("info", {}));
        }
    }
    return parent("chan", k);
}
static std::string genHost()
{
    static const char* ints[] = { "numl", "numr", "uptm", "oldp", "newp", "ver", "vevp", "uppt", "uphp" };
    std::vector<std::string> k;
    int n = rnd(10);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(10))
        {
        case 0: case 1: k.push_back(leaf("ip", ipv())); break;
        case 2: case 3: k.push_back(leaf("port", shortv())); break;
        case 4: k.push_back(leaf(ints[rnd(9)], intv())); break;
        case 5: k.push_back(leaf("flg1", charv())); break;
        case 6: k.push_back(leaf(rnd(2) ? "id" : "cid", idv())); break;
        case 7: k.push_back(leaf(rnd(2) ? "vexp" : "vexn", rnd(2) ? std::string("YT") : shortv())); break;
        case 8: k.push_back(leaf("upip", ipv())); break;
        default: k.push_back(rnd(2) ? leaf("trkr", intv()) : parent("zz", { leaf("a", "") }));
        }
    }
    return parent("host", k);
}
static std::string genRoot()
{
    std::vector<std::string> k;
    int n = rnd(7);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(8))
        {
        case 0: k.push_back(leaf("uint", intv())); break;
        case 1: k.push_back(leaf("url", strv())); break;
        case 2: k.push_back(leaf("chkv", intv())); break;
        case 3: k.push_back(leaf("next", intv())); break;
        case 4: k.push_back(rnd(2) ? parent("upd", { leaf("x", "yy") }) : leaf("upd", bytes(rnd(5)))); break;
        case 5: k.push_back(leaf(rnd(2) ? "mesg" : "asci", strv())); break;
        default: k.push_back(leaf("abcd", intv()));
        }
    }
    return parent("root", k);
}
static std::string genBcst(int depth)
{
    std::vector<std::string> k;
    int n = rnd(9);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(13))
        {
        case 0: k.push_back(leaf("ttl", charv())); break;
        case 1: k.push_back(leaf("hops", charv())); break;
        case 2: k.push_back(leaf("from", idv())); break;
        case 3: k.push_back(leaf("grp", charv())); break;
        case 4: k.push_back(leaf("dest", idv())); break;
        case 5: k.push_back(leaf("cid", idv())); break;
        case 6: k.push_back(leaf(rnd(2) ? "vers" : "vrvp", intv())); break;
        case 7: k.push_back(leaf("vexp", rnd(2) ? std::string("YT") : bytes(rnd(4)))); break;
        case 8: k.push_back(leaf("vexn", shortv())); break;
        default: k.push_back(genAtom(depth + 1));
        }
    }
    return parent("bcst", k);
}
static std::string genPush()
{
    std::vector<std::string> k;
    int n = rnd(5);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(4))
        {
        case 0: k.push_back(leaf("ip", ipv())); break;
        case 1: k.push_back(leaf("port", shortv())); break;
        case 2: k.push_back(leaf("cid", idv())); break;
        default: k.push_back(leaf("wxyz", intv()));
        }
    }
    return parent("push", k);
}

static std::string genAtom(int depth)
{
    if (depth > g_depth)
        g_depth = depth;
    int r = rnd(depth > 4 ? 10 : 16);
    switch (r)
    {
    case 0: case 1: return genChan();
    case 2: return genHost();
    case 3: return genRoot();
    case 4: return leaf(rnd(2) ? "mesg" : "asci", strv());
    case 5: return rnd(3) ? leaf("helo", bytes(rnd(12))) : parent("helo", { leaf("agnt", "x\0"), leaf("port", shortv()) });
    case 6: return genPush();
    case 7: return leaf("ok", intv());
    case 8: return leaf("quit", intv());
    case 9: return leaf(rnd(2) ? "zzzz" : "", intv());
    case 10: case 11: case 12: return genBcst(depth);
    default:
    {
        std::vector<std::string> k;
        int n = 1 + rnd(4);
        for (int i = 0; i < n; i++)
            k.push_back(genAtom(depth + 1));
        return parent("atom", k);
    }
    }
}

static Case genCase()
{
    Case c;
    g_depth = 0;
    std::string p;
    int kind = rnd(100);
    if (kind == 0)
    {
        // 深い入れ子
        int d = 55 + rnd(20);
        p = leaf("ok", le32(0));
        for (int i = 0; i < d; i++)
            p = (rnd(8) ? id4("atom") + le32(1 | 0x80000000u) : id4("bcst") + le32(1 | 0x80000000u)) + p;
        g_depth = d;
    }
    else
        p = genAtom(0);

    c.buf.assign(ChanPacket::MAX_DATALEN, '\0');
    int n = std::min<size_t>(p.size(), c.buf.size());
    memcpy(c.buf.data(), p.data(), n);
    // パケットの後ろ (C++ 版では前のパケットの残り)
    int fill = rnd(10);
    if (fill < 2)
        for (size_t i = n; i < c.buf.size(); i++)
            c.buf[i] = (char) rnd(256);
    else if (fill < 4)
        for (size_t i = n; i < c.buf.size() && n; i++)
            c.buf[i] = p[(i - n) % p.size()];
    // 変異
    if (rnd(3) == 0)
    {
        int m = 1 + rnd(4);
        for (int i = 0; i < m && n; i++)
            c.buf[rnd(n)] = (char) rnd(256);
    }
    c.depth = g_depth;

    c.hasCh = rnd(2);
    c.chBroadcasting = rnd(4) == 0;
    c.hasHl = rnd(2);
    c.isRoot = rnd(6) == 0;
    c.rootMsg = rnd(2) ? "" : "hello";
    static const unsigned char chans[] = { 0, CH_A, HL_B, 0x33 };
    c.bcsChan = chans[rnd(4)];
    c.numHops = rnd(5);
    c.group = rnd(8);
    c.forMe = rnd(2);
    c.streamPos = rnd(2) ? 0 : rnd(100000);
    c.nextRoot = rnd(2) ? 0 : 555;
    c.chStreamPos = rnd(3) ? 0 : rnd(100000);
    return c;
}

static int g_count;
static int g_status;

static void* mainLoop(void*)
{
    // 空回りの打ち切り (SIGALRM) はこのスレッドで受ける
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGALRM);
    pthread_sigmask(SIG_UNBLOCK, &set, nullptr);

    int count = g_count;
    bool trace = getenv("DIFF_PCP_TRACE") != nullptr;
    long same = 0, bad = 0, withEffects = 0;
    std::map<std::string, long> explained;
    for (int i = 0; i < count; i++)
    {
        Case c = genCase();
        if (trace)
            fprintf(stderr, "case %d\n", i);
        std::string a = run(false, c);
        std::string b = run(true, c);
        if (a.empty() || a == "TIMEOUT")
        {
            explained[a.empty() ? "C++ 版がバッファの終わりで空回りしてログを出し続ける (打ち切り)"
                                : "C++ 版がバッファの終わりで空回りする (時間切れで打ち切り)"]++;
            continue;
        }
        if (a == b)
        {
            same++;
            if (a.find("addHit") != std::string::npos || a.find("broadcastPacket") != std::string::npos ||
                a.find("hitlist ") != std::string::npos || a.find("pkt ") != std::string::npos)
                withEffects++;
            continue;
        }
        // Rust 版がこの例外を出すのは入れ子が 64 段を超えたときだけ (パケットの後ろの繰り返しを
        // 子として読んで、生成したよりも深くなることがある)
        if (b.rfind("StreamException PCP: atom nesting too deep\n", 0) == 0)
        {
            explained["入れ子が 64 段を超える (Rust 版は例外)"]++;
            continue;
        }
        if (++bad <= 5)
        {
            printf("---- 違い (%d 件目)\n", i);
            printf("C++:\n%s\nRust:\n%s\n", a.c_str(), b.c_str());
        }
    }
    printf("件数 %d: 同じ %ld (外への働きかけや状態の変化があったもの %ld)\n", count, same, withEffects);
    for (auto& e : explained)
        printf("  説明のつく違い: %s %ld\n", e.first.c_str(), e.second);
    printf("説明のつかない違い %ld\n", bad);
    g_status = bad ? 1 : 0;
    return nullptr;
}

int main(int argc, char** argv)
{
    g_count = argc > 1 ? atoi(argv[1]) : 100000;

    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    signal(SIGALRM, onAlarm);

    // C++ 版は bcst の入れ子ごとに 16KB をスタックに取り、パケットの中で数千段まで入れ子にできる
    // (8MB のスタックでは足りずに落ちる)。比べられるように、大きなスタックのスレッドで動かす。
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGALRM);
    pthread_sigmask(SIG_BLOCK, &set, nullptr);
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 512u << 20);
    pthread_t th;
    if (pthread_create(&th, &attr, mainLoop, nullptr) != 0)
    {
        perror("pthread_create");
        return 2;
    }
    pthread_join(th, nullptr);
    return g_status;
}
