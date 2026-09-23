// チャンネルの情報とホストの一覧 (core/common/chaninfo.cpp と chanhit.cpp) の、C++ 版と Rust 版の差分
// テスト。C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) のクラス。Rust 版は C ABI を直接
// 呼び、返った判断を WITH_RUST_CORE のビルドの橋渡しと同じやり方で当てはめる。
//
// 同じ一覧 (乱数で作ったホストの列) を 2 つ作り、一方に C++ 版の操作、もう一方に Rust 版の操作をして、
// 返り値と、操作のあとの一覧全体を比べる。時刻は MockSys で決める。
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>
#include <vector>

#include "atom.h"
#include "chanhit.h"
#include "chaninfo.h"
#include "peercast_rs.h"
#include "servmgr.h"
#include "sstream.h"
#include "../../../tests/mockpeercast.h"

static long g_bad = 0, g_cases = 0;
static std::mt19937 rng(20260924);
static std::string g_log;

static void logv(const char* k, const char* f, va_list ap)
{
    char b[1024];
    vsnprintf(b, sizeof b, f, ap);
    g_log += std::string(k) + b + "\n";
}
extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("E:", f, ap); va_end(ap); }

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
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", what, show(ctx).c_str(), show(a).c_str(), show(b).c_str());
}

static unsigned R(unsigned n) { return rng() % n; }
static MockSys* msys() { return static_cast<MockSys*>(sys); }

// ---------------------------------------------------------------- 乱数の値

static GnuID randId()
{
    GnuID g;
    switch (R(4))
    {
    case 0: break;
    case 1: g.id[15] = 1; break;
    case 2: g.id[0] = 2; g.id[15] = 2; break;
    default: for (auto& b : g.id) b = rng(); break;
    }
    return g;
}

static IP randIp()
{
    switch (R(7))
    {
    case 0: return IP();
    case 1: return IP(0u);
    case 2: return IP((10u << 24) | R(3));
    case 3: return IP((192u << 24) | (168u << 16) | R(3));
    case 4: { in6_addr a = {}; a.s6_addr[0] = 0x20; a.s6_addr[1] = 0x01; a.s6_addr[15] = R(3); return IP(a); }
    case 5: { in6_addr a = {}; a.s6_addr[15] = 1; return IP(a); }
    default: return IP(rng());
    }
}

static Host randHost()
{
    static const unsigned short ports[] = { 0, 7144, 7145, 65535 };
    return Host(randIp(), ports[R(4)]);
}

static unsigned randNum()
{
    switch (R(5))
    {
    case 0: return 0;
    case 1: return R(4);
    case 2: return R(300);
    case 3: return 0xffffffffu - R(3);
    default: return rng();
    }
}

static ChanHit randHit()
{
    ChanHit h;
    h.host = randHost();
    h.rhost[0] = R(3) ? h.host : randHost();
    h.rhost[1] = randHost();
    h.uphost = R(2) ? Host() : randHost();
    h.numListeners = randNum();
    h.numRelays = randNum();
    h.numHops = R(3) ? R(6) : randNum();
    h.time = 1000 + R(200);
    h.upTime = randNum();
    h.lastContact = 1000 + R(200);
    h.sessionID = randId();
    h.chanID = randId();
    h.version = R(3) ? 0 : randNum();
    h.versionVP = R(2) ? 0 : randNum();
    h.versionExNumber = R(2) ? 0 : randNum();
    static const char* prefixes[] = { "  ", "YT", "ST", "\0\0", "S\0" };
    memcpy(h.versionExPrefix, prefixes[R(5)], 2);
    h.oldestPos = randNum();
    h.newestPos = randNum();
    h.uphostHops = randNum();
    h.firewalled = R(2); h.stable = R(2); h.tracker = R(3) == 0; h.recv = R(2); h.yp = R(2);
    h.dead = R(4) == 0; h.direct = R(2); h.relay = R(2); h.cin = R(2);
    return h;
}

static std::string dumpHost(const Host& h) { return h.ip.str() + ":" + std::to_string(h.port); }

static std::string dumpHit(const ChanHit& h)
{
    char b[512];
    snprintf(b, sizeof b, "%s %s %s %s L%u R%u H%u t%u u%u c%u s%s c%s v%u/%u/%u/%02x%02x o%u n%u uh%u %d%d%d%d%d%d%d%d%d",
             dumpHost(h.host).c_str(), dumpHost(h.rhost[0]).c_str(), dumpHost(h.rhost[1]).c_str(), dumpHost(h.uphost).c_str(),
             h.numListeners, h.numRelays, h.numHops, h.time, h.upTime, h.lastContact, h.sessionID.str().c_str(),
             h.chanID.str().c_str(), h.version, h.versionVP, h.versionExNumber, (unsigned char)h.versionExPrefix[0],
             (unsigned char)h.versionExPrefix[1], h.oldestPos, h.newestPos, h.uphostHops, h.firewalled, h.stable,
             h.tracker, h.recv, h.yp, h.dead, h.direct, h.relay, h.cin);
    return b;
}

static std::string dumpList(const std::shared_ptr<ChanHitList>& l)
{
    std::string s;
    for (auto c = l->hit; c; c = c->next) s += dumpHit(*c) + "\n";
    return s + "last=" + std::to_string(l->lastHitTime);
}

// ---------------------------------------------------------------- Rust 版の橋渡し (chanhit.cpp と同じ)

static pcrs_host rsHost(const Host& h)
{
    pcrs_host r;
    in6_addr a = h.ip.serialize();
    memcpy(r.ip, a.s6_addr, 16);
    r.port = h.port;
    return r;
}

static pcrs_hit rsHit(const ChanHit& h)
{
    pcrs_hit v = {};
    v.host = rsHost(h.host);
    v.rhost[0] = rsHost(h.rhost[0]);
    v.rhost[1] = rsHost(h.rhost[1]);
    v.uphost = rsHost(h.uphost);
    v.num_listeners = h.numListeners;
    v.num_relays = h.numRelays;
    v.num_hops = h.numHops;
    v.time = h.time;
    v.up_time = h.upTime;
    v.last_contact = h.lastContact;
    v.version = h.version;
    v.oldest_pos = h.oldestPos;
    v.newest_pos = h.newestPos;
    v.uphost_hops = h.uphostHops;
    v.version_vp = h.versionVP;
    v.version_ex_number = h.versionExNumber;
    memcpy(v.session_id, h.sessionID.id, 16);
    memcpy(v.version_ex_prefix, h.versionExPrefix, 2);
    v.firewalled = h.firewalled;
    v.tracker = h.tracker;
    v.recv = h.recv;
    v.dead = h.dead;
    v.direct = h.direct;
    v.relay = h.relay;
    v.cin = h.cin;
    return v;
}

struct HitArray
{
    std::vector<std::shared_ptr<ChanHit>> nodes;
    std::vector<pcrs_hit> views;
    std::unique_ptr<bool[]> flags;
    explicit HitArray(const std::shared_ptr<ChanHit>& head)
    {
        for (auto c = head; c; c = c->next) { nodes.push_back(c); views.push_back(rsHit(*c)); }
        flags.reset(new bool[nodes.size() + 1]());
    }
    size_t size() const { return nodes.size(); }
};

static std::shared_ptr<ChanHit> rsAddHit(ChanHitList& l, ChanHit& h)
{
    HitArray a(l.hit);
    pcrs_hit v = rsHit(h);
    int r = pcrs_hits_add(a.views.data(), a.size(), &v, servMgr->sessionID.id, a.flags.get());
    if (r == -2) return nullptr;
    l.lastHitTime = sys->getTime();
    h.time = l.lastHitTime;
    if (r >= 0)
    {
        auto ch = a.nodes[r];
        auto next = ch->next;
        *ch = h;
        ch->next = next;
        return ch;
    }
    for (size_t i = 0; i < a.size(); i++) if (a.flags[i]) l.deleteHit(a.nodes[i]);
    auto ch = std::make_shared<ChanHit>();
    *ch = h;
    ch->chanID = l.info.id;
    ch->next = l.hit;
    l.hit = ch;
    return ch;
}

static int rsPick(ChanHitList& l, ChanHitSearch& chs)
{
    HitArray a(l.hit);
    pcrs_hit_search s = {};
    s.match_host = rsHost(chs.matchHost);
    s.wait_delay = chs.waitDelay;
    s.use_firewalled = chs.useFirewalled;
    s.trackers_only = chs.trackersOnly;
    s.use_busy_relays = chs.useBusyRelays;
    s.use_busy_controls = chs.useBusyControls;
    memcpy(s.exclude_id, chs.excludeID.id, 16);
    s.num_results = chs.numResults;
    unsigned int ctime = sys->getTime();
    bool lan = false;
    int i = pcrs_hits_pick(a.views.data(), a.size(), &s, ctime, &lan);
    if (i < 0) return 0;
    auto bestP = a.nodes[i];
    ChanHit best = *bestP;
    best.host = best.rhost[lan ? 1 : 0];
    if (chs.waitDelay) bestP->lastContact = ctime;
    chs.best[chs.numResults++] = best;
    return 1;
}

// ---------------------------------------------------------------- ホストの一覧

static void hitCase()
{
    msys()->time = 1100 + R(200);
    servMgr->sessionID = randId();

    int n = R(9);
    std::vector<ChanHit> hits;
    for (int i = 0; i < n; i++) hits.push_back(randHit());
    auto make = [&]() {
        auto l = std::make_shared<ChanHitList>();
        l->info.id = GnuID();
        l->info.id.id[3] = 7;
        l->lastHitTime = 5;
        for (int i = n - 1; i >= 0; i--)
        {
            auto c = std::make_shared<ChanHit>();
            *c = hits[i];
            c->next = l->hit;
            l->hit = c;
        }
        return l;
    };
    std::string ctx;
    {
        auto l = make();
        ctx = "time=" + std::to_string(sys->getTime()) + " sid=" + servMgr->sessionID.str() + "\n" + dumpList(l);
    }

    // 1 つずつのホスト
    for (auto& h : hits)
    {
        GnuID cid = R(2) ? GnuID() : randId();
        char buf[4096];
        MemoryStream mem(buf, sizeof buf);
        AtomStream atom(mem);
        h.writeAtoms(atom, cid);
        std::string a(buf, mem.pos);
        pcrs_hit v = rsHit(h);
        report("writeAtoms", a, take(pcrs_hit_write_atoms(&v, cid.id)), dumpHit(h));
        report("versionString", h.versionString(), take(pcrs_hit_version_string(&v)), dumpHit(h));
        report("getColor", std::to_string((int)h.getColor()), std::to_string(pcrs_hit_color(&v)), dumpHit(h));
        report("canGiv", std::to_string(h.canGiv()), std::to_string(pcrs_hit_can_giv(&v)), dumpHit(h));
    }

    // 数え上げ
    {
        auto l = make();
        HitArray a(l->hit);
        auto cnt = [&](int op) { return std::to_string((int)pcrs_hits_count(a.views.data(), a.size(), op)); };
        report("numHits", std::to_string(l->numHits()), cnt(PCRS_HITS_NUM_HITS), ctx);
        report("numListeners", std::to_string(l->numListeners()), cnt(PCRS_HITS_NUM_LISTENERS), ctx);
        report("numRelays", std::to_string(l->numRelays()), cnt(PCRS_HITS_NUM_RELAYS), ctx);
        report("numTrackers", std::to_string(l->numTrackers()), cnt(PCRS_HITS_NUM_TRACKERS), ctx);
        report("numFirewalled", std::to_string(l->numFirewalled()), cnt(PCRS_HITS_NUM_FIREWALLED), ctx);
        report("closestHit", std::to_string(l->closestHit()), cnt(PCRS_HITS_CLOSEST), ctx);
        report("furthestHit", std::to_string(l->furthestHit()), cnt(PCRS_HITS_FURTHEST), ctx);
        report("newestHit", std::to_string(l->newestHit()),
               std::to_string(pcrs_hits_count(a.views.data(), a.size(), PCRS_HITS_NEWEST)), ctx);
        report("getTotalListeners", std::to_string(l->getTotalListeners()), cnt(PCRS_HITS_TOTAL_LISTENERS), ctx);
        report("getTotalRelays", std::to_string(l->getTotalRelays()), cnt(PCRS_HITS_TOTAL_RELAYS), ctx);
        report("getTotalFirewalled", std::to_string(l->getTotalFirewalled()), cnt(PCRS_HITS_TOTAL_FIREWALLED), ctx);
    }

    // pickHits (続けて何回か選ぶ)
    {
        auto la = make(), lb = make();
        ChanHitSearch ca, cb;
        ca.matchHost = R(3) ? Host() : Host(R(2) ? hits.empty() ? randIp() : hits[R(n)].rhost[0].ip : randIp(), 0);
        ca.waitDelay = R(2) ? 0 : R(200);
        ca.useFirewalled = R(2);
        ca.trackersOnly = R(3) == 0;
        ca.useBusyRelays = R(2);
        ca.useBusyControls = R(2);
        ca.excludeID = R(2) ? GnuID() : hits.empty() ? randId() : hits[R(n)].sessionID;
        ca.numResults = R(4) ? 0 : 6 + R(3);
        cb = ca;
        std::string ra, rb;
        for (int k = 0; k < 3; k++)
        {
            ra += std::to_string(la->pickHits(ca)) + ",";
            rb += std::to_string(rsPick(*lb, cb)) + ",";
        }
        ra += std::to_string(ca.numResults) + "\n";
        rb += std::to_string(cb.numResults) + "\n";
        for (int k = 0; k < ca.numResults && k < ChanHitSearch::MAX_RESULTS; k++) ra += dumpHit(ca.best[k]) + "\n";
        for (int k = 0; k < cb.numResults && k < ChanHitSearch::MAX_RESULTS; k++) rb += dumpHit(cb.best[k]) + "\n";
        report("pickHits", ra + dumpList(la), rb + dumpList(lb),
               ctx + "\nsearch match=" + dumpHost(ca.matchHost) + " wait=" + std::to_string(ca.waitDelay) +
                   " fw=" + std::to_string(ca.useFirewalled) + " tr=" + std::to_string(ca.trackersOnly) + " busy=" +
                   std::to_string(ca.useBusyRelays) + std::to_string(ca.useBusyControls) + " ex=" + ca.excludeID.str());
    }

    // clearDeadHits
    {
        auto la = make(), lb = make();
        unsigned timeout = R(300);
        bool clearTrackers = R(2);
        std::string ra = std::to_string(la->clearDeadHits(timeout, clearTrackers));
        HitArray a(lb->hit);
        int cnt = pcrs_hits_clear_dead(a.views.data(), a.size(), timeout, clearTrackers, sys->getTime(), a.flags.get());
        for (size_t i = 0; i < a.size(); i++) if (a.flags[i]) lb->deleteHit(a.nodes[i]);
        report("clearDeadHits", ra + "\n" + dumpList(la), std::to_string(cnt) + "\n" + dumpList(lb), ctx);
    }

    // deadHit / delHit / addHit
    for (int k = 0; k < 3; k++)
    {
        ChanHit h = (n && R(2)) ? hits[R(n)] : randHit();
        if (R(3) == 0) h.rhost[1] = randHost();
        if (R(4) == 0) h.sessionID = servMgr->sessionID;
        if (R(3) == 0 && n) h.sessionID = hits[R(n)].sessionID;
        std::string hctx = ctx + "\nh=" + dumpHit(h);

        auto la = make(), lb = make();
        la->deadHit(h);
        {
            HitArray a(lb->hit);
            pcrs_hit v = rsHit(h);
            pcrs_hits_same_hosts(a.views.data(), a.size(), &v, a.flags.get());
            for (size_t i = 0; i < a.size(); i++) if (a.flags[i]) a.nodes[i]->dead = true;
        }
        report("deadHit", dumpList(la), dumpList(lb), hctx);

        la = make(), lb = make();
        la->delHit(h);
        {
            HitArray a(lb->hit);
            pcrs_hit v = rsHit(h);
            pcrs_hits_same_hosts(a.views.data(), a.size(), &v, a.flags.get());
            for (size_t i = 0; i < a.size(); i++) if (a.flags[i]) lb->deleteHit(a.nodes[i]);
        }
        report("delHit", dumpList(la), dumpList(lb), hctx);

        la = make(), lb = make();
        ChanHit ha = h, hb = h;
        auto pa = la->addHit(ha);
        auto pb = rsAddHit(*lb, hb);
        auto pos = [](const std::shared_ptr<ChanHitList>& l, const std::shared_ptr<ChanHit>& p) {
            int i = 0;
            for (auto c = l->hit; c; c = c->next, i++) if (c == p) return i;
            return p ? -1 : -2;
        };
        report("addHit", std::to_string(pos(la, pa)) + " " + dumpHit(ha) + "\n" + dumpList(la),
               std::to_string(pos(lb, pb)) + " " + dumpHit(hb) + "\n" + dumpList(lb), hctx);
    }
}

// ---------------------------------------------------------------- チャンネルの情報

static const char* STRS[] = {
    "", "FLV", "flv", "OGM", "OGG", "ogm", "MP3", "MOV", "MPG", "MKV", "WEBM", "webm", "MP4", "PLS", "M3U", "m3u",
    "RAW", "UNKNOWN", "WMV", "x", "Hello World", "hello", "WORLD", "ゲーム", "げーむ", "Game", "a\xff", "http://x/",
    "video/x-flv", "application/x-ogg", "audio/mpeg", "video/mp4", "video/webm", "video/x-matroska", "video/quicktime",
    "video/mpeg", "application/octet-stream", "HTTP", "http", "FILE", "PCP", "pcp", "RTMP", "PIPE", "pipe", ".flv", ".x",
};

static const char* randStr() { return STRS[R(sizeof STRS / sizeof *STRS)]; }

static void randInfo(ChanInfo& i)
{
    i.init();
    i.name.set(randStr());
    i.id = randId();
    i.bcID = randId();
    i.bitrate = R(3) ? (int)R(3) * 100 : (int)rng();
    i.contentType.set(R(2) ? randStr() : "UNKNOWN");
    i.MIMEType.set(R(2) ? "" : randStr());
    i.streamExt.set(R(2) ? "" : randStr());
    i.status = R(3) ? ChanInfo::S_UNKNOWN : ChanInfo::S_PLAY;
    i.desc.set(randStr());
    i.genre.set(randStr());
    i.url.set(randStr());
    i.comment.set(randStr());
    i.track.contact.set(randStr());
    i.track.title.set(randStr());
    i.track.artist.set(randStr());
    i.track.album.set(randStr());
    i.track.genre.set(randStr());
    // String の文字コードの種類 (代入で写るもの) も比べられるように変える
    if (R(4) == 0) i.name.type = String::T_UNICODE;
    if (R(4) == 0) i.track.title.type = String::T_ASCII;
}

static pcrs_bytes rsBytes(const char* s) { return { reinterpret_cast<const uint8_t*>(s), strlen(s) }; }

static void rsTrack(pcrs_chan_info& v, const TrackInfo& t)
{
    v.track_contact = rsBytes(t.contact.data);
    v.track_title = rsBytes(t.title.data);
    v.track_artist = rsBytes(t.artist.data);
    v.track_album = rsBytes(t.album.data);
    v.track_genre = rsBytes(t.genre.data);
}

static pcrs_chan_info rsInfo(const ChanInfo& i)
{
    pcrs_chan_info v = {};
    v.name = rsBytes(i.name.data);
    v.content_type = rsBytes(i.contentType.data);
    v.mime = rsBytes(i.MIMEType.data);
    v.ext = rsBytes(i.streamExt.data);
    v.desc = rsBytes(i.desc.data);
    v.genre = rsBytes(i.genre.data);
    v.url = rsBytes(i.url.data);
    v.comment = rsBytes(i.comment.data);
    rsTrack(v, i.track);
    memcpy(v.id, i.id.id, 16);
    memcpy(v.bcid, i.bcID.id, 16);
    v.bitrate = i.bitrate;
    v.status = i.status;
    return v;
}

static bool rsUpdate(ChanInfo& me, const ChanInfo& info)
{
    pcrs_chan_info a = rsInfo(me), b = rsInfo(info);
    uint32_t copy = 0;
    int r = pcrs_chaninfo_update(&a, &b, &copy);
    if (r == 0) return false;
    if (r == 1) { g_log += "E:ChanInfo BC key not valid\n"; return false; }
    if (r == 3) me.bcID = info.bcID;
    if (copy & PCRS_CI_BITRATE) me.bitrate = info.bitrate;
    if (copy & PCRS_CI_CONTENT_TYPE) me.contentType = info.contentType;
    if (copy & PCRS_CI_MIME) me.MIMEType = info.MIMEType;
    if (copy & PCRS_CI_EXT) me.streamExt = info.streamExt;
    if (copy & PCRS_CI_DESC) me.desc = info.desc;
    if (copy & PCRS_CI_NAME) me.name = info.name;
    if (copy & PCRS_CI_COMMENT) me.comment = info.comment;
    if (copy & PCRS_CI_GENRE) me.genre = info.genre;
    if (copy & PCRS_CI_URL) me.url = info.url;
    if (copy & PCRS_CI_TRACK_CONTACT) me.track.contact = info.track.contact;
    if (copy & PCRS_CI_TRACK_TITLE) me.track.title = info.track.title;
    if (copy & PCRS_CI_TRACK_ARTIST) me.track.artist = info.track.artist;
    if (copy & PCRS_CI_TRACK_ALBUM) me.track.album = info.track.album;
    if (copy & PCRS_CI_TRACK_GENRE) me.track.genre = info.track.genre;
    return copy != 0;
}

static std::string dumpStr(const ::String& s) { return std::string(s.data) + "/" + std::to_string((int)s.type); }

static std::string dumpInfo(const ChanInfo& i)
{
    return dumpStr(i.name) + "|" + i.id.str() + "|" + i.bcID.str() + "|" + std::to_string(i.bitrate) + "|" +
           dumpStr(i.contentType) + "|" + dumpStr(i.MIMEType) + "|" + dumpStr(i.streamExt) + "|" +
           std::to_string(i.status) + "|" + dumpStr(i.desc) + "|" + dumpStr(i.genre) + "|" + dumpStr(i.url) + "|" +
           dumpStr(i.comment) + "|" + dumpStr(i.track.contact) + "|" + dumpStr(i.track.title) + "|" +
           dumpStr(i.track.artist) + "|" + dumpStr(i.track.album) + "|" + dumpStr(i.track.genre);
}

static std::string cs(const char* s) { return s ? s : "(null)"; }

static void infoCase()
{
    ChanInfo a, b;
    randInfo(a);
    randInfo(b);
    if (R(3) == 0) b.bcID = a.bcID;
    if (R(3) == 0) b.id = a.id;
    std::string ctx = dumpInfo(a) + "\n" + dumpInfo(b);
    pcrs_chan_info va = rsInfo(a), vb = rsInfo(b);

    // 表
    const char* s = randStr();
    ChanInfo::TYPE t(s);
    report("getTypeExt(TYPE)", cs(ChanInfo::getTypeExt(t)), cs(pcrs_chaninfo_type_ext(rsBytes(s).ptr, strlen(s))), s);
    report("getMIMEType(TYPE)", cs(ChanInfo::getMIMEType(t)), cs(pcrs_chaninfo_mime_type(rsBytes(s).ptr, strlen(s))), s);
    report("getTypeFromStr", ChanInfo::getTypeFromStr(s).c_str(), cs(pcrs_chaninfo_type_from_str(rsBytes(s).ptr, strlen(s))), s);
    std::string m = R(4) ? std::string(s) : std::string(s) + '\0' + "x";
    report("getTypeFromMIME", ChanInfo::getTypeFromMIME(m).c_str(),
           cs(pcrs_chaninfo_type_from_mime(reinterpret_cast<const uint8_t*>(m.data()), m.size())), m);
    report("getProtocolFromStr", std::to_string(ChanInfo::getProtocolFromStr(s)),
           std::to_string(pcrs_chaninfo_protocol_from_str(rsBytes(s).ptr, strlen(s))), s);
    int p = (int)R(9) - 1;
    report("getProtocolStr", cs(ChanInfo::getProtocolStr((ChanInfo::PROTOCOL)p)), cs(pcrs_chaninfo_protocol_str(p)), std::to_string(p));

    report("getTypeExt()", cs(a.getTypeExt()),
           a.streamExt.isEmpty() ? cs(pcrs_chaninfo_type_ext(va.content_type.ptr, va.content_type.len)) : a.streamExt.data, ctx);
    report("getPlayListExt", cs(a.getPlayListExt()), cs(pcrs_chaninfo_playlist_ext(va.content_type.ptr, va.content_type.len)), ctx);
    report("getTypeStringLong", a.getTypeStringLong(), take(pcrs_chaninfo_type_string_long(&va)), ctx);
    report("match", std::to_string(a.match(b)), std::to_string(pcrs_chaninfo_match(&va, &vb, false)), ctx);
    report("matchNameID", std::to_string(a.matchNameID(b)), std::to_string(pcrs_chaninfo_match(&va, &vb, true)), ctx);

    // match の問い合わせは、たいてい項目が少ない
    {
        ChanInfo q;
        q.init();
        if (R(2)) q.name.set(randStr());
        if (R(3) == 0) q.genre.set(randStr());
        if (R(4) == 0) q.bitrate = a.bitrate;
        if (R(4) == 0) q.id = R(2) ? a.id : randId();
        if (R(4) == 0) q.contentType.set(randStr());
        if (R(5) == 0) q.status = ChanInfo::S_PLAY;
        pcrs_chan_info vq = rsInfo(q);
        report("match(q)", std::to_string(a.match(q)), std::to_string(pcrs_chaninfo_match(&va, &vq, false)), ctx + "\nq=" + dumpInfo(q));
    }

    // atom
    {
        char buf[16384];
        MemoryStream mem(buf, sizeof buf);
        AtomStream atom(mem);
        a.writeInfoAtoms(atom);
        a.writeTrackAtoms(atom);
        std::string x(buf, mem.pos);
        std::string y = take(pcrs_chaninfo_write_atoms(&va, false)) + take(pcrs_chaninfo_write_atoms(&va, true));
        report("writeInfoAtoms+writeTrackAtoms", x, y, ctx);
    }

    // update
    {
        ChanInfo ca = a, cb = a;
        g_log.clear();
        bool ra = ca.update(b);
        std::string la = g_log;
        g_log.clear();
        bool rb = rsUpdate(cb, b);
        std::string lb = g_log;
        report("update", std::to_string(ra) + " " + la + dumpInfo(ca), std::to_string(rb) + " " + lb + dumpInfo(cb), ctx);
    }
    {
        TrackInfo ta = a.track, tb = a.track;
        bool ra = ta.update(b.track);
        pcrs_chan_info x = {}, y = {};
        rsTrack(x, tb);
        rsTrack(y, b.track);
        uint32_t c = pcrs_trackinfo_update(&x, &y);
        if (c & PCRS_CI_TRACK_CONTACT) tb.contact = b.track.contact;
        if (c & PCRS_CI_TRACK_TITLE) tb.title = b.track.title;
        if (c & PCRS_CI_TRACK_ARTIST) tb.artist = b.track.artist;
        if (c & PCRS_CI_TRACK_ALBUM) tb.album = b.track.album;
        if (c & PCRS_CI_TRACK_GENRE) tb.genre = b.track.genre;
        ChanInfo xa, xb;
        xa.track = ta;
        xb.track = tb;
        report("TrackInfo::update", std::to_string(ra) + dumpInfo(xa), std::to_string(c != 0) + dumpInfo(xb), ctx);
    }
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 50000;
    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();

    for (long i = 0; i < n; i++)
    {
        hitCase();
        infoCase();
    }
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
