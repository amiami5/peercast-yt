//! PCP のストリーム (core/common/pcp.cpp の `PCPStream`) と、受け取ったパケットの処理でサーバーの
//! 状態を触る部分 (core/common/rustpcp.h の `PcpHost` に当たるもの)。パケットの解析と判断は段階 6 の
//! `crate::pcp`。

use std::sync::{Arc, Mutex};

use super::chanhit::{ChanHit, GnuIdList};
use super::chaninfo::{self as ci, ChanInfo};
use super::channel::Channel;
use super::error::{Error, Result};
use super::host::Ip;
use super::packetbuf::{self as pb, ChanPacket, PacketBuffer, MAX_DATALEN};
use super::pcpconst::*;
use super::pcstr::StrType;
use super::peercast::Peercast;
use super::stream::{Stream, StreamExt};
use super::sys;
use crate::pcp::{self as pcp, BroadcastState};
use crate::reader::Abort;

/// ほかのスレッドから使う部分 (送るパケット、経路の ID、相手の ID)
pub struct PcpShared {
    pub out_data: PacketBuffer,
    pub route_list: Mutex<GnuIdList>,
    pub remote_id: Mutex<[u8; 16]>,
}

impl PcpShared {
    pub fn new(remote_id: [u8; 16]) -> PcpShared {
        let out = PacketBuffer::new();
        out.init_accept(pb::T_PCP);
        PcpShared { out_data: out, route_list: Mutex::new(GnuIdList::new(1000)), remote_id: Mutex::new(remote_id) }
    }

    pub fn remote_id(&self) -> [u8; 16] {
        *self.remote_id.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `sendPacket`: 宛先が相手でも経路の先でもなければ送らない
    pub fn send_packet(&self, pack: &ChanPacket, dest_id: &[u8; 16]) -> bool {
        if ci::is_set(dest_id)
            && *dest_id != self.remote_id()
            && !self.route_list.lock().unwrap_or_else(|e| e.into_inner()).contains(dest_id)
        {
            return false;
        }
        let mut p = pack.clone();
        self.out_data.write_packet(&mut p, false)
    }
}

/// `PCPStream`
pub struct PcpStream {
    pub shared: Arc<PcpShared>,
    pub in_data: PacketBuffer,
    pub last_packet_time: u32,
    pub next_root_packet: u32,
}

impl PcpStream {
    pub fn new(remote_id: [u8; 16]) -> PcpStream {
        let in_data = PacketBuffer::new();
        in_data.init_accept(pb::T_PCP);
        PcpStream { shared: Arc::new(PcpShared::new(remote_id)), in_data, last_packet_time: 0, next_root_packet: 0 }
    }

    /// `init`
    pub fn init(&mut self, remote_id: [u8; 16]) {
        *self.shared.remote_id.lock().unwrap_or_else(|e| e.into_inner()) = remote_id;
        self.shared.route_list.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.last_packet_time = 0;
        self.next_root_packet = 0;
        self.in_data.init_accept(pb::T_PCP);
        self.shared.out_data.init_accept(pb::T_PCP);
    }

    /// `flush`: たまっている送るパケットを書く
    pub fn flush(&self, out: &mut dyn Stream) -> Result<()> {
        while self.shared.out_data.num_pending() > 0 {
            let p = self.shared.out_data.read_packet()?;
            p.write_raw(out)?;
        }
        Ok(())
    }

    /// `readPacket`: 送るパケットを 1 つ書き、届いたパケットを 1 つ読んで処理する。誤りなら 0 以外
    pub fn read_packet(&mut self, pc: &Arc<Peercast>, io: &mut dyn Stream, bcs: &mut BroadcastState) -> i32 {
        let mut error = PCP_ERROR_GENERAL;
        let r: Result<()> = (|| {
            error = PCP_ERROR_WRITE;
            if self.shared.out_data.num_pending() > 0 {
                let p = self.shared.out_data.read_packet()?;
                p.write_raw(io)?;
            }
            if self.shared.out_data.will_skip() {
                error = PCP_ERROR_WRITE + PCP_ERROR_SKIP;
                return Err(Error::stream("Send too slow"));
            }
            error = PCP_ERROR_READ;
            if io.read_ready(0) {
                let (id, numc, numd) = read_header(io)?;
                let mut buf = Vec::new();
                copy_atoms(io, id, numc, numd, &mut buf, 0)?;
                let mut pack = ChanPacket::new(pb::T_PCP, &buf, 0)?;
                self.in_data.write_packet(&mut pack, false);
            }
            error = PCP_ERROR_GENERAL;
            if self.in_data.num_pending() > 0 {
                let pack = self.in_data.read_packet()?;
                let mut data = pack.data.clone();
                data.resize(MAX_DATALEN, 0);
                error = self.proc_packet(pc, &mut data, bcs)?;
                if error != 0 {
                    return Err(Error::stream("PCP exception"));
                }
            }
            error = 0;
            Ok(())
        })();
        if let Err(e) = r {
            crate::log_error!("PCP readPacket: {} ({})", e, error);
        }
        error
    }

    /// 受け取ったパケットの処理 (`procAtom`)
    fn proc_packet(&mut self, pc: &Arc<Peercast>, data: &mut [u8], bcs: &mut BroadcastState) -> Result<i32> {
        let mut state = pcp::State { bcs: bcs.clone(), next_root_packet: self.next_root_packet };
        let mut host = PcpHost { pc, shared: &self.shared, ch: None, has_chl: false, new_info: ChanInfo::new(), error: None };
        let r = pcp::proc_packet(&mut host, data, &mut state);
        *bcs = state.bcs;
        self.next_root_packet = state.next_root_packet;
        match r {
            Ok(v) => Ok(v),
            Err(pcp::Error::Abort) => Err(host.error.take().unwrap_or_else(|| Error::stream("PCP: aborted"))),
            Err(pcp::Error::Stream(m)) => Err(Error::stream(m)),
        }
    }
}

/// atom の見出しを読む (ID、子の数、中身の長さ)
pub fn read_header(io: &mut dyn Stream) -> Result<([u8; 4], i32, i32)> {
    let b = io.read_fixed(8)?;
    let id = [b[0], b[1], b[2], b[3]];
    let v = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    if v & 0x8000_0000 != 0 {
        Ok((id, (v & 0x7fff_ffff) as i32, 0))
    } else {
        Ok((id, 0, v as i32))
    }
}

/// 子の入れ子の上限 (C++ 版には上限がなく、深い入れ子で再帰が深くなっていた)
const MAX_COPY_DEPTH: i32 = 64;

/// `AtomStream::writeAtoms` (入力はソケット): 見出しを読み終えた atom を子も含めて `out` に写す。
/// パケットの大きさ (16KB) を超えたら C++ 版の `MemoryStream` と同じく例外。
pub fn copy_atoms(io: &mut dyn Stream, id: [u8; 4], cnt: i32, data: i32, out: &mut Vec<u8>, depth: i32) -> Result<i32> {
    if depth > MAX_COPY_DEPTH {
        return Err(Error::stream("PCP: atom nesting too deep"));
    }
    let push = |out: &mut Vec<u8>, b: &[u8]| -> Result<()> {
        if out.len() + b.len() > MAX_DATALEN {
            return Err(Error::stream("Stream - premature end of write()"));
        }
        out.extend_from_slice(b);
        Ok(())
    };
    let mut total = 0i32;
    if cnt != 0 {
        push(out, &id)?;
        push(out, &((cnt as u32) | 0x8000_0000).to_le_bytes())?;
        total = total.wrapping_add(8);
        for _ in 0..cnt {
            let (cid, c, d) = read_header(io)?;
            total = total.wrapping_add(copy_atoms(io, cid, c, d, out, depth + 1)?);
        }
    } else {
        push(out, &id)?;
        push(out, &data.to_le_bytes())?;
        // writeTo: 4096 バイトずつ読んで書く
        let mut len = data as i64;
        if len < 0 {
            return Err(Error::stream("Stream::read: negative length"));
        }
        let mut tmp = [0u8; 4096];
        while len > 0 {
            let r = len.min(4096) as usize;
            tmp[..r].iter_mut().for_each(|b| *b = 0);
            io.read(&mut tmp[..r])?;
            push(out, &tmp[..r])?;
            len -= r as i64;
        }
        total = total.wrapping_add(8).wrapping_add(data);
    }
    Ok(total)
}

/// PCP のパケットの処理から見たサーバー (`PcpHost`)
struct PcpHost<'a> {
    pc: &'a Arc<Peercast>,
    shared: &'a Arc<PcpShared>,
    ch: Option<Arc<Channel>>,
    has_chl: bool,
    new_info: ChanInfo,
    error: Option<Error>,
}

impl PcpHost<'_> {
    fn guard(&mut self, r: Result<()>) -> std::result::Result<(), Abort> {
        match r {
            Ok(()) => Ok(()),
            Err(e) => {
                self.error = Some(e);
                Err(Abort)
            }
        }
    }

    fn info_field(&mut self, f: pcp::InfoField) -> &mut super::pcstr::PcString {
        let i = &mut self.new_info;
        match f {
            pcp::InfoField::Name => &mut i.name,
            pcp::InfoField::Genre => &mut i.genre,
            pcp::InfoField::Url => &mut i.url,
            pcp::InfoField::Desc => &mut i.desc,
            pcp::InfoField::Comment => &mut i.comment,
            pcp::InfoField::ContentType => &mut i.content_type,
            pcp::InfoField::MimeType => &mut i.mime_type,
            pcp::InfoField::StreamExt => &mut i.stream_ext,
            pcp::InfoField::TrackTitle => &mut i.track.title,
            pcp::InfoField::TrackCreator => &mut i.track.artist,
            pcp::InfoField::TrackUrl => &mut i.track.contact,
            pcp::InfoField::TrackAlbum => &mut i.track.album,
        }
    }

    fn find_chan(&mut self, id: &[u8; 16]) {
        self.ch = self.pc.chanmgr.find_channel_by_id(id);
        self.has_chl = self.pc.chanmgr.has_hitlist_by_id(id);
    }
}

fn to_ip(a: &pcp::atom::Ip) -> Ip {
    match a {
        pcp::atom::Ip::V4(v) => Ip::from_v4(*v),
        pcp::atom::Ip::V6(b) => Ip(*b),
    }
}

impl pcp::Host for PcpHost<'_> {
    fn session_id(&mut self) -> [u8; 16] {
        self.pc.servmgr.session_id
    }

    fn is_root(&mut self) -> bool {
        self.pc.servmgr.is_root()
    }

    fn time(&mut self) -> u32 {
        sys::get_time()
    }

    fn log(&mut self, level: pcp::Level, msg: &[u8]) {
        let l = match level {
            pcp::Level::Debug => super::log::Level::Debug,
            pcp::Level::Info => super::log::Level::Info,
            pcp::Level::Error => super::log::Level::Error,
        };
        super::log::add_log(l, msg);
    }

    fn route_add(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.shared.route_list.lock().unwrap_or_else(|e| e.into_inner()).add(id);
        Ok(())
    }

    fn set_update_interval(&mut self, si: i32) -> std::result::Result<(), Abort> {
        self.pc.chanmgr.set_update_interval(si as u32);
        Ok(())
    }

    fn upgrade(&mut self, url: &[u8]) -> std::result::Result<(), Abort> {
        {
            let mut s = self.pc.servmgr.settings();
            let u = &url[..url.iter().position(|&c| c == 0).unwrap_or(url.len())];
            s.download_url = u[..u.len().min(127)].to_vec();
        }
        self.pc.notify_message(
            super::notif::NT_UPGRADE,
            b"There is a new version of PeerCast available, please click here to upgrade your client.",
        );
        Ok(())
    }

    fn tracker_update(&mut self) -> std::result::Result<(), Abort> {
        let rid = self.shared.remote_id();
        self.pc.chanmgr.broadcast_tracker_update(self.pc, &rid, true);
        Ok(())
    }

    fn root_message(&mut self, msg: &[u8]) -> std::result::Result<(), Abort> {
        let msg = &msg[..msg.iter().position(|&c| c == 0).unwrap_or(msg.len())];
        let changed = {
            let mut s = self.pc.servmgr.settings();
            if s.root_msg.data != msg {
                s.root_msg.set(msg, StrType::Ascii);
                Some((s.root_host.data.clone(), s.root_msg.data.clone()))
            } else {
                None
            }
        };
        if let Some((host, m)) = changed {
            crate::log_debug!("PCP got new root mesg: {}", String::from_utf8_lossy(&m));
            if !m.is_empty() {
                let mut n = host;
                n.extend_from_slice("「".as_bytes());
                n.extend_from_slice(&m);
                n.extend_from_slice("」".as_bytes());
                self.pc.notify_message(super::notif::NT_PEERCAST, &n);
            }
        }
        Ok(())
    }

    fn hit(&mut self, h: &pcp::Hit, add: bool) -> std::result::Result<(), Abort> {
        let mut hit = ChanHit::new();
        for i in 0..2 {
            if let Some(ip) = &h.rhost_ip[i] {
                hit.rhost[i].ip = to_ip(ip);
            }
            if let Some(p) = h.rhost_port[i] {
                hit.rhost[i].port = p as u16;
            }
        }
        if let Some(v) = h.num_listeners {
            hit.num_listeners = v as u32;
        }
        if let Some(v) = h.num_relays {
            hit.num_relays = v as u32;
        }
        if let Some(v) = h.up_time {
            hit.up_time = v as u32;
        }
        if let Some(v) = h.oldest_pos {
            hit.oldest_pos = v as u32;
        }
        if let Some(v) = h.newest_pos {
            hit.newest_pos = v as u32;
        }
        if let Some(v) = h.version {
            hit.version = v as u32;
        }
        if let Some(v) = h.version_vp {
            hit.version_vp = v as u32;
        }
        if let Some(v) = h.version_ex_prefix {
            hit.version_ex_prefix = v;
        }
        if let Some(v) = h.version_ex_number {
            hit.version_ex_number = v as u32;
        }
        if let Some(fl1) = h.flags1 {
            hit.recv = fl1 & 0x10 != 0;
            hit.relay = fl1 & 0x02 != 0;
            hit.direct = fl1 & 0x04 != 0;
            hit.cin = fl1 & 0x20 != 0;
            hit.tracker = fl1 & 0x01 != 0;
            hit.firewalled = fl1 & 0x08 != 0;
        }
        if let Some(v) = h.session_id {
            hit.session_id = v;
        }
        if let Some(ip) = &h.uphost_ip {
            hit.uphost.ip = to_ip(ip);
        }
        if let Some(p) = h.uphost_port {
            hit.uphost.port = p as u16;
        }
        if let Some(v) = h.uphost_hops {
            hit.uphost_hops = v as u32;
        }
        hit.host = hit.rhost[0];
        hit.chan_id = h.chan_id;
        hit.num_hops = h.num_hops as u32;
        if add {
            self.pc.chanmgr.add_hit(self.pc, &hit);
        } else {
            self.pc.chanmgr.del_hit(&hit);
        }
        Ok(())
    }

    fn push(&mut self, ip: Option<pcp::atom::Ip>, port: Option<i32>, chan_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        let mut host = super::host::Host::none();
        if let Some(ip) = &ip {
            host.ip = to_ip(ip);
        }
        if let Some(p) = port {
            host.port = p as u16;
        }
        let pc = self.pc;
        let go = if ci::is_set(chan_id) {
            match pc.chanmgr.find_channel_by_id(chan_id) {
                Some(ch) => ch.is_broadcasting() || (!ch.is_full(pc) && !pc.servmgr.relays_full(self.pc) && ch.id() == *chan_id),
                None => false,
            }
        } else {
            true
        };
        if go {
            let s = pc.servmgr.alloc_servent();
            crate::log_debug!("GIVing to {}", host.str());
            super::servent::init_giv(pc, &s, host, *chan_id);
        }
        Ok(())
    }

    fn chan_begin(&mut self, chan_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.new_info = ChanInfo::new();
        self.find_chan(chan_id);
        if let Some(ch) = &self.ch {
            self.new_info = ch.info();
        } else if self.has_chl {
            if let Some(i) = self.pc.chanmgr.with_hitlist_by_id(chan_id, |chl| chl.info.clone()) {
                self.new_info = i;
            }
        }
        Ok(())
    }

    fn chan_has_channel(&mut self) -> bool {
        self.ch.is_some()
    }

    fn chan_packet(&mut self, p: &pcp::Packet) -> std::result::Result<(), Abort> {
        let r = (|| -> Result<()> {
            if p.data.len() > MAX_DATALEN {
                return Err(Error::stream("Data size too large"));
            }
            let ch = match &self.ch {
                Some(c) => c.clone(),
                None => return Ok(()),
            };
            let mut pack = ChanPacket { ty: p.kind, pos: p.pos, sync: 0, cont: p.cont, data: p.data.clone() };
            let stream_pos = ch.st().stream_pos;
            let diff = pack.pos as i64 - stream_pos as i64;
            if diff != 0 {
                let sdiff = if diff > 0 { format!("+{}", diff) } else { diff.to_string() };
                crate::log_debug!("PCP skipping {} ({} -> {})", sdiff, stream_pos, pack.pos);
            }
            if pack.ty == pb::T_HEAD {
                crate::log_debug!("New head packet at {}", pack.pos);
                if pack.pos == 0 {
                    crate::log_info!("PCP resetting stream");
                    {
                        let mut st = ch.st();
                        st.stream_index = st.stream_index.wrapping_add(1);
                    }
                    ch.raw_data.init();
                }
                ch.st().head_pack = pack.clone();
                ch.raw_data.write_packet(&mut pack, true);
                let mut st = ch.st();
                st.stream_pos = pack.pos.wrapping_add(pack.len());
            } else if pack.ty == pb::T_DATA {
                ch.raw_data.write_packet(&mut pack, true);
                let mut st = ch.st();
                st.stream_pos = pack.pos.wrapping_add(pack.len());
            }
            Ok(())
        })();
        self.guard(r)
    }

    fn chan_info_string(&mut self, field: pcp::InfoField, bytes: &[u8]) -> std::result::Result<(), Abort> {
        let s = self.info_field(field);
        // readString で String に書いたもの (NUL まで、255 バイトまで)。種類は変えない
        let b = &bytes[..bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len())];
        s.data = b[..b.len().min(255)].to_vec();
        if matches!(field, pcp::InfoField::Url | pcp::InfoField::TrackUrl) && !crate::url::is_http_url(&s.data) {
            s.clear();
        }
        Ok(())
    }

    fn chan_info_bitrate(&mut self, bitrate: i32) -> std::result::Result<(), Abort> {
        self.new_info.bitrate = bitrate;
        Ok(())
    }

    fn chan_bcid(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.new_info.bc_id = *id;
        Ok(())
    }

    fn chan_id(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.new_info.id = *id;
        self.find_chan(id);
        Ok(())
    }

    fn chan_end(&mut self) -> std::result::Result<(), Abort> {
        let pc = self.pc;
        let id = self.new_info.id;
        if !self.has_chl {
            pc.chanmgr.add_hit_list(&self.new_info);
            self.has_chl = true;
        }
        let new_info = self.new_info.clone();
        let log_info = pc.chanmgr.with_hitlist_by_id(&id, |chl| {
            chl.info.update(&new_info);
            chl.clone()
        });
        let chan_log = pc.servmgr.settings().chan_log.data.clone();
        if let (Some(chl), false) = (log_info, chan_log.is_empty()) {
            let mut rn = super::xmlnode::XmlNode::new(format!("update time=\"{}\"", sys::get_time()));
            let mut n = chl.info.channel_xml(pc.chanmgr.max_uptime());
            n.add(chl.xml(false));
            n.add(chl.info.track_xml());
            rn.add(n);
            let mut out = Vec::new();
            match rn.write(&mut out).and_then(|_| super::stream::FileStream::open_append(&chan_log)) {
                Ok(mut f) => {
                    let _ = f.write(&out);
                }
                Err(e) => crate::log_error!("Unable to update channel log: {}", e),
            }
        }
        if let Some(ch) = &self.ch {
            if !ch.is_broadcasting() {
                ch.update_info(pc, &new_info);
            }
        }
        Ok(())
    }

    fn broadcast(&mut self, target: pcp::Target, pack: &[u8], chan_id: &[u8; 16], dest_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        let r = (|| -> Result<()> {
            if pack.len() > MAX_DATALEN {
                return Err(Error::stream("Packet data too large"));
            }
            let mut p = ChanPacket::new(pb::T_PCP, pack, 0)?;
            let rid = self.shared.remote_id();
            let pc = self.pc;
            match target {
                pcp::Target::Up => {
                    pc.chanmgr.broadcast_packet_up(&p, chan_id, &rid, dest_id);
                }
                pcp::Target::Cout => {
                    pc.servmgr.broadcast_packet(&mut p, chan_id, &rid, dest_id, super::servent::T_COUT);
                }
                pcp::Target::Cin => {
                    pc.servmgr.broadcast_packet(&mut p, chan_id, &rid, dest_id, super::servent::T_CIN);
                }
                pcp::Target::Relay => {
                    pc.servmgr.broadcast_packet(&mut p, chan_id, &rid, dest_id, super::servent::T_RELAY);
                }
            }
            Ok(())
        })();
        self.guard(r)
    }
}

/// `PCPStream::readVersion`
pub fn read_version(io: &mut dyn Stream) -> Result<i32> {
    let len = io.read_u32_le()? as i32;
    if len != 4 {
        return Err(Error::stream("Invalid PCP"));
    }
    let ver = io.read_u32_le()? as i32;
    crate::log_debug!("PCP ver: {}", ver);
    Ok(ver)
}

