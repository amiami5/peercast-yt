// メディアコンテナの解析 (core/common の flv, mkv, ogg, mp3, mp4) の、C++ 版と Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の FLVStream など。Rust 版は、本番と同じ
// 橋渡しのコード (core/common/rustmedia.h の rustbridge::MediaStream) を通して呼ぶ。どちらにも
// 同じ入力 (StringStream) と同じ状態の Channel を与えて readHeader と readPacket を繰り返し、
// 次のものを比べる。
//  - Channel::newPacket に渡ったパケット (リンク時の --wrap で横取りする) と、そのときの
//    streamPos、streamIndex、ビットレート
//  - sys->sleep の呼び出し (時刻は呼ばれるたびに進むので、時刻を読む回数と順序も比べることになる)
//  - 投げた例外の種類とメッセージ、読み終えた位置、最後の Channel の状態
//
// C++ 版が初期化していないメモリを読む箇所 (読み出しが足りなかったときのバッファの残りなど) は、
// Rust 版では 0 として扱う。比べられるように C++ 版も 0 にする: スタックは
// -ftrivial-auto-var-init=zero (Makefile)、ヒープはこのファイルの operator new で 0 に埋める。
//
// 使い方: ./diff_media [1 種類あたりの件数 (既定 20000)]
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <memory>
#include <new>
#include <random>
#include <string>
#include <vector>

#include "channel.h"
#include "flv.h"
#include "mkv.h"
#include "mp3.h"
#include "mp4.h"
#include "ogg.h"
#include "rustmedia.h"
#include "sstream.h"
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

// ---------------------------------------------------------------- 記録

static std::vector<std::string> g_events;
static bool g_cxx = false;     // C++ 版を動かしている間
static long g_logInfo = 0;

// C++ 版の OGG は、コメント数の値だけ (最大約 21 億回) 空回りしてログを出すことがある。
// 比べられないので、ログの回数が多すぎたら打ち切る。
struct HarnessAbort {};
static const long LOG_LIMIT = 200000;

static uint64_t fnv(const void* p, size_t n)
{
    uint64_t h = 1469598103934665603ULL;
    const unsigned char* s = static_cast<const unsigned char*>(p);
    for (size_t i = 0; i < n; i++) { h ^= s[i]; h *= 1099511628211ULL; }
    return h;
}

extern "C" void __real__ZN7Channel9newPacketER10ChanPacket(Channel*, ChanPacket&);
extern "C" void __wrap__ZN7Channel9newPacketER10ChanPacket(Channel* ch, ChanPacket& p)
{
    __real__ZN7Channel9newPacketER10ChanPacket(ch, p);
    char b[256];
    snprintf(b, sizeof b, "pkt t=%d pos=%u len=%u cont=%d sync=%u h=%016llx | sp=%u si=%u br=%d",
             (int) p.type, p.pos, p.len, (int) p.cont, p.sync, (unsigned long long) fnv(p.data, p.len),
             ch->streamPos, ch->streamIndex, ch->info.bitrate);
    g_events.push_back(b);
}

static std::string hex(const char* s)
{
    std::string r;
    char b[4];
    for (; *s; s++) { snprintf(b, sizeof b, "%02x", (unsigned char) *s); r += b; }
    return r;
}

static std::string trackStr(const TrackInfo& t)
{
    return "track " + hex(t.artist.c_str()) + "/" + hex(t.title.c_str()) + "/" + hex(t.genre.c_str()) + "/" +
           hex(t.contact.c_str()) + "/" + hex(t.album.c_str());
}

// updateInfo (チャンネル情報の更新) も記録する。ただし Channel::processMp3Metadata からの呼び出しは
// 同じオブジェクトファイルの中なので横取りできない (最後の状態で比べる)。
extern "C" bool __real__ZN7Channel10updateInfoERK8ChanInfo(Channel*, const ChanInfo&);
extern "C" bool __wrap__ZN7Channel10updateInfoERK8ChanInfo(Channel* ch, const ChanInfo& info)
{
    bool r = __real__ZN7Channel10updateInfoERK8ChanInfo(ch, info);
    char b[64];
    snprintf(b, sizeof b, "updateInfo %d br=%d ", (int) r, ch->info.bitrate);
    g_events.push_back(b + trackStr(ch->info.track));
    return r;
}

// ログは捨てる (INFO だけ数える)
extern "C" void __wrap__Z9LOG_TRACEPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_WARNPKcz(const char*, ...) {}
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char*, ...) {}
extern "C" void __wrap__Z8LOG_INFOPKcz(const char*, ...)
{
    if (g_cxx && ++g_logInfo > LOG_LIMIT)
        throw HarnessAbort();
}

// 時刻は読むたびに進み、sleep は記録するだけ
class RecSys : public MockSys
{
public:
    double dnow = 1000.0;
    unsigned int now = 1000;
    void sleep(int ms) override
    {
        char b[32];
        snprintf(b, sizeof b, "sleep %d", ms);
        g_events.push_back(b);
    }
    unsigned int getTime() override { return now++; }
    double getDTime() override { dnow += 0.37; return dnow; }
};
static RecSys* recsys;

// readReady の返し方を選べる StringStream
class TestStream : public StringStream
{
public:
    TestStream(const std::string& s, int mode) : StringStream(s), m_mode(mode), m_calls(0) {}
    bool readReady(int) override
    {
        switch (m_mode)
        {
        case 0: return true;
        case 1: return !eof();
        default: return (++m_calls % 3) != 0;
        }
    }
    int m_mode, m_calls;
};

// ---------------------------------------------------------------- 1 件の実行

enum Kind { MP3 = PCRS_MEDIA_MP3, FLV = PCRS_MEDIA_FLV, OGG = PCRS_MEDIA_OGG, MKV = PCRS_MEDIA_MKV, MP4 = PCRS_MEDIA_MP4 };

struct Case
{
    Kind kind;
    std::string input;
    int icy = 0;
    bool readDelay = false;
    int ready = 0;
    int bitrate = 0;
    double rate = 0;
    unsigned int streamPos = 0;
    unsigned int headLen = 0;
    int maxPackets = 60;
};

static ChannelStream* cxxStream(Kind k)
{
    switch (k)
    {
    case MP3: return new MP3Stream();
    case FLV: return new FLVStream();
    case OGG: return new OGGStream();
    case MKV: return new MKVStream();
    default:  return new MP4Stream();
    }
}

// 結果を文字列にする。C++ 版が打ち切られたら空文字列。
static std::string run(bool rust, const Case& c)
{
    g_events.clear();
    g_cxx = !rust;
    g_logInfo = 0;
    recsys->dnow = 1000.0;
    recsys->now = 1000;

    auto ch = std::make_shared<Channel>();
    // ID と名前がないと updateInfo が何も更新しない
    ch->info.id.fromStr("0123456789abcdef0123456789abcdef");
    ch->info.name.set("test");
    ch->icyMetaInterval = c.icy;
    ch->readDelay = c.readDelay;
    ch->info.bitrate = c.bitrate;
    ch->streamPos = c.streamPos;
    ch->startTime = 5.0;
    if (c.headLen)
    {
        ch->headPack.type = ChanPacket::T_HEAD;
        ch->headPack.len = c.headLen;
        memset(ch->headPack.data, 'h', c.headLen);
    }

    TestStream in(c.input, c.ready);
    in.stat.m_bytesInPerSecAvg = c.rate;

    std::unique_ptr<ChannelStream> s(rust ? new rustbridge::MediaStream(c.kind) : cxxStream(c.kind));
    std::string ex = "none";
    int n = 0;
    try {
        s->readHeader(in, ch);
        for (; n < c.maxPackets; n++)
            s->readPacket(in, ch);
    } catch (HarnessAbort&) {
        g_cxx = false;
        return "";
    } catch (StreamException& e) {
        ex = std::string("StreamException:") + e.what();
    } catch (std::runtime_error& e) {
        ex = std::string("runtime_error:") + e.what();
    } catch (std::exception& e) {
        ex = std::string("exception:") + e.what();
    }
    g_cxx = false;

    std::string r;
    for (auto& e : g_events)
        r += e + "\n";
    char b[512];
    const ChanPacket& h = ch->headPack;
    snprintf(b, sizeof b, "%s\nn=%d inpos=%d sp=%u si=%u br=%d ct=%s st=%.3f head t=%d len=%u pos=%u sync=%u cont=%d h=%016llx\n",
             ex.c_str(), n, in.getPosition(), ch->streamPos, ch->streamIndex, ch->info.bitrate, ch->info.contentType.c_str(),
             ch->startTime, (int) h.type, h.len, h.pos, h.sync, (int) h.cont,
             (unsigned long long) fnv(h.data, h.len < ChanPacket::MAX_DATALEN ? h.len : ChanPacket::MAX_DATALEN));
    r += b;
    r += trackStr(ch->info.track) + "\n";
    return r;
}

// ---------------------------------------------------------------- 入力を作る

static std::mt19937 rng(20260923);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }
static bool coin(int percent) { return rnd(100) < percent; }

static std::string randomBytes(int n)
{
    std::string s;
    for (int i = 0; i < n; i++)
        s += (char) rnd(256);
    return s;
}

static std::string filler(int n) { return coin(50) ? std::string(n, (char) rnd(256)) : randomBytes(n); }

static std::string be(uint64_t v, int n)
{
    std::string s;
    for (int i = n - 1; i >= 0; i--)
        s += (char) (v >> (8 * i));
    return s;
}

static std::string le(uint64_t v, int n)
{
    std::string s;
    for (int i = 0; i < n; i++)
        s += (char) (v >> (8 * i));
    return s;
}

// 小さいもの、中くらい、パケットの大きさ (15 KiB、16 KiB) をまたぐもの
static int sizeClass()
{
    int r = rnd(100);
    if (r < 70) return rnd(300);
    if (r < 90) return 1000 + rnd(5000);
    return 14000 + rnd(12000);
}

static std::string mutate(std::string s)
{
    int k = 1 + rnd(4);
    for (int j = 0; j < k; j++)
    {
        int i = rnd((int) s.size() + 1);
        switch (rnd(5))
        {
        case 0: case 1: if (i < (int) s.size()) s[i] = (char) rnd(256); break;
        case 2: s.insert(i, 1, (char) rnd(256)); break;
        case 3: if (i < (int) s.size()) s.erase(i, 1); break;
        default: s.resize(i); break;
        }
    }
    return s;
}

// ---- FLV

static std::string amfString(const std::string& s) { return "\x02" + be(s.size(), 2) + s; }

static std::string amfNumber(double d)
{
    uint64_t u;
    memcpy(&u, &d, 8);
    return std::string(1, '\0') + be(u, 8);
}

static double someNumber()
{
    switch (rnd(8))
    {
    case 0: return 0;
    case 1: return -rnd(1000);
    case 2: return 1e10;
    case 3: return 1e300;
    case 4: return NAN;
    case 5: return rnd(100000) / 7.0;
    default: return rnd(5000);
    }
}

static std::string flvMeta()
{
    std::string v = amfString(coin(85) ? "onMetaData" : (coin(50) ? "onCuePoint" : randomBytes(rnd(12))));
    if (coin(8))
        return v + amfNumber(1.0);
    v += coin(80) ? std::string("\x03") : "\x08" + be(rnd(5), 4);
    int n = rnd(6);
    for (int i = 0; i < n; i++)
    {
        static const char* keys[] = { "videodatarate", "audiodatarate", "width", "duration", "" };
        std::string key = coin(90) ? keys[rnd(5)] : randomBytes(rnd(8));
        if (key.empty())
            key = "x";
        v += be(key.size(), 2) + key;
        int r = rnd(10);
        if (r < 6) v += amfNumber(someNumber());
        else if (r < 7) v += amfString(coin(20) ? std::string(17000, 'j') : "str");
        else if (r < 8) v += "\x01\x01";
        else if (r < 9) v += "\x03" + be(1, 2) + "a" + amfNumber(1) + std::string("\x00\x00\x09", 3);
        else v += std::string(1, (char) rnd(16));
    }
    return v + std::string("\x00\x00\x09", 3);
}

static std::string flvTag(int type, uint32_t ts, const std::string& body)
{
    std::string t(1, (char) type);
    t += be(body.size(), 3);
    t += be(ts & 0xffffff, 3);
    t += (char) (ts >> 24);
    t += be(rnd(3) == 0 ? rnd(256) : 0, 3);
    t += body;
    t += be(body.size() + 11, 4);
    return t;
}

static std::string genFlv()
{
    std::string s = coin(95) ? std::string("FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00", 13) : randomBytes(13);
    int n = rnd(25);
    uint32_t ts = coin(80) ? rnd(1000) : rnd(1 << 30) * 4u;
    for (int i = 0; i < n; i++)
    {
        ts += coin(90) ? rnd(100) : rnd(100000);
        int r = rnd(100);
        if (r < 12)
            s += flvTag(18, ts, flvMeta());
        else if (r < 60)
        {
            static const unsigned char first[] = { 0x17, 0x27, 0x47, 0x12, 0x14 };
            std::string body = filler(sizeClass());
            if (!body.empty())
                body[0] = coin(80) ? (char) first[rnd(5)] : (char) rnd(256);
            s += flvTag(9, coin(10) ? 0 : ts, body);
        }
        else if (r < 95)
            s += flvTag(8, ts, filler(sizeClass()));
        else
            s += flvTag(rnd(256), ts, filler(rnd(100)));
    }
    return s;
}

// ---- MKV

static std::string vint(uint64_t v, int len)
{
    std::string s = be(v, len);
    s[0] = (char) (s[0] | (0x80 >> (len - 1)));
    return s;
}

static std::string sizeVint(uint64_t v)
{
    int len = 1;
    while (len < 8 && v >= (1ULL << (7 * len)) - 1)
        len++;
    if (coin(20))
        len += rnd(9 - len);
    return vint(v, len);
}

static std::string el(const std::string& id, const std::string& body) { return id + sizeVint(body.size()) + body; }

static const std::string EBML("\x1A\x45\xDF\xA3", 4), SEGMENT("\x18\x53\x80\x67", 4), CLUSTER("\x1F\x43\xB6\x75", 4),
    SIMPLEBLOCK("\xA3"), INFO("\x15\x49\xA9\x66", 4), TIMECODESCALE("\x2A\xD7\xB1", 3), TRACKS("\x16\x54\xAE\x6B", 4),
    TRACKENTRY("\xAE"), TRACKNUMBER("\xD7"), TRACKTYPE("\x83"), CODECID("\x86"), TIMECODE("\xE7"), VOID("\xEC"),
    SEEKHEAD("\x11\x4D\x9B\x74", 4), DOCTYPE("\x42\x82", 2), BLOCKGROUP("\xA0");

static std::string mkvBlock(int track)
{
    if (coin(5))
        return el(SIMPLEBLOCK, vint(track, 1) + std::string(rnd(3), 'b'));   // 短い SimpleBlock
    std::string b = vint(track, coin(90) ? 1 : 2) + be(rnd(65536), 2) + (char) (coin(30) ? 0x80 : rnd(128));
    return el(SIMPLEBLOCK, b + filler(sizeClass()));
}

static std::string genMkv()
{
    std::string s = el(EBML, el(DOCTYPE, "webm"));
    if (coin(10))
        s += el(VOID, filler(rnd(50)));
    s += SEGMENT + (coin(5) ? std::string("\xFF") : coin(80) ? std::string("\x01\x00\x00\x00\x00\x00\x00\x00", 8) : sizeVint(rnd(100000)));

    int videoTrack = coin(80) ? 1 + rnd(3) : rnd(256);
    int n = rnd(5);
    for (int i = 0; i < n; i++)
    {
        switch (rnd(4))
        {
        case 0:
        {
            std::string scale = coin(70) ? std::string("\x0F\x42\x40", 3) : randomBytes(rnd(10));
            s += el(INFO, el(TIMECODESCALE, scale) + (coin(20) ? el(VOID, filler(rnd(20))) : "") + (coin(5) ? randomBytes(rnd(5)) : ""));
            break;
        }
        case 1:
        {
            std::string tracks;
            int k = rnd(4);
            for (int j = 0; j < k; j++)
            {
                std::string e;
                if (coin(90)) e += el(TRACKNUMBER, coin(90) ? std::string(1, (char) (j + 1)) : randomBytes(rnd(3)));
                if (coin(20)) e += el(CODECID, "V_VP8");
                if (coin(90)) e += el(TRACKTYPE, std::string(1, (char) (j == videoTrack - 1 ? 1 : coin(80) ? 2 : rnd(4))));
                if (coin(5)) e += randomBytes(rnd(4));
                tracks += el(TRACKENTRY, e);
            }
            if (coin(20)) tracks += el(VOID, filler(rnd(5000)));
            s += el(TRACKS, tracks);
            break;
        }
        case 2: s += el(SEEKHEAD, filler(rnd(60))); break;
        default: s += el(VOID, filler(coin(10) ? 17000 : rnd(100))); break;
        }
    }

    int clusters = rnd(8);
    for (int i = 0; i < clusters; i++)
    {
        std::string body;
        if (coin(90))
            body += el(TIMECODE, be(i * 1000 + rnd(1000), rnd(5)));
        int blocks = rnd(6);
        for (int j = 0; j < blocks; j++)
        {
            if (coin(10))
                body += el(BLOCKGROUP, filler(rnd(200)));
            else
                body += mkvBlock(coin(60) ? videoTrack : 1 + rnd(3));
        }
        s += el(CLUSTER, body);
    }
    return s;
}

// ---- OGG

static std::string oggPage(int flags, uint32_t serial, int64_t granpos, const std::vector<std::string>& packets)
{
    std::string segs, body;
    for (auto& p : packets)
    {
        size_t n = p.size();
        while (n >= 255) { segs += (char) 255; n -= 255; }
        segs += (char) n;
        body += p;
    }
    if (segs.size() > 255)
    {
        segs.resize(255);
        size_t total = 0;
        for (unsigned char c : segs) total += c;
        body.resize(total);
    }
    std::string v = "OggS";
    v += '\0';
    v += (char) flags;
    v += le(granpos, 8);
    v += le(serial, 4);
    v += le(rnd(100), 4);
    v += le(0, 4);
    v += (char) segs.size();
    return v + segs + body;
}

static uint32_t someInt()
{
    switch (rnd(6))
    {
    case 0: return 0;
    case 1: return 0xffffffffu;
    case 2: return 128000;
    case 3: return 44100;
    default: return rng();
    }
}

static std::string vorbisIdent()
{
    std::string v("\x01vorbis", 7);
    v += le(0, 4);
    v += (char) 2;
    v += le(coin(80) ? 44100 : someInt(), 4);
    v += le(someInt(), 4);
    v += le(coin(70) ? 128000 : someInt(), 4);
    v += le(someInt(), 4);
    v += (char) 0xb8;
    v += (char) (coin(95) ? 1 : 0);
    return v;
}

static std::string vorbisComment()
{
    std::string v("\x03vorbis", 7);
    std::string vendor = randomBytes(rnd(20));
    v += le(coin(95) ? vendor.size() : someInt(), 4) + vendor;
    static const char* samples[] = { "TITLE=Song", "artist=Me", "Genre=Rock", "CONTACT=http://example.com/", "album=A",
                                     "x=TITLE=y", "comment=nothing", "ARTIST=" };
    int n = rnd(6);
    v += le(coin(95) ? n : someInt(), 4);
    for (int i = 0; i < n; i++)
    {
        std::string c = coin(80) ? samples[rnd(8)] : randomBytes(rnd(30));
        if (coin(3)) c = std::string(8192, 'T');
        v += le(coin(97) ? c.size() : someInt(), 4) + c;
    }
    v += (char) (coin(95) ? 1 : 0);
    return v;
}

static std::string theoraInfo()
{
    std::string v("\x80theora", 7);
    std::string b = randomBytes(35);
    if (coin(80))
    {
        std::string fps = be(30000, 4) + be(1001, 4);
        b.replace(15, 8, fps);
    }
    return v + b;
}

static std::string genOgg()
{
    std::string s;
    if (coin(10))
        s += coin(50) ? randomBytes(rnd(10)) : "OOgg";
    uint32_t sv = coin(90) ? 0x1000 + rnd(1000) : rnd(3);
    uint32_t st = coin(90) ? 0x2000 + rnd(1000) : rnd(3);
    bool vorbis = coin(85), theora = coin(40);

    for (int round = 0; round < (coin(20) ? 2 : 1); round++)
    {
        if (theora) s += oggPage(2, st, 0, { theoraInfo() });
        if (vorbis) s += oggPage(2, sv, 0, { vorbisIdent() });
        std::vector<std::string> pages;
        if (theora) pages.push_back(oggPage(0, st, 0, { "\x81theora" + randomBytes(rnd(30)), "\x82theora" + randomBytes(rnd(30)) }));
        if (vorbis) pages.push_back(oggPage(0, sv, 0, { vorbisComment(), "\x05vorbis" + randomBytes(rnd(100)) }));
        if (pages.size() == 2 && coin(50)) std::swap(pages[0], pages[1]);
        for (auto& p : pages) s += p;

        int64_t gran = 0;
        int n = rnd(12);
        for (int i = 0; i < n; i++)
        {
            gran += coin(90) ? rnd(5000) : (int64_t) rng() << 20;
            uint32_t ser = coin(45) ? sv : coin(80) ? st : rnd(3);
            int flags = coin(5) ? 4 : coin(3) ? 2 : coin(3) ? 1 : 0;
            int len = coin(95) ? rnd(600) : 16000 + rnd(3000);
            s += oggPage(flags, ser, gran, { filler(len) });
        }
        sv += 7;
        st += 7;
    }
    return s;
}

// ---- MP4

static std::string mp4Box(const char* type, int bodyLen)
{
    return be(bodyLen + 8, 4) + std::string(type, 4) + filler(bodyLen);
}

static std::string genMp4()
{
    std::string s = mp4Box(coin(95) ? "ftyp" : "free", rnd(30));
    s += mp4Box(coin(95) ? "moov" : "mdat", coin(90) ? rnd(3000) : 16000 + rnd(2000));
    int n = rnd(6);
    for (int i = 0; i < n; i++)
    {
        s += mp4Box(coin(95) ? "moof" : "mdat", rnd(500));
        s += mp4Box(coin(95) ? "mdat" : "moof", coin(80) ? sizeClass() : 30000 + rnd(40000));
    }
    if (coin(10))
        s += be(rnd(8), 4) + "moof";
    return s;
}

// ---- MP3

static std::string genMp3(int icy)
{
    if (icy <= 0)
        return filler(rnd(30000));
    std::string s;
    int n = rnd(5);
    for (int i = 0; i < n; i++)
    {
        s += filler(icy);
        int r = rnd(10);
        if (r < 4)
            s += '\0';
        else if (r < 9)
        {
            std::string meta = coin(80) ? "StreamTitle='title " + std::to_string(i) + "';StreamUrl='http://x/';" : randomBytes(rnd(60));
            int len = (int) (meta.size() + 15) / 16;
            meta.resize(len * 16, '\0');
            s += (char) len + meta;
        }
        else
            s += (char) 255 + filler(255 * 16);
    }
    return s;
}

static Case genCase(Kind k)
{
    Case c;
    c.kind = k;
    switch (k)
    {
    case FLV: c.input = genFlv(); break;
    case MKV: c.input = genMkv(); break;
    case OGG: c.input = genOgg(); break;
    case MP4: c.input = genMp4(); break;
    case MP3:
    {
        static const int icys[] = { 0, 0, 1, 16, 100, 8192, 8193, 20000, -3 };
        c.icy = icys[rnd(9)];
        c.input = genMp3(c.icy);
        break;
    }
    }
    if (coin(60))
        c.input = mutate(c.input);
    c.readDelay = coin(50);
    c.ready = rnd(3);
    c.bitrate = coin(70) ? 0 : coin(50) ? 100 : -1;
    c.rate = coin(50) ? 0 : rnd(100000);
    c.streamPos = coin(70) ? 0 : rng();
    c.headLen = coin(70) ? 0 : rnd(3000);
    return c;
}

// ---------------------------------------------------------------- 比較

// C++ 版は FLV のメタデータのビットレートを int に変換するとき範囲を確かめておらず、x86 では
// 2^31 以上の値が -2^31 になる。Rust 版は int の最大値にする。
static bool bitrateOverflow(const std::string& cxx, std::string rs)
{
    const std::string from = "br=2147483647", to = "br=-2147483648";
    size_t p;
    bool changed = false;
    while ((p = rs.find(from)) != std::string::npos)
    {
        rs.replace(p, from.size(), to);
        changed = true;
    }
    return changed && rs == cxx;
}

int main(int argc, char** argv)
{
    int count = argc > 1 ? atoi(argv[1]) : 20000;

    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    recsys = new RecSys();
    sys = recsys;

    const Kind kinds[] = { FLV, MKV, OGG, MP4, MP3 };
    const char* names[] = { "flv", "mkv", "ogg", "mp4", "mp3" };
    long totalBad = 0;
    for (int ki = 0; ki < 5; ki++)
    {
        long same = 0, bad = 0, withPackets = 0, withInfo = 0;
        std::map<std::string, long> explained;
        for (int i = 0; i < count; i++)
        {
            Case c = genCase(kinds[ki]);
            std::string a = run(false, c);
            std::string b = run(true, c);
            if (a.empty())
            {
                explained["C++ 版が空のコメントで空回りする (打ち切り)"]++;
                continue;
            }
            if (a == b)
            {
                same++;
                if (a.find("pkt ") != std::string::npos)
                    withPackets++;
                if (a.find("updateInfo 1") != std::string::npos || a.find("\ntrack ////\n") == std::string::npos)
                    withInfo++;
                continue;
            }
            if (kinds[ki] == FLV && bitrateOverflow(a, b))
            {
                explained["メタデータのビットレートが int の範囲を超える"]++;
                continue;
            }
            if (++bad <= 3)
            {
                char fn[64];
                snprintf(fn, sizeof fn, "/tmp/diff_media_%s_%ld.bin", names[ki], bad);
                FILE* f = fopen(fn, "wb");
                if (f) { fwrite(c.input.data(), 1, c.input.size(), f); fclose(f); }
                printf("--- %s: 違い (入力 %s, icy=%d delay=%d ready=%d br=%d rate=%g sp=%u head=%u)\n", names[ki], fn,
                       c.icy, (int) c.readDelay, c.ready, c.bitrate, c.rate, c.streamPos, c.headLen);
                printf("C++:\n%s\nRust:\n%s\n", a.c_str(), b.c_str());
            }
        }
        printf("%s: %d 件、一致 %ld (うちパケットを出したもの %ld、チャンネル情報を更新したもの %ld)",
               names[ki], count, same, withPackets, withInfo);
        for (auto& e : explained)
            printf("、%s %ld", e.first.c_str(), e.second);
        printf("、説明のつかない違い %ld\n", bad);
        totalBad += bad;
    }
    printf("説明のつかない違い: 合計 %ld\n", totalBad);
    return totalBad ? 1 : 0;
}
