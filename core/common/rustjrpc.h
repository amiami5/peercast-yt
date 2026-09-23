// JSON-RPC の API (peercast-rs の src/jrpc.rs) を JrpcApi から使う。WITH_RUST_CORE のビルドで
// jrpc.cpp から使うほか、peercast-rs の差分テストからも使う。
//
// Rust は要求の解釈と結果の JSON の組み立てを行い、サーバーの状態 (chanMgr、servMgr、ログなど) は
// pcrs_jrpc_host の call を通してだけ触る。call の中身は、C++ 版の jrpc.cpp の同じ箇所をそのまま
// 写したもの。
#ifndef _RUSTJRPC_H
#define _RUSTJRPC_H

#include <cstring>
#include <exception>
#include <stdexcept>
#include <string>
#include <vector>

#include "chandir.h"
#include "chanmgr.h"
#include "channel.h"
#include "json.hpp"
#include "logbuf.h"
#include "notif.h"
#include "peercast.h"
#include "peercast_rs.h"
#include "rustbridge.h"
#include "rustchan.h"
#include "servent.h"
#include "servmgr.h"
#include "stats.h"
#include "str.h"
#include "stream.h"
#include "version2.h"
#include "yplist.h"

namespace rustbridge
{

// pcrs_jrpc_host の実装。1 回の呼び出しの間だけ使う。
class JrpcHost
{
public:
    JrpcHost()
    {
        m_host.ctx = this;
        m_host.log = log;
        m_host.call = call;
    }

    const pcrs_jrpc_host* get() const { return &m_host; }

private:
    static pcrs_bytes b(const std::string& s)
    {
        return { reinterpret_cast<const uint8_t*>(s.data()), s.size() };
    }

    static std::string s(const pcrs_bytes& v)
    {
        return std::string(reinterpret_cast<const char*>(v.ptr), v.len);
    }

    static void putBytes(pcrs_jrpc_sink* out, const std::string& v)
    {
        pcrs_jrpc_put_bytes(out, reinterpret_cast<const uint8_t*>(v.data()), v.size());
    }

    static void log(void*, int level, const uint8_t* msg, size_t len)
    {
        std::string m(reinterpret_cast<const char*>(msg), len);
        switch (level)
        {
        case 0: LOG_DEBUG("%s", m.c_str()); break;
        case 1: LOG_INFO("%s", m.c_str()); break;
        case 2: LOG_WARN("%s", m.c_str()); break;
        default: LOG_ERROR("%s", m.c_str()); break;
        }
    }

    static int call(void*, int op, const pcrs_jrpc_args* args, pcrs_jrpc_sink* out)
    {
        try {
            dispatch(op, *args, out);
            return 0;
        } catch (std::domain_error& e) {
            pcrs_jrpc_put_error(out, reinterpret_cast<const uint8_t*>(e.what()), strlen(e.what()));
            return 2;
        } catch (std::exception& e) {
            pcrs_jrpc_put_error(out, reinterpret_cast<const uint8_t*>(e.what()), strlen(e.what()));
            return 1;
        }
    }

    // 渡す値は、この関数の間だけ生きていればよい (Rust は put のたびに写す)
    static void putChannel(pcrs_jrpc_sink* out, std::shared_ptr<Channel> c)
    {
        std::string sourceHost = c->sourceHost.host.str(true);
        std::string sockHost = c->sock ? (std::string) c->sock->host : "";
        pcrs_jrpc_channel v = {};
        v.info = rsInfo(c->info);
        v.status = c->status;
        v.source_url = rsBytes(c->sourceURL.c_str());
        v.source_host = b(sourceHost);
        v.uptime = c->info.getUptime();
        v.local_relays = c->localRelays();
        v.local_directs = c->localListeners();
        v.total_relays = c->totalRelays();
        v.total_directs = c->totalListeners();
        v.is_broadcasting = c->isBroadcasting();
        v.is_full = c->isFull();
        v.is_receiving = c->isReceiving();
        v.ip_version = c->ipVersion;
        v.has_sock = c->sock != nullptr;
        v.sock_host = b(sockHost);
        v.source_rate = c->sourceData ? c->sourceData->getSourceRate() : 0;
        v.src_protocol = c->info.srcProtocol;
        v.stream_pos = c->streamPos;
        pcrs_jrpc_put_channel(out, &v);
    }

    static void dispatch(int op, const pcrs_jrpc_args& a, pcrs_jrpc_sink* out)
    {
        GnuID id;
        memcpy(id.id, a.id, 16);

        switch (op)
        {
        case PCRS_JRPC_AGENT:
            putBytes(out, PCX_AGENT);
            break;
        case PCRS_JRPC_LOG_LINES:
        {
            std::vector<std::string> lines =
                sys->logBuf->toLines([](unsigned int time, LogBuffer::TYPE type, const char* line)
                                     {
                                         std::string buf;

                                         if (type != LogBuffer::T_NONE)
                                         {
                                             buf += str::rstrip(String().setFromTime(time));
                                             buf += " [";
                                             buf += LogBuffer::getTypeStr(type);
                                             buf += "] ";
                                         }

                                         buf += line;

                                         return buf;
                                     });
            for (auto& l : lines)
                putBytes(out, l);
            break;
        }
        case PCRS_JRPC_CLEAR_LOG:
            sys->logBuf->clear();
            break;
        case PCRS_JRPC_LOG_LEVEL:
            pcrs_jrpc_put_int(out, servMgr->logLevel());
            break;
        case PCRS_JRPC_SET_LOG_LEVEL:
            servMgr->logLevel(a.i);
            break;
        case PCRS_JRPC_FETCH:
        {
            std::string url = s(a.fields[0]);
            std::string typeStr = s(a.fields[5]);

            ChanInfo info;
            info.name    = s(a.fields[1]);
            info.desc    = s(a.fields[2]);
            info.genre   = s(a.fields[3]);
            info.url     = s(a.fields[4]);
            info.bitrate = a.i;
            info.setContentType(typeStr.c_str());

            // ソースに接続できなかった場合もチャンネルを同定したいの
            // で、事前にチャンネルIDを設定する。
            Servent::setBroadcastIdChannelId(info, chanMgr->broadcastID);

            auto c = chanMgr->createChannel(info);
            if (!c)
                break;
            if (a.j) {
                c->ipVersion = Channel::IP_V6;
                servMgr->checkFirewallIPv6();
            }
            c->startURL(url.c_str());

            GnuID cid = c->getID();
            pcrs_jrpc_put_bytes(out, cid.id, 16);
            break;
        }
        case PCRS_JRPC_CHANNELS:
        {
            std::lock_guard<std::recursive_mutex> cs(chanMgr->lock);
            for (auto c = chanMgr->channel; c != nullptr; c = c->next)
                putChannel(out, c);
            break;
        }
        case PCRS_JRPC_FIND_CHANNEL:
        {
            auto c = chanMgr->findChannelByID(id);
            if (c)
                putChannel(out, c);
            break;
        }
        case PCRS_JRPC_SERVENTS:
        {
            std::lock_guard<std::recursive_mutex> cs(servMgr->lock);
            for (Servent* sv = servMgr->servents; sv != nullptr; sv = sv->next)
            {
                if (!sv->chanID.isSame(id))
                    continue;

                std::string sockHost = sv->sock ? (std::string) sv->sock->host : "";
                pcrs_jrpc_servent v = {};
                v.index = sv->serventIndex;
                v.type = rsBytes(sv->getTypeStr());
                v.status = rsBytes(sv->getStatusStr());
                v.send_rate = sv->sock ? sv->sock->bytesOutPerSec() : 0;
                v.recv_rate = sv->sock ? sv->sock->bytesInPerSec() : 0;
                v.protocol = sv->outputProtocol;
                v.agent = rsBytes(sv->agent.cstr());
                v.has_sock = sv->sock != nullptr;
                v.sock_host = b(sockHost);
                pcrs_jrpc_put_servent(out, &v);
            }
            break;
        }
        case PCRS_JRPC_STOP_CONNECTION:
        {
            bool success = false;
            std::lock_guard<std::recursive_mutex> cs(servMgr->lock);
            for (Servent* sv = servMgr->servents; sv != nullptr; sv = sv->next)
            {
                 if (sv->serventIndex == a.i &&
                     sv->chanID.isSame(id) &&
                     sv->type == Servent::T_RELAY)
                 {
                     sv->abort();
                     success = true;
                     break;
                 }
            }
            pcrs_jrpc_put_int(out, success);
            break;
        }
        case PCRS_JRPC_RELAY_TREE:
        {
            auto ch = chanMgr->findChannelByID(id);
            if (!ch)
            {
                pcrs_jrpc_put_int(out, 0);
                break;
            }
            auto hitList = chanMgr->findHitListByID(id);
            if (!hitList)
            {
                pcrs_jrpc_put_int(out, 1);
                break;
            }
            pcrs_jrpc_put_int(out, 2);

            ChanHit self;
            Host uphost;
            bool isTracker = ch->isBroadcasting();

            if (!isTracker)
                uphost = ch->sourceHost.host;

            self.initLocal(ch->localListeners(),
                           ch->localRelays(),
                           ch->info.numSkips,
                           ch->info.getUptime(),
                           ch->isPlaying(),
                           ch->rawData.getOldestPos(),
                           ch->rawData.getLatestPos(),
                           ch->canAddRelay(),
                           uphost,
                           (ch->ipVersion == 6));
            self.tracker = isTracker;

            // HostGraph のコンストラクターと同じ順に並べる (自分、続いてリストの順)
            std::vector<ChanHit> hits = { self };
            for (auto p = hitList->hit; p; p = p->next)
            {
                LOG_DEBUG("HostGraph: %s", p->rhost[0].str().c_str());
                hits.push_back(*p);
            }
            for (auto& h : hits)
            {
                pcrs_hit v = rsHit(h);
                std::string addr = h.rhost[0].ip.str();
                pcrs_jrpc_put_hit(out, &v, reinterpret_cast<const uint8_t*>(addr.data()), addr.size());
            }
            break;
        }
        case PCRS_JRPC_BUMP:
        {
            auto channel = chanMgr->findChannelByID(id);
            if (channel)
                channel->bump = true;
            pcrs_jrpc_put_int(out, channel != nullptr);
            break;
        }
        case PCRS_JRPC_PLAY:
        {
            ChanInfo info;
            info.id = id;
            chanMgr->findAndPlayChannel(info, /*keep=*/ false);
            break;
        }
        case PCRS_JRPC_STOP_CHANNEL:
        {
            auto channel = chanMgr->findChannelByID(id);
            if (channel)
                channel->thread.shutdown();
            break;
        }
        case PCRS_JRPC_ROOT_HOST:
            putBytes(out, servMgr->rootHost.cstr());
            break;
        case PCRS_JRPC_CLEAR_ROOT_HOST:
        {
            std::lock_guard<std::recursive_mutex> cs(servMgr->lock);
            servMgr->rootHost.clear();
            break;
        }
        case PCRS_JRPC_SETTINGS:
            pcrs_jrpc_put_int(out, servMgr->maxRelays);
            pcrs_jrpc_put_int(out, chanMgr->maxRelaysPerChannel);
            pcrs_jrpc_put_int(out, servMgr->maxDirect);
            pcrs_jrpc_put_int(out, servMgr->maxBitrateOut);
            break;
        case PCRS_JRPC_SET_SETTING:
            switch (a.i)
            {
            case 0: servMgr->setMaxRelays(a.j); break;
            case 1: chanMgr->maxRelaysPerChannel = a.j; break;
            case 2: servMgr->maxDirect = a.j; break;
            default: servMgr->maxBitrateOut = a.j; break;
            }
            break;
        case PCRS_JRPC_STATUS:
        {
            std::string globalIP = servMgr->serverHost.IPtoStr();
            auto port            = servMgr->serverHost.port;
            std::string localIP  = servMgr->serverLocalIP.str();
            pcrs_jrpc_put_int(out, servMgr->getUptime());
            pcrs_jrpc_put_int(out, servMgr->getFirewall(4));
            pcrs_jrpc_put_int(out, port);
            putBytes(out, globalIP);
            putBytes(out, localIP);
            break;
        }
        case PCRS_JRPC_STATE:
            switch (a.i)
            {
            case 0: putBytes(out, servMgr->getState().inspect()); break;
            case 1: putBytes(out, chanMgr->getState().inspect()); break;
            case 2: putBytes(out, stats.getState().inspect()); break;
            case 3: putBytes(out, g_notificationBuffer.getState().inspect()); break;
            case 4: putBytes(out, sys->getState().inspect()); break;
            default: putBytes(out, g_ypList->getState().inspect()); break;
            }
            break;
        case PCRS_JRPC_UPDATE_INFO:
        {
            auto channel = chanMgr->findChannelByID(id);
            if (!channel)
                break;

            ChanInfo i = channel->info;
            i.name    = s(a.fields[0]).c_str();
            i.desc    = s(a.fields[1]).c_str();
            i.genre   = s(a.fields[2]).c_str();
            i.url     = s(a.fields[3]).c_str();
            i.comment = s(a.fields[4]).c_str();

            i.track.contact = s(a.fields[5]).c_str();
            i.track.title   = s(a.fields[6]).c_str();
            i.track.artist  = s(a.fields[7]).c_str();
            i.track.album   = s(a.fields[8]).c_str();
            i.track.genre   = s(a.fields[9]).c_str();

            channel->updateInfo(i);
            break;
        }
        case PCRS_JRPC_YP_CHANNELS:
            for (auto& c : servMgr->channelDirectory->channels())
            {
                pcrs_jrpc_yp v = {};
                v.feed_url = b(c.feedUrl);
                v.name = b(c.name);
                memcpy(v.id, c.id.id, 16);
                v.tip = b(c.tip);
                v.url = b(c.url);
                v.genre = b(c.genre);
                v.desc = b(c.desc);
                v.comment = b(c.comment);
                v.bitrate = c.bitrate;
                v.content_type = b(c.contentTypeStr);
                v.track_name = b(c.trackName);
                v.track_album = b(c.trackAlbum);
                v.track_artist = b(c.trackArtist);
                v.track_contact = b(c.trackContact);
                v.num_directs = c.numDirects;
                v.num_relays = c.numRelays;
                pcrs_jrpc_put_yp(out, &v);
            }
            break;
        case PCRS_JRPC_READ_STORAGE:
        {
            std::string dir = peercastApp->getStateDirPath();
            std::string path = dir + "/" + s(a.a) + ".json";

            FileStream fs;
            try {
                fs.openReadOnly(path);
            } catch (StreamException& e)
            {
                pcrs_jrpc_put_int(out, 0);
                break;
            }
            auto size = fs.length();
            char* buf = new char [size];
            fs.read(buf, size);
            fs.close();
            std::string str(buf, buf + size);
            delete[] buf;

            pcrs_jrpc_put_int(out, 1);
            putBytes(out, str);
            break;
        }
        case PCRS_JRPC_WRITE_STORAGE:
        {
            std::string dir = peercastApp->getStateDirPath();
            std::string path = dir + "/" + s(a.a) + ".json";

            FileStream fs;
            fs.openWriteReplace(path);

            fs.writeString(s(a.b));
            fs.close();
            break;
        }
        case PCRS_JRPC_CHANNELS_FOUND:
        {
            std::lock_guard<std::recursive_mutex> cs(chanMgr->lock);
            for (auto hitList = chanMgr->hitlist; hitList; hitList = hitList->next)
            {
                if (!hitList->isUsed())
                    continue;

                ChanInfo info = hitList->info;
                pcrs_jrpc_found f = {};
                f.info = rsInfo(info);
                f.uptime = info.getUptime();
                f.skips = info.numSkips;
                f.age = info.getAge();
                f.bcflags = info.bcID.getFlags();
                f.hosts = hitList->numHits();
                f.listeners = hitList->numListeners();
                f.relays = hitList->numRelays();
                f.firewalled = hitList->numFirewalled();
                f.closest = hitList->closestHit();
                f.furthest = hitList->furthestHit();
                f.newest = sys->getTime() - hitList->newestHit();
                pcrs_jrpc_put_found(out, &f);

                for (auto h = hitList->hit; h; h = h->next)
                {
                    if (!h->host.ip)
                        continue;
                    std::string ip = h->host;
                    pcrs_jrpc_found_hit v = {};
                    v.ip = b(ip);
                    v.hops = h->numHops;
                    v.listeners = h->numListeners;
                    v.relays = h->numRelays;
                    v.uptime = h->upTime;
                    v.push = h->firewalled;
                    v.relay = h->relay;
                    v.direct = h->direct;
                    v.cin = h->cin;
                    v.stable = h->stable;
                    v.version = h->version;
                    v.update = sys->getTime() - h->time;
                    v.tracker = h->tracker;
                    pcrs_jrpc_put_found_hit(out, &v);
                }
            }
            break;
        }
        default:
            throw std::logic_error("jrpc: unknown op");
        }
    }

    pcrs_jrpc_host m_host;
};

// Rust の通知を受けて nlohmann::json を組み立てる。
class JsonBuilder
{
public:
    using json = nlohmann::json;

    JsonBuilder()
    {
        m_builder.ctx = this;
        m_builder.null_value = [](void* c) { self(c)->value(nullptr); };
        m_builder.boolean = [](void* c, bool v) { self(c)->value(v); };
        m_builder.integer = [](void* c, int64_t v) { self(c)->value(static_cast<json::number_integer_t>(v)); };
        m_builder.unsigned_integer = [](void* c, uint64_t v) { self(c)->value(static_cast<json::number_unsigned_t>(v)); };
        m_builder.number = [](void* c, double v) { self(c)->value(v); };
        m_builder.string = [](void* c, const uint8_t* s, size_t n) {
            self(c)->value(std::string(reinterpret_cast<const char*>(s), n));
        };
        m_builder.begin_array = [](void* c) { self(c)->begin(json::array()); };
        m_builder.begin_object = [](void* c) { self(c)->begin(json::object()); };
        m_builder.key = [](void* c, const uint8_t* s, size_t n) {
            self(c)->m_keys.back().assign(reinterpret_cast<const char*>(s), n);
        };
        m_builder.end = [](void* c) { self(c)->end(); };
    }

    const pcrs_json_builder* get() const { return &m_builder; }

    json result;

private:
    static JsonBuilder* self(void* c) { return static_cast<JsonBuilder*>(c); }

    void value(json v)
    {
        if (m_stack.empty())
            result = std::move(v);
        else if (m_stack.back().is_array())
            m_stack.back().push_back(std::move(v));
        else
            m_stack.back()[m_keys.back()] = std::move(v);
    }

    void begin(json v)
    {
        m_stack.push_back(std::move(v));
        m_keys.emplace_back();
    }

    void end()
    {
        json v = std::move(m_stack.back());
        m_stack.pop_back();
        m_keys.pop_back();
        value(std::move(v));
    }

    pcrs_json_builder m_builder;
    std::vector<json> m_stack;
    std::vector<std::string> m_keys;
};

// JrpcApi::call の本体。応答を書き出せなかったときは、C++ 版と同じく例外を投げる。
inline std::string jrpcCall(const std::string& request)
{
    JrpcHost host;
    RustBuf out;
    int r = pcrs_jrpc_call(reinterpret_cast<const uint8_t*>(request.data()), request.size(), host.get(), out.out());
    if (r != 0)
        throw std::runtime_error(out.str());
    return out.str();
}

} // namespace rustbridge

#endif
