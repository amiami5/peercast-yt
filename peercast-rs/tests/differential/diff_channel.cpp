// チャンネル (core/common/channel.cpp) と ChanMgr (chanmgr.cpp) のうち、Rust に移した部分の C++ 版と
// Rust 版の差分テスト。C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) のクラス。Rust 版は
// C ABI を直接呼び、WITH_RUST_CORE のビルドの橋渡しと同じやり方で結果を当てはめる。
//
// ICY のメタデータ、トラッカーへの更新と情報の更新の atom (送る直前のパケットを横取りする)、
// renderHexDump、getBufferString、checkReadDelay (眠った時間を記録する)、authToken、closeOldestIdle。
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <random>
#include <string>
#include <vector>

#include "atom.h"
#include "chanmgr.h"
#include "channel.h"
#include "peercast_rs.h"
#include "servmgr.h"
#include "sstream.h"
#include "../../../tests/mockpeercast.h"

static long g_bad = 0, g_cases = 0;
static std::mt19937 rng(20260926);

extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char*, ...) {}
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
        printf("  [違い] %s %s\n    C++ =%s\n    Rust=%s\n", what, show(ctx.substr(0, 2000)).c_str(),
               show(a.substr(0, 2000)).c_str(), show(b.substr(0, 2000)).c_str());
}

static unsigned R(unsigned n) { return rng() % n; }

// 眠った時間を記録する Sys
class RecSys : public MockSys
{
public:
    void sleep(int ms) override { slept.push_back(ms); }
    std::vector<int> slept;
};
static RecSys* rsys() { return static_cast<RecSys*>(sys); }

// ---------------------------------------------------------------- 横取り

// ICY のメタデータで曲名などが変わった件数
static long g_metaChanged = 0;

struct Sent { int type; std::string data; };
static std::vector<Sent> g_sent;
extern "C" int __wrap__ZN7ServMgr15broadcastPacketER10ChanPacketRK5GnuIDS4_S4_N7Servent4TYPEE(ServMgr*, ChanPacket& p, const GnuID&, const GnuID&, const GnuID&, int type)
{
    g_sent.push_back({ type, std::string(p.data, p.data + p.len) });
    return 1;
}
extern "C" void __wrap__ZN8peercast13notifyMessageEN7ServMgr11NOTIFY_TYPEERKNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEEE(int, const std::string&) {}

// ---------------------------------------------------------------- 橋渡し (rustchan.h と同じ)

static pcrs_bytes rsBytes(const char* s) { return { reinterpret_cast<const uint8_t*>(s), strlen(s) }; }

static pcrs_chan_info rsInfo(const ChanInfo& i)
{
    pcrs_chan_info v = {};
    v.name = rsBytes(i.name.data); v.content_type = rsBytes(i.contentType.data); v.mime = rsBytes(i.MIMEType.data);
    v.ext = rsBytes(i.streamExt.data); v.desc = rsBytes(i.desc.data); v.genre = rsBytes(i.genre.data);
    v.url = rsBytes(i.url.data); v.comment = rsBytes(i.comment.data);
    v.track_contact = rsBytes(i.track.contact.data); v.track_title = rsBytes(i.track.title.data);
    v.track_artist = rsBytes(i.track.artist.data); v.track_album = rsBytes(i.track.album.data);
    v.track_genre = rsBytes(i.track.genre.data);
    memcpy(v.id, i.id.id, 16); memcpy(v.bcid, i.bcID.id, 16);
    v.bitrate = i.bitrate; v.status = i.status;
    return v;
}

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
    v.host = rsHost(h.host); v.rhost[0] = rsHost(h.rhost[0]); v.rhost[1] = rsHost(h.rhost[1]); v.uphost = rsHost(h.uphost);
    v.num_listeners = h.numListeners; v.num_relays = h.numRelays; v.num_hops = h.numHops; v.time = h.time;
    v.up_time = h.upTime; v.last_contact = h.lastContact; v.version = h.version; v.oldest_pos = h.oldestPos;
    v.newest_pos = h.newestPos; v.uphost_hops = h.uphostHops; v.version_vp = h.versionVP;
    v.version_ex_number = h.versionExNumber;
    memcpy(v.session_id, h.sessionID.id, 16); memcpy(v.version_ex_prefix, h.versionExPrefix, 2);
    v.firewalled = h.firewalled; v.tracker = h.tracker; v.recv = h.recv; v.dead = h.dead; v.direct = h.direct;
    v.relay = h.relay; v.cin = h.cin;
    return v;
}

// channel.cpp の WITH_RUST_CORE の writeTrackerUpdateAtom と同じ
static std::string rsTrackerUpdate(Channel& ch)
{
    int numListeners = ch.totalListeners();
    int numRelays = ch.totalRelays();
    unsigned int oldp = ch.rawData.getOldestPos();
    unsigned int newp = ch.rawData.getLatestPos();
    ChanHit hit;
    hit.initLocal(numListeners, numRelays, ch.info.numSkips, ch.info.getUptime(), ch.isPlaying(),
                  oldp, newp, ch.canAddRelay(), ch.sourceHost.host, (ch.ipVersion == Channel::IP_V6));
    hit.tracker = true;
    pcrs_chan_info v = rsInfo(ch.info);
    pcrs_hit h = rsHit(hit);
    return take(pcrs_channel_tracker_update_atom(&v, &h, servMgr->sessionID.id, chanMgr->broadcastID.id));
}

// ---------------------------------------------------------------- 乱数の値

static std::string randStr()
{
    static const char* words[] = { "", "a", "YP", "http://x/", "日本語", "\x82\xa0\x82\xa2", "'q'", "&amp;", "a;b", "=" };
    switch (R(6))
    {
    case 0: { std::string s; int n = R(12); for (int i = 0; i < n; i++) s += (char)(1 + R(255)); return s; }
    case 1: return std::string(250 + R(20), 'x');
    default: return words[R(sizeof words / sizeof words[0])];
    }
}

static void randId(GnuID& g) { for (auto& b : g.id) b = R(4) ? rng() : 0; if (R(5) == 0) g.clear(); }

static void randInfo(ChanInfo& i)
{
    i.name.set(randStr().c_str()); i.genre.set(randStr().c_str()); i.url.set(randStr().c_str());
    i.desc.set(randStr().c_str()); i.comment.set(randStr().c_str());
    static const char* types[] = { "MP3", "FLV", "OGG", "UNKNOWN", "", "MKV" };
    i.contentType.set(types[R(6)]);
    i.MIMEType.set(R(2) ? "" : randStr().c_str());
    i.streamExt.set(R(2) ? "" : ".flv");
    i.track.title.set(randStr().c_str()); i.track.artist.set(randStr().c_str());
    i.track.album.set(randStr().c_str()); i.track.contact.set(randStr().c_str());
    i.track.genre.set(randStr().c_str());
    randId(i.id);
    randId(i.bcID);
    i.bitrate = R(3) ? R(2000) : rng();
}

// ---------------------------------------------------------------- 各場合

static std::string metaPart()
{
    static const char* names[] = { "StreamTitle", "StreamUrl", "streamtitle", "X", "", "StreamTitle ", ";StreamUrl" };
    std::string s = names[R(7)];
    if (R(8)) s += "=";
    s += R(2) ? "'" + randStr() + "'" : randStr();
    if (R(6)) s += ";";
    return s;
}

static void metadataCase()
{
    std::string s;
    int n = R(5);
    for (int i = 0; i < n; i++) s += metaPart();
    if (R(10) == 0 && !s.empty()) s[R(s.size())] = '\0';

    // 同じ .cpp の中の呼び出しは横取りできないので、本物の updateInfo が写したあとの情報を比べる。
    // 写されるように、ID を設定し、名前を空でなくし、放送者のキーを空にしておく。
    auto ch = std::make_shared<Channel>();
    randInfo(ch->info);
    ch->info.id.id[0] = 1;
    ch->info.name.set("ch");
    ch->info.bcID.clear();
    auto ch2 = std::make_shared<Channel>();
    ch2->info = ch->info;

    // C++ 版
    std::vector<char> buf(s.begin(), s.end());
    buf.push_back(0);
    ch->processMp3Metadata(buf.data());
    std::string a = ch->info.track.title.str() + "|" + ch->info.track.contact.str();
    if (a != ch2->info.track.title.str() + "|" + ch2->info.track.contact.str()) g_metaChanged++;

    // Rust 版 (channel.cpp の WITH_RUST_CORE の processMp3Metadata と同じ)
    ChanInfo newInfo = ch2->info;
    const char* str = s.c_str();
    size_t tpos = 0, tlen = 0, upos = 0, ulen = 0;
    int found = pcrs_channel_mp3_metadata(reinterpret_cast<const uint8_t*>(str), strlen(str), &tpos, &tlen, &upos, &ulen);
    if (found & 1)
    {
        newInfo.track.title.setUnquote(std::string(str + tpos, tlen).c_str(), String::T_ASCII);
        newInfo.track.title.convertTo(String::T_UNICODE);
    }
    if (found & 2)
    {
        newInfo.track.contact.setUnquote(std::string(str + upos, ulen).c_str(), String::T_ASCII);
        newInfo.track.contact.convertTo(String::T_UNICODE);
    }
    ch2->updateInfo(newInfo);
    std::string b = ch2->info.track.title.str() + "|" + ch2->info.track.contact.str();
    report("processMp3Metadata", a, b, s);
}

static void atomCase()
{
    auto ch = std::make_shared<Channel>();
    randInfo(ch->info);
    ch->type = Channel::T_BROADCAST;
    ch->status = R(4) ? Channel::S_BROADCASTING : Channel::S_RECEIVING;
    ch->ipVersion = R(2) ? Channel::IP_V4 : Channel::IP_V6;
    ch->sourceHost.host = R(2) ? Host() : Host(rng(), R(65536));
    ch->info.numSkips = R(5);
    ch->info.lastPlayStart = R(2) ? 0 : 900;
    ch->lastMetaUpdate = R(4) ? 0 : 990;
    ch->lastTrackerUpdate = R(4) ? 0 : 990;
    for (auto& b : servMgr->sessionID.id) b = rng();
    for (auto& b : chanMgr->broadcastID.id) b = rng();
    rsys()->time = 1000 + R(10);
    if (R(5)) chanMgr->addHitList(ch->info);

    // writeTrackerUpdateAtom (ヒットリストがなければ例外)
    {
        char buf[16384];
        MemoryStream mem(buf, sizeof buf);
        AtomStream atom(mem);
        std::string a;
        try { ch->writeTrackerUpdateAtom(atom); a = std::string(buf, mem.pos); }
        catch (StreamException& e) { a = std::string("E:") + e.msg; }
        std::string b = chanMgr->findHitListByID(ch->info.id) ? rsTrackerUpdate(*ch)
                                                                : "E:Broadcast channel has no hitlist";
        report("writeTrackerUpdateAtom", a, b);
    }

    // updateInfo が送るパケット
    ChanInfo newInfo = ch->info;
    randInfo(newInfo);
    newInfo.id = ch->info.id;
    if (R(3) == 0) newInfo.bcID = ch->info.bcID;
    g_sent.clear();
    std::string a, b;
    try
    {
        bool r = ch->updateInfo(newInfo);
        a = std::to_string(r);
    }
    catch (StreamException& e) { a = std::string("E:") + e.msg; }
    for (auto& p : g_sent) a += "|" + std::to_string(p.type) + ":" + p.data;

    // Rust 版: 送ったはずのものを、更新後の ch->info から作る
    b = a.substr(0, a.find('|'));
    for (auto& p : g_sent)
    {
        pcrs_chan_info v = rsInfo(ch->info);
        if (p.type == Servent::T_RELAY)
            b += "|" + std::to_string(p.type) + ":" + take(pcrs_channel_info_update_atom(&v, servMgr->sessionID.id));
        else
            b += "|" + std::to_string(p.type) + ":" + rsTrackerUpdate(*ch);
    }
    report("updateInfo", a, b);
    chanMgr->clearHitLists();
}

static void hexCase()
{
    std::string s;
    int n = R(4) ? R(40) : R(300);
    for (int i = 0; i < n; i++) s += (char) rng();
    report("renderHexDump", Channel::renderHexDump(s),
           take(pcrs_channel_hex_dump(reinterpret_cast<const uint8_t*>(s.data()), s.size())));
}

class RateSource : public ChannelSource
{
public:
    explicit RateSource(int r) : rate(r) {}
    void stream(std::shared_ptr<Channel>) override {}
    int getSourceRateAvg() override { return rate; }
    int rate;
};

static void bufferCase()
{
    auto ch = std::make_shared<Channel>();
    static const int rates[] = { 0, 1, 3, 8, 1000, 16000, 0x7fffffff };
    if (R(4)) ch->sourceData = std::make_shared<RateSource>(R(2) ? rates[R(7)] : (int) R(100000));
    int n = R(4) ? R(20) : 0;
    unsigned int pos = R(2) ? 0 : rng();
    char data[8192] = {};
    for (int i = 0; i < n; i++)
    {
        ChanPacket p;
        unsigned int len = R(4) ? 1 + R(8000) : 1 + R(10);
        p.init(R(3) ? ChanPacket::T_DATA : ChanPacket::T_HEAD, data, len, pos);
        p.cont = R(2);
        pos += len;
        ch->rawData.writePacket(p, true);
    }
    rsys()->time = R(2) ? 1000 + R(100000) : rng();
    ch->rawData.lastWriteTime = R(2) ? rsys()->time - R(10) : rng();

    std::string a = ch->getBufferString();

    // channel.cpp の WITH_RUST_CORE の getBufferString と同じ
    double byterate = (ch->sourceData) ? ch->sourceData->getSourceRateAvg() : 0.0;
    unsigned int now = sys->getTime();
    auto stat = ch->rawData.getStatistics();
    std::string b = take(pcrs_channel_buffer_string(byterate, now, ch->rawData.lastWriteTime,
                                                    stat.packetLengths.data(), stat.packetLengths.size(),
                                                    stat.continuations, stat.nonContinuations));
    report("getBufferString", a, b);
}

static void readDelayCase()
{
    auto ch = std::make_shared<Channel>();
    ch->readDelay = R(4) != 0;
    // bitrate * 1024 が桁あふれしない範囲 (C++ 版は桁あふれで未定義の動作、0 で割ることもある)
    static const int rates[] = { 0, 1, 7, 8, 128, 320, -1, 2097151, -2097152 };
    ch->info.bitrate = R(2) ? rates[R(9)] : (int)(rng() % 4194304) - 2097152;
    unsigned int len = R(2) ? R(20000) : rng();

    rsys()->slept.clear();
    ch->checkReadDelay(len);
    std::string a;
    for (int t : rsys()->slept) a += std::to_string(t) + ";";

    std::string b;
    unsigned int time;
    if (pcrs_channel_read_delay(ch->readDelay, len, ch->info.bitrate, &time))
        b = std::to_string((int) time) + ";";
    report("checkReadDelay", a, b, std::to_string(ch->info.bitrate) + " " + std::to_string(len));
}

static void chanMgrCase()
{
    // authToken
    for (auto& b : chanMgr->broadcastID.id) b = rng();
    GnuID id;
    randId(id);
    report("authToken", chanMgr->authToken(id), take(pcrs_chanmgr_auth_token(chanMgr->broadcastID.id, id.id)));

    // closeOldestIdle
    ChanMgr mgr;
    std::vector<std::shared_ptr<Channel>> chs;
    int n = R(7);
    for (int i = 0; i < n; i++)
    {
        auto c = std::make_shared<Channel>();
        c->type = R(5) ? Channel::T_RELAY : Channel::T_NONE;
        c->status = R(4) ? Channel::S_IDLE : Channel::S_RECEIVING;
        c->thread.m_active = R(5) != 0;
        static const unsigned times[] = { 0, 1, 5, 5, 100, 0xfffffffeu, 0xffffffffu };
        c->lastIdleTime = times[R(7)];
        chs.push_back(c);
    }
    for (int i = n - 1; i >= 0; i--)
    {
        chs[i]->next = mgr.channel;
        mgr.channel = chs[i];
    }

    // Rust 版 (chanmgr.cpp の WITH_RUST_CORE の closeOldestIdle と同じ判断)
    std::unique_ptr<bool[]> idle(new bool[chs.size() + 1]());
    std::vector<uint32_t> times;
    for (size_t i = 0; i < chs.size(); i++)
    {
        idle[i] = chs[i]->isActive() && chs[i]->thread.active() && chs[i]->status == Channel::S_IDLE;
        times.push_back(chs[i]->lastIdleTime);
    }
    ptrdiff_t k = pcrs_chanmgr_oldest_idle(idle.get(), times.data(), chs.size());
    std::string b;
    for (size_t i = 0; i < chs.size(); i++) b += (chs[i]->thread.active() && (ptrdiff_t) i != k) ? "1" : "0";

    mgr.closeOldestIdle();
    std::string a;
    for (auto& c : chs) a += c->thread.active() ? "1" : "0";
    report("closeOldestIdle", a, b);

    for (auto& c : chs) { c->thread.m_active = false; c->next = nullptr; }
    mgr.channel = nullptr;
}

int main(int argc, char** argv)
{
    long n = argc > 1 ? atol(argv[1]) : 50000;
    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    sys = new RecSys();

    for (long i = 0; i < n; i++)
    {
        metadataCase();
        atomCase();
        hexCase();
        bufferCase();
        readDelayCase();
        chanMgrCase();
    }
    printf("メタデータで情報が変わった件数 %ld\n", g_metaChanged);
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_cases, g_bad);
    return g_bad ? 1 : 0;
}
