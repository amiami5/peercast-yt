// メディアコンテナの解析 (peercast-rs の src/media) を ChannelStream として使う。
// WITH_RUST_CORE のビルドで flv.h などから使うほか、peercast-rs の差分テストからも使う。
//
// Rust の解析器は、pcrs_media_host のコールバックを通してだけ、入力の Stream と Channel を触る。
// コールバックは C++ 版の解析器と同じ順序で Channel のメンバーを読み書きする。
#ifndef _RUSTMEDIA_H
#define _RUSTMEDIA_H

#include <cstring>
#include <exception>
#include <memory>
#include <stdexcept>
#include <string>

#include "channel.h"
#include "rustbridge.h"
#include "sys.h"

namespace rustbridge
{

// 1 回の readHeader / readPacket の間、Stream と Channel を Rust に貸す。
// コールバックの中で起きた例外は保存しておき、check() が投げ直す。
class MediaHost
{
public:
    MediaHost(Stream& in, std::shared_ptr<Channel> ch)
        : m_in(in), m_reader(in), m_ch(ch)
    {
        m_host.ctx = this;
        m_host.reader = m_reader.get();
        m_host.ready = [](void* c, bool* out) {
            return self(c)->guard([&]() { *out = self(c)->m_in.readReady(); });
        };
        m_host.raise_bitrate = [](void* c) {
            return self(c)->guard([&]() {
                auto& ch = self(c)->m_ch;
                ChanInfo info = ch->info;
                int newBitrate = self(c)->m_in.stat.bytesInPerSecAvg() / 1000 * 8;
                if (newBitrate > info.bitrate) {
                    info.bitrate = newBitrate;
                    ch->updateInfo(info);
                }
            });
        };
        m_host.packet = [](void* c, const uint8_t* data, size_t len, bool cont, bool readDelay) {
            return self(c)->guard([&]() {
                auto& ch = self(c)->m_ch;
                ChanPacket pack;
                pack.type = ChanPacket::T_DATA;
                pack.pos = ch->streamPos;
                pack.len = checkedLen(len);
                pack.cont = cont;
                memcpy(pack.data, data, len);
                ch->newPacket(pack);
                if (readDelay)
                    ch->checkReadDelay(pack.len);
                ch->streamPos += pack.len;
            });
        };
        m_host.head = [](void* c, int kind, const uint8_t* data, size_t len) {
            return self(c)->guard([&]() { self(c)->head(kind, data, len); });
        };
        m_host.head_len = [](void* c) -> uint32_t { return self(c)->m_ch->headPack.len; };
        m_host.head_clear = [](void* c) { self(c)->m_ch->headPack.len = 0; };
        m_host.head_append = [](void* c, const uint8_t* data, size_t len) {
            return self(c)->guard([&]() {
                ChanPacket& head = self(c)->m_ch->headPack;
                if (head.len + len > ChanPacket::MAX_DATALEN)
                    throw StreamException("OGG packet too big for headPack");
                memcpy(&head.data[head.len], data, len);
                head.len += len;
            });
        };
        m_host.set_bitrate = [](void* c, int32_t bitrate) {
            return self(c)->guard([&]() {
                auto& ch = self(c)->m_ch;
                ChanInfo info = ch->info;
                info.bitrate = bitrate;
                ch->updateInfo(info);
            });
        };
        m_host.ogg_set_info = [](void* c, int32_t bitrate, bool ogm) {
            auto& ch = self(c)->m_ch;
            ch->info.bitrate = bitrate;
            if (ogm)
                ch->info.contentType = ChanInfo::T_OGM;
        };
        m_host.set_track = [](void* c, const pcrs_track* t) {
            return self(c)->guard([&]() {
                auto& ch = self(c)->m_ch;
                ChanInfo newInfo = ch->info;
                newInfo.track.clear();
                setField(newInfo.track.artist, t->artist);
                setField(newInfo.track.title, t->title);
                setField(newInfo.track.genre, t->genre);
                setField(newInfo.track.contact, t->contact);
                setField(newInfo.track.album, t->album);
                ch->updateInfo(newInfo);
            });
        };
        m_host.mp3_metadata = [](void* c, const uint8_t* buf, size_t len) {
            return self(c)->guard([&]() {
                char meta[1024 + 1] = {};
                memcpy(meta, buf, len < sizeof(meta) ? len : sizeof(meta));
                meta[1024] = 0;
                self(c)->m_ch->processMp3Metadata(meta);
            });
        };
        m_host.icy_meta_interval = [](void* c) -> int32_t { return self(c)->m_ch->icyMetaInterval; };
        m_host.read_delay = [](void* c) -> bool { return self(c)->m_ch->readDelay; };
        m_host.dtime = [](void*) { return sys->getDTime(); };
        m_host.time = [](void*) -> uint32_t { return sys->getTime(); };
        m_host.sleep = [](void*, int32_t ms) { sys->sleep(ms); };
        m_host.sleep_until = [](void* c, double t) { self(c)->m_ch->sleepUntil(t); };
        m_host.log = [](void*, int level, const uint8_t* msg, size_t len) {
            int n = static_cast<int>(len);
            const char* s = reinterpret_cast<const char*>(msg);
            switch (level)
            {
            case PCRS_LOG_TRACE: LOG_TRACE("%.*s", n, s); break;
            case PCRS_LOG_DEBUG: LOG_DEBUG("%.*s", n, s); break;
            case PCRS_LOG_INFO:  LOG_INFO("%.*s", n, s); break;
            case PCRS_LOG_WARN:  LOG_WARN("%.*s", n, s); break;
            default:             LOG_ERROR("%.*s", n, s); break;
            }
        };
    }

    const pcrs_media_host* get() const { return &m_host; }

    // Rust の結果 (pcrs_media_read_* の返り値) に応じて、C++ 版と同じ例外を投げる。
    void check(int code, RustBuf& err)
    {
        if (m_ex)
            std::rethrow_exception(m_ex);
        m_reader.rethrowIfAborted();
        if (code == 2)
            throw StreamException(err.str());
        if (code != 0)
            throw StreamException("media: aborted");
    }

private:
    static MediaHost* self(void* c) { return static_cast<MediaHost*>(c); }

    template <typename F>
    int guard(F f)
    {
        try {
            f();
            return 0;
        } catch (...) {
            m_ex = std::current_exception();
            return -1;
        }
    }

    static unsigned int checkedLen(size_t len)
    {
        if (len > ChanPacket::MAX_DATALEN)
            throw StreamException("Packet data too large");
        return static_cast<unsigned int>(len);
    }

    static void setField(::String& s, const pcrs_track_field& f)
    {
        if (!f.present)
            return;
        std::string v(reinterpret_cast<const char*>(f.ptr), f.len);
        s.set(v.c_str(), String::T_ASCII);
        s.convertTo(String::T_UNICODE);
    }

    void head(int kind, const uint8_t* data, size_t len)
    {
        auto& ch = m_ch;
        switch (kind)
        {
        case PCRS_HEAD_NEW_STREAM:
            memcpy(ch->headPack.data, data, checkedLen(len));
            ch->rawData.init();
            ch->streamIndex++;
            ch->headPack.type = ChanPacket::T_HEAD;
            ch->headPack.len = len;
            ch->headPack.pos = 0;
            ch->newPacket(ch->headPack);
            ch->streamPos = ch->headPack.len;
            break;
        case PCRS_HEAD_MKV:
        {
            ch->streamIndex++;
            ch->rawData.init();
            ch->streamPos = 0;
            ChanPacket pack;
            pack.type = ChanPacket::T_HEAD;
            pack.pos = ch->streamPos;
            pack.len = checkedLen(len);
            pack.cont = false;
            memcpy(pack.data, data, len);
            ch->headPack = pack;
            ch->newPacket(pack);
            ch->streamPos += pack.len;
            break;
        }
        case PCRS_HEAD_OGG:
            ch->headPack.type = ChanPacket::T_HEAD;
            ch->headPack.pos = ch->streamPos;
            ch->startTime = sys->getDTime();
            ch->streamPos += ch->headPack.len;
            ch->newPacket(ch->headPack);
            break;
        default:
            throw StreamException("media: unknown head kind");
        }
    }

    Stream& m_in;
    StreamReader m_reader;
    std::shared_ptr<Channel> m_ch;
    pcrs_media_host m_host;
    std::exception_ptr m_ex;
};

// Rust の解析器を持つ ChannelStream。FLVStream などはこれを継承する。
class MediaStream : public ChannelStream
{
public:
    explicit MediaStream(int kind) : m_kind(kind), m_parser(pcrs_media_new(kind)) {}
    ~MediaStream() override { pcrs_media_free(m_parser); }
    MediaStream(const MediaStream&) = delete;
    MediaStream& operator=(const MediaStream&) = delete;

    void readHeader(Stream& in, std::shared_ptr<Channel> ch) override
    {
        run(pcrs_media_read_header, in, ch);
    }

    int readPacket(Stream& in, std::shared_ptr<Channel> ch) override
    {
        run(pcrs_media_read_packet, in, ch);
        return 0;
    }

    void readEnd(Stream&, std::shared_ptr<Channel>) override
    {
        if (m_kind == PCRS_MEDIA_MP4)
            LOG_DEBUG("MP4Stream::readEnd");
    }

private:
    typedef int (*Fn)(pcrs_media*, const pcrs_media_host*, pcrs_buf*);

    void run(Fn f, Stream& in, std::shared_ptr<Channel> ch)
    {
        if (m_kind != PCRS_MEDIA_MKV)
            return runRust(f, in, ch);

        // MKVStream は、中で起きた std::runtime_error を同じメッセージの StreamException に
        // 変えていた。
        try {
            runRust(f, in, ch);
        } catch (std::runtime_error& e) {
            throw StreamException(e.what());
        }
    }

    void runRust(Fn f, Stream& in, std::shared_ptr<Channel> ch)
    {
        MediaHost host(in, ch);
        RustBuf err;
        int code = f(m_parser, host.get(), err.out());
        host.check(code, err);
    }

    int m_kind;
    pcrs_media* m_parser;
};

} // namespace rustbridge

#endif
