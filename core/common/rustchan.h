// チャンネルの情報とホスト (ChanInfo、TrackInfo、Host、ChanHit) を、peercast-rs に渡す形
// (peercast_rs.h の pcrs_chan_info、pcrs_host、pcrs_hit) にする。WITH_RUST_CORE のときだけ使う。
// 渡すのは中身を指すポインタなので、元のオブジェクトより長く使わないこと。
#ifndef _RUSTCHAN_H
#define _RUSTCHAN_H

#include <cstring>

#include "chanhit.h"
#include "chaninfo.h"
#include "peercast_rs.h"

inline pcrs_bytes rsBytes(const char* s)
{
    return { reinterpret_cast<const uint8_t*>(s), strlen(s) };
}

inline void rsTrack(pcrs_chan_info& v, const TrackInfo& t)
{
    v.track_contact = rsBytes(t.contact.data);
    v.track_title   = rsBytes(t.title.data);
    v.track_artist  = rsBytes(t.artist.data);
    v.track_album   = rsBytes(t.album.data);
    v.track_genre   = rsBytes(t.genre.data);
}

inline pcrs_chan_info rsInfo(const ChanInfo& i)
{
    pcrs_chan_info v = {};
    v.name         = rsBytes(i.name.data);
    v.content_type = rsBytes(i.contentType.data);
    v.mime         = rsBytes(i.MIMEType.data);
    v.ext          = rsBytes(i.streamExt.data);
    v.desc         = rsBytes(i.desc.data);
    v.genre        = rsBytes(i.genre.data);
    v.url          = rsBytes(i.url.data);
    v.comment      = rsBytes(i.comment.data);
    rsTrack(v, i.track);
    memcpy(v.id, i.id.id, 16);
    memcpy(v.bcid, i.bcID.id, 16);
    v.bitrate = i.bitrate;
    v.status  = i.status;
    return v;
}

inline pcrs_host rsHost(const Host& h)
{
    pcrs_host r;
    in6_addr a = h.ip.serialize();
    memcpy(r.ip, a.s6_addr, 16);
    r.port = h.port;
    return r;
}

inline pcrs_hit rsHit(const ChanHit& h)
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

#endif
