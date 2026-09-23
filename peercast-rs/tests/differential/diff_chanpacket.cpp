// チャンネルのパケットのバッファ (core/common/chanpacket.cpp の ChanPacketBuffer) の、C++ 版と
// Rust 版の差分テスト。
//
// C++ 版は Rust を使わずにビルドしたコア一式 (cxxcore.a) の ChanPacketBuffer。Rust 版は、同じ並びの
// 記憶領域 (パケット 64 個と位置) に対して peercast-rs の pcrs_cpb_* を呼ぶ (本番の橋渡しの
// chanpacket.cpp は gtest の chanpacketbuffer_unittest で確かめる)。どちらにも同じ操作の列を与え、
// 操作ごとに返り値、位置、パケットの長さなどを比べ、列の終わりにパケットの中身 (全体) も比べる。
//
// 位置は、通し番号が 2^32 近くで一周するところも試す。ただし C++ 版は lastPos が UINT_MAX だと
// ループが終わらないので、そこには届かないようにする。copyFrom は、C++ 版が packets[writePos++]
// を剰余なしで書く (配列の外に書く) ので、書き先の writePos が 0 のときだけ試す。
//
// 使い方: ./diff_chanpacket [操作の列の数 (既定 20000)]
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <new>
#include <random>
#include <string>

#include "chanpacket.h"
#include "peercast_rs.h"
#include "../../../tests/mocksys.h"

// C++ 版のパケットの中身 (ChanPacket は data を初期化しない) を比べられるように、ヒープを 0 で埋める
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

static std::mt19937 rng(20260925);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }

static uint64_t fnv(const void* p, size_t n)
{
    uint64_t h = 1469598103934665603ULL;
    const unsigned char* s = static_cast<const unsigned char*>(p);
    for (size_t i = 0; i < n; i++) { h ^= s[i]; h *= 1099511628211ULL; }
    return h;
}

// Rust 版の記憶領域
struct Mirror
{
    pcrs_chan_packet packets[64];
    uint32_t last, first, safe, read, write, accept, lastWrite;

    pcrs_cpb view() { return { packets, &last, &first, &safe, &read, &write, &accept, &lastWrite }; }
};

static std::string state(ChanPacketBuffer& c, bool data)
{
    char b[160];
    snprintf(b, sizeof b, "last=%u first=%u safe=%u read=%u write=%u accept=%u lw=%u", c.lastPos, c.firstPos, c.safePos,
             c.readPos, c.writePos, c.accept, c.lastWriteTime);
    std::string s = b;
    for (auto& p : c.packets)
        s += " " + std::to_string(p.type) + "/" + std::to_string(p.len) + "/" + std::to_string(p.pos) + "/" +
             std::to_string(p.sync) + "/" + std::to_string(p.cont) + "/" + (data ? std::to_string(fnv(p.data, sizeof p.data)) : std::string());
    return s;
}
static std::string state(Mirror& m, bool data)
{
    char b[160];
    snprintf(b, sizeof b, "last=%u first=%u safe=%u read=%u write=%u accept=%u lw=%u", m.last, m.first, m.safe, m.read,
             m.write, m.accept, m.lastWrite);
    std::string s = b;
    for (auto& p : m.packets)
        s += " " + std::to_string(p.type) + "/" + std::to_string(p.len) + "/" + std::to_string(p.pos) + "/" +
             std::to_string(p.sync) + "/" + std::to_string(p.cont) + "/" + (data ? std::to_string(fnv(p.data, sizeof p.data)) : std::string());
    return s;
}
static std::string pk(const ChanPacket& p)
{
    return std::to_string(p.type) + "/" + std::to_string(p.len) + "/" + std::to_string(p.pos) + "/" +
           std::to_string(p.sync) + "/" + std::to_string(p.cont) + "/" + std::to_string(fnv(p.data, p.len));
}
static std::string pk(const pcrs_chan_packet& p)
{
    return std::to_string(p.type) + "/" + std::to_string(p.len) + "/" + std::to_string(p.pos) + "/" +
           std::to_string(p.sync) + "/" + std::to_string(p.cont) + "/" + std::to_string(fnv(p.data, p.len));
}

static MockSys* g_sys;
static long g_ops = 0, g_bad = 0;

static void check(const std::string& what, const std::string& a, const std::string& b)
{
    g_ops++;
    if (a != b && ++g_bad <= 5)
        printf("---- 違い: %s\nC++:  %.300s\nRust: %.300s\n", what.c_str(), a.c_str(), b.c_str());
}

static void runSeq()
{
    auto* c = new ChanPacketBuffer();
    auto* m = new Mirror();
    auto* c2 = new ChanPacketBuffer();
    auto* m2 = new Mirror();
    pcrs_cpb v = m->view(), v2 = m2->view();

    // 位置の初期値 (一周するところの近くも試す。writePacket で作れる形の位置だけ)
    static const uint32_t starts[] = { 0, 1, 60, 63, 64, 100, 0x7ffffff0u, 0xfffffe00u };
    if (rnd(3) == 0)
    {
        uint32_t w = starts[rnd(8)];
        uint32_t vals[7] = { w ? w - 1 : 0, w >= 64 ? w - 64 : 0, w >= 56 ? w - 56 : 0, w - (uint32_t) rnd(70), w, (uint32_t) rnd(20), 0 };
        c->lastPos = m->last = vals[0];
        c->firstPos = m->first = vals[1];
        c->safePos = m->safe = vals[2];
        c->readPos = m->read = vals[3];
        c->writePos = m->write = vals[4];
        c->accept = m->accept = vals[5];
    }
    if (c->lastPos >= 0xffffff00u)
        c->lastPos = m->last = 0xffffff00u - 1;

    int n = 1 + rnd(150);
    for (int i = 0; i < n; i++)
    {
        g_sys->time = rnd(100000);
        int op = rnd(16);
        switch (op)
        {
        case 0: case 1: case 2: case 3:
        {
            ChanPacket p;
            pcrs_chan_packet* q = new pcrs_chan_packet();
            p.type = static_cast<ChanPacket::TYPE>(rnd(3) ? 2 : rnd(20));
            p.len = rnd(10) ? rnd(40) : 0;
            p.pos = rnd(3) ? (uint32_t) (i * 100) : (uint32_t) rng();
            p.cont = rnd(3) == 0;
            p.sync = rnd(1000);
            for (unsigned k = 0; k < p.len; k++) p.data[k] = (char) rnd(256);
            q->type = p.type; q->len = p.len; q->pos = p.pos; q->cont = p.cont; q->sync = p.sync;
            memcpy(q->data, p.data, p.len);
            bool upd = rnd(3) == 0;
            bool a = c->writePacket(p, upd);
            bool b = pcrs_cpb_write_packet(&v, q, upd, g_sys->time);
            check("writePacket", std::to_string(a) + " " + pk(p), std::to_string(b) + " " + pk(*q));
            delete q;
            break;
        }
        case 4:
        {
            // 読めるときか、遅れすぎのときだけ (まだないと C++ 版は待つ)
            if (c->writePos <= c->readPos && !(c->readPos < c->firstPos))
                break;
            ChanPacket p;
            pcrs_chan_packet* q = new pcrs_chan_packet();
            std::string a, b;
            try { c->readPacket(p); a = "ok " + pk(p); }
            catch (StreamException& e) { a = std::string("exc ") + e.msg; }
            int st = pcrs_cpb_read_state(&v, true);
            if (st == 1) b = "exc Read too far behind";
            else if (st == 2) b = "empty";
            else { pcrs_cpb_take(&v, q); b = "ok " + pk(*q); }
            check("readPacket", a, b);
            delete q;
            break;
        }
        case 5:
        {
            uint32_t spos = rnd(2) ? (uint32_t) (rnd(n) * 100) : (uint32_t) rng();
            ChanPacket p;
            pcrs_chan_packet* q = new pcrs_chan_packet();
            bool a = c->findPacket(spos, p);
            bool b = pcrs_cpb_find_packet(&v, spos, q);
            check("findPacket", std::to_string(a) + (a ? pk(p) : ""), std::to_string(b) + (b ? pk(*q) : ""));
            delete q;
            break;
        }
        case 6:
            check("getLatestPos", std::to_string(c->getLatestPos()), std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_LATEST_POS, 0)));
            check("getOldestPos", std::to_string(c->getOldestPos()), std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_OLDEST_POS, 0)));
            break;
        case 7:
        {
            uint32_t spos = rnd(2) ? (uint32_t) (rnd(n) * 100) : (uint32_t) rng();
            check("findOldestPos", std::to_string(c->findOldestPos(spos)), std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_FIND_OLDEST_POS, spos)));
            break;
        }
        case 8:
        {
            uint32_t idx = rng();
            check("getStreamPos", std::to_string(c->getStreamPos(idx)), std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_STREAM_POS, idx)));
            check("getStreamPosEnd", std::to_string(c->getStreamPosEnd(idx)), std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_STREAM_POS_END, idx)));
            break;
        }
        case 9:
            // C++ 版は writePos が 0 のとき、ロックの前に 0 を返す (Rust 版も同じ)
            check("getLatestNonContinuationPos", std::to_string(c->getLatestNonContinuationPos()),
                  std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_LATEST_NONCONT_POS, 0)));
            check("getOldestNonContinuationPos", std::to_string(c->getOldestNonContinuationPos()),
                  std::to_string(pcrs_cpb_pos(&v, PCRS_CPB_OLDEST_NONCONT_POS, 0)));
            break;
        case 10:
        {
            auto s = c->getStatistics();
            uint32_t lens[64];
            int cs = 0, ncs = 0;
            size_t k = pcrs_cpb_statistics(&v, lens, &cs, &ncs);
            std::string a = std::to_string(s.continuations) + "/" + std::to_string(s.nonContinuations);
            std::string b = std::to_string(cs) + "/" + std::to_string(ncs);
            for (auto l : s.packetLengths) a += " " + std::to_string(l);
            for (size_t j = 0; j < k; j++) b += " " + std::to_string(lens[j]);
            check("getStatistics", a, b);
            break;
        }
        case 11:
            check("willSkip/numPending", std::to_string(c->willSkip()) + " " + std::to_string(c->numPending()),
                  std::to_string(pcrs_cpb_will_skip(&v)) + " " + std::to_string((int) (m->write - m->read)));
            break;
        case 12:
            if (rnd(10) == 0)
            {
                c->init();
                pcrs_cpb_init(&v);
            }
            break;
        default:
            break;
        }
        check("state", state(*c, i + 1 == n), state(*m, i + 1 == n));
    }

    // copyFrom は reqPos を合わせて別に試す
    if (c2->writePos == 0 && m2->write == 0)
    {
        uint32_t req = rnd(2) ? 0 : (uint32_t) (rnd(n) * 100);
        uint32_t acc = rnd(2) ? ChanPacket::T_DATA : (uint32_t) rnd(20);
        c2->accept = m2->accept = acc;
        int a = c2->copyFrom(*c, req);
        int b = pcrs_cpb_copy_from(&v2, &v, req);
        check("copyFrom", std::to_string(a) + " " + state(*c2, true), std::to_string(b) + " " + state(*m2, true));
    }

    delete c; delete m; delete c2; delete m2;
}

int main(int argc, char** argv)
{
    int count = argc > 1 ? atoi(argv[1]) : 20000;
    g_sys = new MockSys();
    sys = g_sys;
    for (int i = 0; i < count; i++)
        runSeq();
    printf("比較件数 %ld、説明のつかない違い %ld\n", g_ops, g_bad);
    return g_bad ? 1 : 0;
}
