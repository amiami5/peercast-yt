// PCP の受け取ったパケットの処理 (peercast-rs の src/pcp) を PCPStream から使う。
// WITH_RUST_CORE のビルドで pcp.cpp から使うほか、peercast-rs の差分テストからも使う。
//
// Rust は atom の解析と判断をし、チャンネルやサーバーの状態 (chanMgr、servMgr、Channel) は
// pcrs_pcp_host のコールバックを通してだけ触る。コールバックの中身は、C++ 版の pcp.cpp と
// chaninfo.cpp の同じ箇所をそのまま写したもの。
#ifndef _RUSTPCP_H
#define _RUSTPCP_H

#include <cstdint>
#include <cstring>
#include <exception>
#include <memory>
#include <new>
#include <string>

#include "chanmgr.h"
#include "channel.h"
#include "pcp.h"
#include "peercast.h"
#include "rustbridge.h"
#include "servent.h"
#include "servmgr.h"
#include "str.h"
#include "version2.h"
#include "xml.h"

namespace rustbridge
{

// atom のアドレス (readAddress の値)
inline IP pcpIP(const pcrs_pcp_ip& a)
{
    if (a.kind == 4)
        return IP(a.v4);
    in6_addr addr;
    memcpy(addr.s6_addr, a.v6, 16);
    return IP(addr);
}

// 1 回の procAtom の間、PCPStream を Rust に貸す。コールバックの中で起きた例外は保存しておき、
// Rust から戻ったあとで投げ直す。
class PcpHost
{
public:
    explicit PcpHost(PCPStream& pcp)
        : m_pcp(pcp)
    {
        m_host.ctx = this;
        m_host.session_id = [](void*, uint8_t* out) { memcpy(out, servMgr->sessionID.id, 16); };
        m_host.is_root = [](void*) -> bool { return servMgr->isRoot; };
        m_host.time = [](void*) -> uint32_t { return sys->getTime(); };
        m_host.log = [](void*, int level, const uint8_t* msg, size_t len) {
            std::string s(reinterpret_cast<const char*>(msg), len);
            if (level == 0)
                LOG_DEBUG("%s", s.c_str());
            else if (level == 1)
                LOG_INFO("%s", s.c_str());
            else
                LOG_ERROR("%s", s.c_str());
        };
        m_host.event = [](void* c, int op, int32_t arg, const uint8_t* data, size_t len) {
            return self(c)->guard([&]() { self(c)->event(op, arg, data, len); });
        };
        m_host.hit = [](void* c, const pcrs_pcp_hit* h, bool add) {
            return self(c)->guard([&]() { hit(*h, add); });
        };
        m_host.push = [](void* c, const pcrs_pcp_ip* ip, bool portSet, int32_t port, const uint8_t* chanID) {
            return self(c)->guard([&]() { push(*ip, portSet, port, chanID); });
        };
        m_host.chan_has_channel = [](void* c) -> bool { return self(c)->m_ch != nullptr; };
        m_host.chan_packet = [](void* c, int32_t kind, uint32_t pos, bool cont, const uint8_t* data, size_t len) {
            return self(c)->guard([&]() { self(c)->chanPacket(kind, pos, cont, data, len); });
        };
        m_host.broadcast = [](void* c, int target, const uint8_t* pack, size_t len, const uint8_t* chanID, const uint8_t* destID) {
            return self(c)->guard([&]() { self(c)->broadcast(target, pack, len, chanID, destID); });
        };
    }

    // パケットのバッファ data (len バイト) を処理して procAtom の値を返す。bcs と
    // PCPStream::nextRootPacket を書き換える。
    int procPacket(char* data, size_t len, BroadcastState& bcs)
    {
        pcrs_pcp_state st;
        memcpy(st.chan_id, bcs.chanID.id, 16);
        memcpy(st.bc_id, bcs.bcID.id, 16);
        st.num_hops = bcs.numHops;
        st.for_me = bcs.forMe;
        st.stream_pos = bcs.streamPos;
        st.group = bcs.group;
        st.next_root_packet = m_pcp.nextRootPacket;

        int32_t result = 0;
        RustBuf err;
        int r = pcrs_pcp_proc_packet(&m_host, reinterpret_cast<uint8_t*>(data), len, &st, &result, err.out());

        memcpy(bcs.chanID.id, st.chan_id, 16);
        memcpy(bcs.bcID.id, st.bc_id, 16);
        bcs.numHops = st.num_hops;
        bcs.forMe = st.for_me;
        bcs.streamPos = st.stream_pos;
        bcs.group = st.group;
        m_pcp.nextRootPacket = st.next_root_packet;

        if (r == 1)
            std::rethrow_exception(m_ex);
        if (r == 2)
            throw StreamException(err.str());
        return result;
    }

private:
    static PcpHost* self(void* c) { return static_cast<PcpHost*>(c); }

    template <typename F>
    int guard(F f)
    {
        try
        {
            f();
            return 0;
        }catch (...)
        {
            m_ex = std::current_exception();
            return -1;
        }
    }

    static GnuID gnuid(const uint8_t* p)
    {
        GnuID id;
        memcpy(id.id, p, 16);
        return id;
    }

    static IP ip(const pcrs_pcp_ip& a) { return pcpIP(a); }

    // AtomStream::readString で String に書いたとき (残りは前の中身のまま)
    static void readString(::String& s, const uint8_t* data, size_t len)
    {
        memcpy(s.data, data, len);
        s.data[sizeof(s.data) - 1] = 0;
    }

    ::String* infoField(int field)
    {
        switch (field)
        {
        case PCRS_PCP_INFO_NAME:        return &m_newInfo.name;
        case PCRS_PCP_INFO_GENRE:       return &m_newInfo.genre;
        case PCRS_PCP_INFO_URL:         return &m_newInfo.url;
        case PCRS_PCP_INFO_DESC:        return &m_newInfo.desc;
        case PCRS_PCP_INFO_COMMENT:     return &m_newInfo.comment;
        case PCRS_PCP_INFO_TYPE:        return &m_newInfo.contentType;
        case PCRS_PCP_INFO_STREAMTYPE:  return &m_newInfo.MIMEType;
        case PCRS_PCP_INFO_STREAMEXT:   return &m_newInfo.streamExt;
        case PCRS_PCP_TRACK_TITLE:      return &m_newInfo.track.title;
        case PCRS_PCP_TRACK_CREATOR:    return &m_newInfo.track.artist;
        case PCRS_PCP_TRACK_URL:        return &m_newInfo.track.contact;
        case PCRS_PCP_TRACK_ALBUM:      return &m_newInfo.track.album;
        default: throw GeneralException("PcpHost: bad info field");
        }
    }

    void event(int op, int32_t arg, const uint8_t* data, size_t len)
    {
        switch (op)
        {
        case PCRS_PCP_EV_ROUTE_ADD:
            m_pcp.routeList.add(gnuid(data));
            break;
        case PCRS_PCP_EV_UPDATE_INTERVAL:
            chanMgr->setUpdateInterval(arg);
            break;
        case PCRS_PCP_EV_UPGRADE:
        {
            std::string url(reinterpret_cast<const char*>(data), len);
            Sys::strcpy_truncate(servMgr->downloadURL, sizeof(servMgr->downloadURL), url.c_str());
            peercast::notifyMessage(ServMgr::NT_UPGRADE, "There is a new version of PeerCast available, please click here to upgrade your client.");
            break;
        }
        case PCRS_PCP_EV_TRACKER_UPDATE:
            chanMgr->broadcastTrackerUpdate(m_pcp.remoteID, true);
            break;
        case PCRS_PCP_EV_ROOT_MESSAGE:
        {
            ::String newMsg;
            memcpy(newMsg.data, data, len);
            newMsg.data[len] = 0;
            if (!newMsg.isSame(servMgr->rootMsg.cstr()))
            {
                servMgr->rootMsg = newMsg;
                LOG_DEBUG("PCP got new root mesg: %s", servMgr->rootMsg.cstr());
                if (servMgr->rootMsg != "")
                    peercast::notifyMessage(ServMgr::NT_PEERCAST,
                                            (std::string(servMgr->rootHost.str()) + "「" + servMgr->rootMsg.cstr() + "」").c_str());
            }
            break;
        }
        case PCRS_PCP_EV_CHAN_BEGIN:
        {
            GnuID chanID = gnuid(data);
            resetNewInfo();
            m_ch = chanMgr->findChannelByID(chanID);
            m_chl = chanMgr->findHitListByID(chanID);
            if (m_ch)
                m_newInfo = m_ch->info;
            else if (m_chl)
                m_newInfo = m_chl->info;
            break;
        }
        case PCRS_PCP_EV_CHAN_INFO_STRING:
        {
            ::String* s = infoField(arg);
            readString(*s, data, len);
            // 他のノードから届いた URL は、UI でリンクとして表示される。
            // "javascript:" などが入り込まないよう、http(s) のみ許可する。
            if ((arg == PCRS_PCP_INFO_URL || arg == PCRS_PCP_TRACK_URL) && !str::is_http_url(s->cstr()))
                s->clear();
            break;
        }
        case PCRS_PCP_EV_CHAN_INFO_BITRATE:
            m_newInfo.bitrate = arg;
            break;
        case PCRS_PCP_EV_CHAN_BCID:
            memcpy(m_newInfo.bcID.id, data, 16);
            break;
        case PCRS_PCP_EV_CHAN_ID:
            memcpy(m_newInfo.id.id, data, 16);
            m_ch = chanMgr->findChannelByID(m_newInfo.id);
            m_chl = chanMgr->findHitListByID(m_newInfo.id);
            break;
        case PCRS_PCP_EV_CHAN_END:
            chanEnd();
            break;
        default:
            throw GeneralException("PcpHost: bad event");
        }
    }

    // C++ 版の newInfo は readChanAtoms のスタックの変数で、String のまだ書いていない部分 (NUL の
    // 後ろ) は初期化されていなかった。NUL で終わらない文字列の atom を読むとその続きが文字列に
    // 入り、ほかのノードへも中継されていた。Rust 版はそこを 0 とするので、0 で埋めた記憶領域の上に
    // 作り直す (String の代入は strcpy なので、ch->info などを写したあとも NUL の後ろは 0 のまま)。
    void resetNewInfo()
    {
        m_newInfo.~ChanInfo();
        memset(static_cast<void*>(&m_newInfo), 0, sizeof(ChanInfo));
        new (&m_newInfo) ChanInfo();
    }

    // readChanAtoms の終わり
    void chanEnd()
    {
        auto& chl = m_chl;
        auto& ch = m_ch;
        ChanInfo& newInfo = m_newInfo;

        if (!chl)
            chl = chanMgr->addHitList(newInfo);

        if (chl)
        {
            chl->info.update(newInfo);

            if (!servMgr->chanLog.isEmpty())
            {
                try
                {
                    FileStream file;
                    file.openWriteAppend(servMgr->chanLog.cstr());
                    XML::Node *rn = new XML::Node("update time=\"%u\"", sys->getTime());
                    XML::Node *n = chl->info.createChannelXML();
                    n->add(chl->createXML(false));
                    n->add(chl->info.createTrackXML());
                    rn->add(n);
                    rn->write(file, 0);
                    delete rn;
                    file.close();
                }catch (StreamException &e)
                {
                    LOG_ERROR("Unable to update channel log: %s", e.msg);
                }
            }
        }

        if (ch && !ch->isBroadcasting())
            ch->updateInfo(newInfo);
    }

    // readHostAtoms の、値を読んだあと
    static void hit(const pcrs_pcp_hit& h, bool add)
    {
        ChanHit hit;
        hit.init();

        for (int i = 0; i < 2; i++)
        {
            if (h.rhost_ip[i].kind)
                hit.rhost[i].ip = ip(h.rhost_ip[i]);
            if (h.rhost_port_set[i])
                hit.rhost[i].port = h.rhost_port[i];
        }
        if (h.set & PCRS_PCP_HIT_NUML)
            hit.numListeners = h.num_listeners;
        if (h.set & PCRS_PCP_HIT_NUMR)
            hit.numRelays = h.num_relays;
        if (h.set & PCRS_PCP_HIT_UPTIME)
            hit.upTime = h.up_time;
        if (h.set & PCRS_PCP_HIT_OLDPOS)
            hit.oldestPos = h.oldest_pos;
        if (h.set & PCRS_PCP_HIT_NEWPOS)
            hit.newestPos = h.newest_pos;
        if (h.set & PCRS_PCP_HIT_VERSION)
            hit.version = h.version;
        if (h.set & PCRS_PCP_HIT_VERSION_VP)
            hit.versionVP = h.version_vp;
        if (h.set & PCRS_PCP_HIT_VEX_PREFIX)
            memcpy(hit.versionExPrefix, h.version_ex_prefix, 2);
        if (h.set & PCRS_PCP_HIT_VEX_NUMBER)
            hit.versionExNumber = h.version_ex_number;
        if (h.set & PCRS_PCP_HIT_FLAGS1)
        {
            int fl1 = h.flags1;

            hit.recv = (fl1 & PCP_HOST_FLAGS1_RECV) !=0;
            hit.relay = (fl1 & PCP_HOST_FLAGS1_RELAY) !=0;
            hit.direct = (fl1 & PCP_HOST_FLAGS1_DIRECT) !=0;
            hit.cin = (fl1 & PCP_HOST_FLAGS1_CIN) !=0;
            hit.tracker = (fl1 & PCP_HOST_FLAGS1_TRACKER) !=0;
            hit.firewalled = (fl1 & PCP_HOST_FLAGS1_PUSH) !=0;
        }
        if (h.set & PCRS_PCP_HIT_SESSION_ID)
            memcpy(hit.sessionID.id, h.session_id, 16);
        if (h.uphost_ip.kind)
            hit.uphost.ip = ip(h.uphost_ip);
        if (h.set & PCRS_PCP_HIT_UPHOST_PORT)
            hit.uphost.port = h.uphost_port;
        if (h.set & PCRS_PCP_HIT_UPHOST_HOPS)
            hit.uphostHops = h.uphost_hops;

        hit.host = hit.rhost[0];
        hit.chanID = gnuid(h.chan_id);

        hit.numHops = h.num_hops;

        if (add)
            chanMgr->addHit(hit);
        else
            chanMgr->delHit(hit);
    }

    // readPushAtoms の、自分宛てのとき
    static void push(const pcrs_pcp_ip& ipv, bool portSet, int32_t port, const uint8_t* chanIDp)
    {
        Host host;
        if (ipv.kind)
            host.ip = ip(ipv);
        if (portSet)
            host.port = port;
        GnuID chanID = gnuid(chanIDp);

        Servent *s = nullptr;

        if (chanID.isSet())
        {
            auto ch = chanMgr->findChannelByID(chanID);
            if (ch)
                if (ch->isBroadcasting() || (!ch->isFull() && !servMgr->relaysFull() && ch->info.id.isSame(chanID)))
                    s = servMgr->allocServent();
        }else{
            s = servMgr->allocServent();
        }

        if (s)
        {
            LOG_DEBUG("GIVing to %s", host.str().c_str());
            s->initGIV(host, chanID);
        }
    }

    // readPktAtoms の、値を読んだあとのチャンネル側
    void chanPacket(int32_t kind, uint32_t pos, bool cont, const uint8_t* data, size_t len)
    {
        ChanPacket pack;
        pack.type = static_cast<ChanPacket::TYPE>(kind);
        pack.pos = pos;
        pack.cont = cont;
        if (len > ChanPacket::MAX_DATALEN)
            throw StreamException("Data size too large");
        pack.len = len;
        memcpy(pack.data, data, len);

        auto& ch = m_ch;
        std::lock_guard<std::recursive_mutex> cs(ch->lock);

        // stream positions (= byte offsets) are unsigned ints
        std::int64_t diff = (std::int64_t) pack.pos - ch->streamPos;
        if (diff) {
            std::string sdiff = std::to_string(diff);
            if (sdiff[0] != '-') {
                sdiff = "+" + sdiff;
            }
            LOG_DEBUG("PCP skipping %s (%u -> %u)", sdiff.c_str(), ch->streamPos, pack.pos);
        }

        if (pack.type == ChanPacket::T_HEAD)
        {
            LOG_DEBUG("New head packet at %u", pack.pos);

            // check for stream restart
            if (pack.pos == 0)
            {
                LOG_INFO("PCP resetting stream");
                ch->streamIndex++;
                ch->rawData.init();
            }

            ch->headPack = pack;

            ch->rawData.writePacket(pack, true);
            ch->streamPos = pack.pos+pack.len;
        }else if (pack.type == ChanPacket::T_DATA)
        {
            ch->rawData.writePacket(pack, true);
            ch->streamPos = pack.pos+pack.len;
        }
    }

    // readBroadcastAtoms の、中継するところ
    void broadcast(int target, const uint8_t* data, size_t len, const uint8_t* chanIDp, const uint8_t* destIDp)
    {
        ChanPacket pack;
        if (len > ChanPacket::MAX_DATALEN)
            throw StreamException("Packet data too large");
        memcpy(pack.data, data, len);
        pack.len = len;
        pack.type = ChanPacket::T_PCP;
        GnuID chanID = gnuid(chanIDp), destID = gnuid(destIDp);

        switch (target)
        {
        case PCRS_PCP_BCAST_UP:
            chanMgr->broadcastPacketUp(pack, chanID, m_pcp.remoteID, destID);
            break;
        case PCRS_PCP_BCAST_COUT:
            servMgr->broadcastPacket(pack, chanID, m_pcp.remoteID, destID, Servent::T_COUT);
            break;
        case PCRS_PCP_BCAST_CIN:
            servMgr->broadcastPacket(pack, chanID, m_pcp.remoteID, destID, Servent::T_CIN);
            break;
        case PCRS_PCP_BCAST_RELAY:
            servMgr->broadcastPacket(pack, chanID, m_pcp.remoteID, destID, Servent::T_RELAY);
            break;
        default:
            throw GeneralException("PcpHost: bad broadcast target");
        }
    }

    PCPStream& m_pcp;
    pcrs_pcp_host m_host;
    std::exception_ptr m_ex;

    // readChanAtoms の間の ch、chl、newInfo
    std::shared_ptr<Channel> m_ch;
    std::shared_ptr<ChanHitList> m_chl;
    ChanInfo m_newInfo;
};

// ハンドシェイクで受け取った helo / oleh (peercast-rs の src/pcp/handshake.rs)
struct PcpHello : pcrs_pcp_hello
{
    PcpHello() { memset(static_cast<pcrs_pcp_hello*>(this), 0, sizeof(pcrs_pcp_hello)); }

    bool has(uint32_t bit) const { return (set & bit) != 0; }
    std::string agentStr() const { return std::string(reinterpret_cast<const char*>(agent), agent_len); }
    void sessionID(GnuID& id) const { memcpy(id.id, session_id, 16); }
    ID4 unexpectedID() const
    {
        ID4 id;
        memcpy(id.getData(), unexpected, 4);
        return id;
    }
};

// readHello のエラー。呼んだ側が読んだ値を使ってから raise() で投げる。
struct PcpHelloError
{
    std::exception_ptr ex;      // 読み出しの例外 (投げられた例外そのもの)
    bool stream = false;        // Rust の StreamException
    std::string msg;

    // GeneralException はコピーすると msg が古い msgbuf を指すので、StreamException はここで作って投げる
    void raise() const
    {
        if (ex)
            std::rethrow_exception(ex);
        if (stream)
            throw StreamException(msg);
    }
};

// in から helo / oleh を読む (kind は PCRS_PCP_KIND_*)。C++ 版ではエラーの前に読んだ値が呼んだ側の
// 変数に入っているので、エラーは投げずに返し、呼んだ側が値を使ってから投げる。
inline PcpHelloError readHello(Stream& in, int kind, PcpHello& h)
{
    PcpHelloError e;
    StreamReader reader(in);
    RustBuf err;
    auto log = [](void*, const uint8_t* msg, size_t len) {
        LOG_DEBUG("%s", std::string(reinterpret_cast<const char*>(msg), len).c_str());
    };
    int r = pcrs_pcp_read_hello(reader.get(), kind, servMgr->sessionID.id, nullptr, log, &h, err.out());
    if (r == 1)
    {
        try
        {
            reader.rethrowIfAborted();
        }catch (...)
        {
            e.ex = std::current_exception();
        }
    }
    if (r == 2)
    {
        e.stream = true;
        e.msg = err.str();
    }
    return e;
}

// PCPStream::readVersion の、長さと版を読むところ。版を返す。
inline int readPcpVersion(Stream& in)
{
    StreamReader reader(in);
    int32_t ver = 0;
    RustBuf err;
    int r = pcrs_pcp_read_version(reader.get(), &ver, err.out());
    if (r == 1)
        reader.rethrowIfAborted();
    if (r == 2)
        throw StreamException(err.str());
    return ver;
}

// Servent::handshakeIncomingPCP の、相手の helo を読むところ。読んだ値は C++ 版と同じ変数に入れる
// (エラーのときも、それまでに読んだ値を入れてから投げる)。
inline void readIncomingHelo(AtomStream& atom, Host& rhost, GnuID& rid, String& agent, int& version, int& pingPort)
{
    PcpHello h;
    PcpHelloError ex = readHello(atom.io, PCRS_PCP_KIND_HELO, h);
    if (h.is_unexpected)
    {
        LOG_DEBUG("PCP incoming reply: %s", h.unexpectedID().getString().str());
        atom.writeInt(PCP_QUIT, PCP_ERROR_QUIT+PCP_ERROR_BADRESPONSE);
        throw StreamException("Got unexpected PCP response");
    }
    if (h.header_ok)
    {
        rhost.port = 0;
        if (h.has_agent)
            agent.set(h.agentStr().c_str());
        if (h.has(PCRS_PCP_HELLO_VERSION))
            version = h.version;
        if (h.has(PCRS_PCP_HELLO_SESSION_ID))
            h.sessionID(rid);
        if (h.has(PCRS_PCP_HELLO_PORT))
            rhost.port = h.port;
        if (h.has(PCRS_PCP_HELLO_PING))
            pingPort = h.ping;
    }
    ex.raise();
}

// Servent::handshakeOutgoingPCP の、相手の oleh を読むところ
inline void readOutgoingOleh(AtomStream& atom, GnuID& rid, String& agent, Host& thisHost, int& version, int& disable)
{
    PcpHello h;
    PcpHelloError ex = readHello(atom.io, PCRS_PCP_KIND_OLEH, h);
    if (h.is_unexpected)
    {
        LOG_DEBUG("PCP outgoing reply: %s", h.unexpectedID().getString().str());
        atom.writeInt(PCP_QUIT, PCP_ERROR_QUIT + PCP_ERROR_BADRESPONSE);
        throw StreamException("Got unexpected PCP response");
    }
    if (h.header_ok)
    {
        rid.clear();
        if (h.has_agent)
            agent.set(h.agentStr().c_str());
        if (h.remote_ip.kind)
            thisHost.ip = pcpIP(h.remote_ip);
        if (h.has(PCRS_PCP_HELLO_PORT))
            thisHost.port = h.port;
        if (h.has(PCRS_PCP_HELLO_VERSION))
            version = h.version;
        if (h.has(PCRS_PCP_HELLO_DISABLE))
            disable = h.disable;
        if (h.has(PCRS_PCP_HELLO_SESSION_ID))
            h.sessionID(rid);
    }
    ex.raise();
}

// Servent::pingHost の、相手の oleh を読むところ
inline void readPingOleh(AtomStream& atom, GnuID& sid)
{
    PcpHello h;
    memcpy(h.session_id, sid.id, 16);
    PcpHelloError ex = readHello(atom.io, PCRS_PCP_KIND_PING, h);
    if (h.has(PCRS_PCP_HELLO_SESSION_ID))
        h.sessionID(sid);
    ex.raise();
    if (h.is_unexpected)
    {
        LOG_DEBUG("Ping response: %s", h.unexpectedID().getString().str());
        throw StreamException("Bad ping response");
    }
}

} // namespace rustbridge

#endif
