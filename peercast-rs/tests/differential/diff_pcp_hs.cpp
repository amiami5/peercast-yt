// PCP のハンドシェイクで受け取る helo / oleh の読み取り (servent.cpp の handshakeIncomingPCP、
// handshakeOutgoingPCP、pingHost と、pcp.cpp の PCPStream::readVersion) の、C++ 版と Rust 版の
// 差分テスト。
//
// C++ 版は、servent.cpp の読み取りの部分 (WITH_RUST_CORE でないほう) をこのファイルにそのまま
// 写したもの (static メソッドの途中なので外から呼べない)。Rust 版は、本番と同じ
// core/common/rustpcp.h の readIncomingHelo などを呼ぶ。どちらにも同じ入力と同じ初期値の変数を
// 与え、次のものを比べる。
//  - 読み取ったあとの変数 (rid、agent、rhost、version、pingPort など)
//  - 返り値か例外、読んだバイト数、書いたもの (quit)、ログ
//
// 入力の Stream は、ソケットと同じく足りなければ例外を投げるものと、MemoryStream と同じく 0 で
// 埋めて位置を進めないものの 2 通りで試す。後者で子の数が大きく化けると、C++ 版も Rust 版も
// データの終わりのあと子の数だけ (最大約 21 億回) 空回りするので、読み出しが 1 万回失敗したら
// 打ち切り、比べずに数える (本番のハンドシェイクはソケットから読むので起きない)。
//
// 使い方: ./diff_pcp_hs [件数 (既定 200000)]
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>
#include <vector>

#include "atom.h"
#include "pcp.h"
#include "rustpcp.h"
#include "servmgr.h"
#include "../../../tests/mockpeercast.h"

// ---------------------------------------------------------------- 記録

struct HarnessAbort {};

static std::vector<std::string> g_events;

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
static std::string esc(const std::string& s) { return esc(s.data(), s.size()); }

static void logv(const char* level, const char* fmt, va_list ap)
{
    char buf[4096];
    vsnprintf(buf, sizeof buf, fmt, ap);
    g_events.push_back(std::string(level) + esc(buf, strlen(buf)));
}
extern "C" void __wrap__Z9LOG_TRACEPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("T:", f, ap); va_end(ap); }
extern "C" void __wrap__Z9LOG_DEBUGPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("D:", f, ap); va_end(ap); }
extern "C" void __wrap__Z8LOG_INFOPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("I:", f, ap); va_end(ap); }
extern "C" void __wrap__Z8LOG_WARNPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("W:", f, ap); va_end(ap); }
extern "C" void __wrap__Z9LOG_ERRORPKcz(const char* f, ...) { va_list ap; va_start(ap, f); logv("E:", f, ap); va_end(ap); }

// 入力と出力。sock ならソケットのように、足りなければ例外を投げる。そうでなければ MemoryStream の
// ように 0 で埋めて位置を進めない。
class TestStream : public Stream
{
public:
    TestStream(const std::string& in, bool sock) : m_in(in), m_sock(sock) {}

    int read(void* p, int l) override
    {
        if (l < 0)
            throw StreamException("TestStream::read: negative length");
        if (m_pos + l <= m_in.size())
        {
            memcpy(p, m_in.data() + m_pos, l);
            m_pos += l;
            return l;
        }
        if (m_sock)
        {
            m_pos = m_in.size();
            throw EOFException("Closed on read");
        }
        if (++m_fails > 10000)
            throw HarnessAbort();
        memset(p, 0, l);
        return 0;
    }
    void write(const void* p, int l) override { m_out.append(static_cast<const char*>(p), l); }
    bool eof() override { return m_pos >= m_in.size(); }

    std::string m_in, m_out;
    size_t m_pos = 0;
    int m_fails = 0;
    bool m_sock;
};

// ---------------------------------------------------------------- C++ 版 (servent.cpp から写したもの)

static void cxxIncoming(AtomStream &atom, Host &rhost, GnuID &rid, String &agent, int& version, int& pingPort)
{
    int numc, numd;
    ID4 id = atom.read(numc, numd);

    if (id != PCP_HELO)
    {
        LOG_DEBUG("PCP incoming reply: %s", id.getString().str());
        atom.writeInt(PCP_QUIT, PCP_ERROR_QUIT+PCP_ERROR_BADRESPONSE);
        throw StreamException("Got unexpected PCP response");
    }

    char arg[64];

    ID4 osType;

    GnuID bcID;
    GnuID clientID;

    rhost.port = 0;

    for (int i=0; i<numc; i++)
    {
        int c, dlen;
        ID4 id = atom.read(c, dlen);

        if (id == PCP_HELO_AGENT)
        {
            atom.readString(arg, sizeof(arg), dlen);
            agent.set(arg);
        }else if (id == PCP_HELO_VERSION)
        {
            version = atom.readInt();
        }else if (id == PCP_HELO_SESSIONID)
        {
            atom.readBytes(rid.id, 16);
            if (rid.isSame(servMgr->sessionID))
                throw StreamException("Servent loopback");
        }else if (id == PCP_HELO_BCID)
        {
            atom.readBytes(bcID.id, 16);
        }else if (id == PCP_HELO_OSTYPE)
        {
            osType = atom.readInt();
        }else if (id == PCP_HELO_PORT)
        {
            rhost.port = atom.readShort();
        }else if (id == PCP_HELO_PING)
        {
            pingPort = atom.readShort();
        }else
        {
            LOG_DEBUG("PCP handshake skip: %s", id.getString().str());
            atom.skip(c, dlen);
        }
    }
}

static void cxxOutgoing(AtomStream &atom, GnuID &rid, String &agent, Host& thisHost, int& version, int& disable)
{
    int numc, numd;
    ID4 id = atom.read(numc, numd);
    if (id != PCP_OLEH)
    {
        LOG_DEBUG("PCP outgoing reply: %s", id.getString().str());
        atom.writeInt(PCP_QUIT, PCP_ERROR_QUIT + PCP_ERROR_BADRESPONSE);
        throw StreamException("Got unexpected PCP response");
    }

    char arg[64];

    GnuID clientID;
    rid.clear();

    // read OLEH response
    for (int i = 0; i < numc; i++)
    {
        int c, dlen;
        ID4 id = atom.read(c, dlen);

        if (id == PCP_HELO_AGENT)
        {
            atom.readString(arg, sizeof(arg), dlen);
            agent.set(arg);
        }else if (id == PCP_HELO_REMOTEIP)
        {
            thisHost.ip = atom.readAddress();
        }else if (id == PCP_HELO_PORT)
        {
            thisHost.port = atom.readShort();
        }else if (id == PCP_HELO_VERSION)
        {
            version = atom.readInt();
        }else if (id == PCP_HELO_DISABLE)
        {
            disable = atom.readInt();
        }else if (id == PCP_HELO_SESSIONID)
        {
            atom.readBytes(rid.id, 16);
            if (rid.isSame(servMgr->sessionID))
                throw StreamException("Servent loopback");
        }else
        {
            LOG_DEBUG("PCP handshake skip: %s", id.getString().str());
            atom.skip(c, dlen);
        }
    }
}

static void cxxPing(AtomStream &atom, GnuID &sid)
{
    int numc, numd;
    ID4 id = atom.read(numc, numd);
    if (id == PCP_OLEH)
    {
        for (int i=0; i<numc; i++)
        {
            int c, d;
            ID4 pid = atom.read(c, d);
            if (pid == PCP_SESSIONID)
                atom.readBytes(sid.id, 16, d);
            else
                atom.skip(c, d);
        }
    }else
    {
        LOG_DEBUG("Ping response: %s", id.getString().str());
        throw StreamException("Bad ping response");
    }
}

static void cxxVersion(Stream &in)
{
    int len = in.readInt();

    if (len != 4)
        throw StreamException("Invalid PCP");

    int ver = in.readInt();

    LOG_DEBUG("PCP ver: %d", ver);
}

// ---------------------------------------------------------------- 比べる

struct Vars
{
    Host rhost, thisHost;
    GnuID rid, sid;
    String agent;
    int version, pingPort, disable;
};

static std::string show(const Vars& v)
{
    char b[256];
    snprintf(b, sizeof b, "rhost=%s thisHost=%s version=%d ping=%d disable=%d", v.rhost.str().c_str(), v.thisHost.str().c_str(),
             v.version, v.pingPort, v.disable);
    return std::string(b) + " rid=" + v.rid.str() + " sid=" + v.sid.str() + " agent=" + esc(v.agent.c_str(), strlen(v.agent.c_str()));
}

static std::string run(bool rust, int kind, const std::string& input, bool sock, const Vars& init)
{
    g_events.clear();
    TestStream s(input, sock);
    AtomStream atom(s);
    Vars v = init;
    std::string res;
    try
    {
        switch (kind)
        {
        case 0:
            if (rust) rustbridge::readIncomingHelo(atom, v.rhost, v.rid, v.agent, v.version, v.pingPort);
            else cxxIncoming(atom, v.rhost, v.rid, v.agent, v.version, v.pingPort);
            break;
        case 1:
            if (rust) rustbridge::readOutgoingOleh(atom, v.rid, v.agent, v.thisHost, v.version, v.disable);
            else cxxOutgoing(atom, v.rid, v.agent, v.thisHost, v.version, v.disable);
            break;
        case 2:
            if (rust) rustbridge::readPingOleh(atom, v.sid);
            else cxxPing(atom, v.sid);
            break;
        default:
            if (rust) LOG_DEBUG("PCP ver: %d", rustbridge::readPcpVersion(s));
            else cxxVersion(s);
        }
        res = "ok\n";
    }catch (HarnessAbort&)
    {
        return "";
    }catch (EOFException& e)
    {
        res = "EOFException " + std::string(e.msg) + "\n";
    }catch (StreamException& e)
    {
        res = "StreamException " + std::string(e.msg) + "\n";
    }
    res += show(v) + "\n";
    res += "read=" + std::to_string(s.m_pos) + " wrote=" + esc(s.m_out) + "\n";
    for (auto& e : g_events)
        res += e + "\n";
    return res;
}

// ---------------------------------------------------------------- 入力を作る

static std::mt19937 rng(20260924);
static int rnd(int n) { return n <= 0 ? 0 : std::uniform_int_distribution<int>(0, n - 1)(rng); }
static const unsigned char SID = 0x5d;

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
static std::string bytes(int n)
{
    std::string s(n, '\0');
    for (auto& c : s)
        c = (char) rnd(256);
    return s;
}
static std::string leaf(const char* id, const std::string& data) { return id4(id) + le32(data.size()) + data; }
static std::string parent(const char* id, const std::vector<std::string>& kids, int extra = 0)
{
    std::string s = id4(id) + le32((kids.size() + extra) | 0x80000000u);
    for (auto& k : kids)
        s += k;
    return s;
}

static std::string strv()
{
    switch (rnd(9))
    {
    case 0: return "";
    case 1: return std::string("PeerCast/0.1218\0", 16);
    case 2: return std::string("PeerCastStation/2.9.0", 21);
    case 3: return std::string(63, 'a') + std::string(1, '\0');
    case 4: return std::string(64, 'b');
    case 5: return std::string(65, 'c');
    case 6: return std::string("ab\0cd", 5);
    default: return bytes(rnd(20));
    }
}
static std::string sized(int n)
{
    if (rnd(20) == 0)
        return bytes(rnd(2) ? n - 1 : n + 1);
    return bytes(n);
}
static std::string sidv()
{
    std::string s = rnd(4) == 0 ? std::string(16, (char) SID) : bytes(16);
    if (rnd(15) == 0)
        s = bytes(rnd(20));
    return s;
}

static std::string genHello(int kind)
{
    static const char* ids[] = { "agnt", "ver", "sid", "bcid", "ostp", "port", "ping", "rip", "dis", "xyz" };
    std::vector<std::string> k;
    int n = rnd(9);
    for (int i = 0; i < n; i++)
    {
        const char* id = ids[rnd(10)];
        std::string v;
        if (!strcmp(id, "agnt")) v = strv();
        else if (!strcmp(id, "ver") || !strcmp(id, "ostp") || !strcmp(id, "dis")) v = sized(4);
        else if (!strcmp(id, "sid") || !strcmp(id, "bcid")) v = sidv();
        else if (!strcmp(id, "port") || !strcmp(id, "ping")) v = sized(2);
        else if (!strcmp(id, "rip")) v = rnd(3) ? bytes(4) : bytes(rnd(2) ? 16 : 7);
        else v = bytes(rnd(10));
        if (rnd(15) == 0)
            k.push_back(parent(id, { leaf("a", "b") }));
        else
            k.push_back(leaf(id, v));
    }
    const char* head = kind == 0 ? "helo" : "oleh";
    if (rnd(12) == 0)
        head = rnd(2) ? (kind == 0 ? "oleh" : "helo") : "quit";
    int extra = rnd(10) == 0 ? 1 + rnd(3) : 0;
    std::string s = parent(head, k, extra);
    if (rnd(8) == 0)
        s = s.substr(0, rnd(s.size() + 1));     // 途中で切れる
    if (rnd(6) == 0 && !s.empty())
        s[rnd(s.size())] = (char) rnd(256);      // 変異
    return s + bytes(rnd(3) ? 0 : rnd(20));
}

static std::string genVersion()
{
    std::string s = le32(rnd(4) ? 4 : rnd(8)) + le32(rng());
    if (rnd(5) == 0)
        s = s.substr(0, rnd(s.size() + 1));
    return s;
}

int main(int argc, char** argv)
{
    int count = argc > 1 ? atoi(argv[1]) : 200000;

    peercastApp = new MockPeercastApplication();
    peercastInst = new MockPeercastInstance();
    peercastInst->init();
    memset(servMgr->sessionID.id, SID, 16);

    long same = 0, bad = 0, ok = 0, aborted = 0;
    for (int i = 0; i < count; i++)
    {
        int kind = rnd(4);
        std::string input = kind == 3 ? genVersion() : genHello(kind);
        bool sock = rnd(2);
        Vars init;
        init.rhost = Host(rng(), 7144);
        init.thisHost = Host(0, 0);
        memset(init.rid.id, 0x11, 16);
        memset(init.sid.id, 0x22, 16);
        init.agent.set(rnd(2) ? "old agent" : "");
        init.version = 5;
        init.pingPort = 6;
        init.disable = 7;

        std::string a = run(false, kind, input, sock, init);
        std::string b = run(true, kind, input, sock, init);
        if (a.empty() && b.empty())
        {
            aborted++;
            continue;
        }
        if (a == b)
        {
            same++;
            if (a.rfind("ok\n", 0) == 0)
                ok++;
            continue;
        }
        if (++bad <= 5)
            printf("---- 違い (kind %d, sock %d)\nC++:\n%sRust:\n%s\n", kind, sock, a.c_str(), b.c_str());
    }
    printf("件数 %d: 同じ %ld (うち最後まで読めたもの %ld)\n", count, same, ok);
    printf("  データの終わりで空回りしたので打ち切ったもの (比べない) %ld\n", aborted);
    printf("説明のつかない違い %ld\n", bad);
    return bad ? 1 : 0;
}
