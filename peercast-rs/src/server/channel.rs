//! チャンネル (core/common/channel.cpp の `Channel`)。判断と文字列と atom の組み立ては段階 7d の
//! `crate::channel`。配信元 (`ChannelSource` の派生クラス) は `super::sources`。
//!
//! C++ 版はチャンネルのメンバーをロックなしで読み書きしていた箇所が多い。Rust 版はメンバーを
//! `ChanState` にまとめ、1 つのロックで守る。ロックを持ったままほかのロック (一覧や ServMgr) は
//! 取らない。

use std::sync::{Arc, Mutex, MutexGuard};

use super::chanhit::{ChanHit, ChanHitSearch};
use super::chaninfo::{self as ci, ChanInfo};
use super::host::Host;
use super::packetbuf::{self as pb, ChanPacket, PacketBuffer};
use super::pcpconst::*;
use super::pcstr::{PcString, StrType};
use super::peercast::Peercast;
use super::socket::ClientSocket;
use super::sources::{self, Source, SourceStream};
use super::state::{arr, obj, s, Value};
use super::stream::{Stat, Stream};
use super::sys;
use super::xmlnode::XmlNode;
use crate::pcp::write::AtomBuf;

// `Channel::STATUS`
pub const S_NONE: i32 = 0;
pub const S_WAIT: i32 = 1;
pub const S_CONNECTING: i32 = 2;
pub const S_REQUESTING: i32 = 3;
pub const S_CLOSING: i32 = 4;
pub const S_RECEIVING: i32 = 5;
pub const S_BROADCASTING: i32 = 6;
pub const S_ABORT: i32 = 7;
pub const S_SEARCHING: i32 = 8;
pub const S_NOHOSTS: i32 = 9;
pub const S_IDLE: i32 = 10;
pub const S_ERROR: i32 = 11;
pub const S_NOTFOUND: i32 = 12;

const STATUS_MSGS: [&str; 13] =
    ["NONE", "WAIT", "CONNECT", "REQUEST", "CLOSE", "RECEIVE", "BROADCAST", "ABORT", "SEARCH", "NOHOSTS", "IDLE", "ERROR", "NOTFOUND"];

// `Channel::TYPE`
pub const T_NONE: i32 = 0;
pub const T_ALLOCATED: i32 = 1;
pub const T_BROADCAST: i32 = 2;
pub const T_RELAY: i32 = 3;

// `Channel::SRC_TYPE`
pub const SRC_NONE: i32 = 0;
pub const SRC_PEERCAST: i32 = 1;
pub const SRC_SHOUTCAST: i32 = 2;
pub const SRC_ICECAST: i32 = 3;
pub const SRC_URL: i32 = 4;
pub const SRC_HTTPPUSH: i32 = 5;

const SRC_TYPES: [&str; 6] = ["NONE", "PEERCAST", "SHOUTCAST", "ICECAST", "URL", "HTTPPUSH"];

pub const IP_V4: i32 = 4;
pub const IP_V6: i32 = 6;

/// `Channel` のメンバー (ロックで守るもの)
pub struct ChanState {
    pub mount: PcString,
    /// `insertMeta` は使われていないので持たない
    pub head_pack: ChanPacket,
    pub stream_index: u32,
    pub info: ChanInfo,
    pub source_host: ChanHit,
    pub designated_host: ChanHit,
    pub remote_id: [u8; 16],
    pub source_url: PcString,
    pub bump: bool,
    pub stay_connected: bool,
    pub icy_meta_interval: i32,
    pub stream_pos: u32,
    pub read_delay: bool,
    pub ty: i32,
    pub src_type: i32,
    pub source: Option<Source>,
    pub last_idle_time: u32,
    pub status: i32,
    pub last_tracker_update: u32,
    pub last_meta_update: u32,
    pub start_time: f64,
    pub ip_version: i32,
    pub root_host: Vec<u8>,
    /// 受け取ったソケットのホスト (HTTP Push の `getSourceString` で使う)
    pub sock_host: Host,
}

impl ChanState {
    fn reset(&mut self) {
        self.source_host = ChanHit::new();
        self.remote_id = [0; 16];
        self.stream_index = 0;
        self.last_idle_time = 0;
        self.info.init();
        self.mount.clear();
        self.bump = false;
        self.stay_connected = false;
        self.icy_meta_interval = 0;
        self.stream_pos = 0;
        self.head_pack = ChanPacket::default();
        self.status = S_NONE;
        self.ty = T_NONE;
        self.read_delay = false;
        self.source_url.clear();
        self.source = None;
        self.last_tracker_update = 0;
        self.last_meta_update = 0;
        self.src_type = SRC_NONE;
        self.start_time = 0.0;
        self.ip_version = IP_V4;
    }
}

/// `Channel`
pub struct Channel {
    st: Mutex<ChanState>,
    pub raw_data: PacketBuffer,
    pub thread: sys::ThreadFlag,
    /// 受信しているストリームの量 (`getSourceRate`)
    pub src_stat: Mutex<Option<Arc<Stat>>>,
    /// 配信元のソケット (ICY と HTTP Push では始めるときに渡され、GIV では `acceptGIV` で入る)
    pub sock: Mutex<Option<ClientSocket>>,
    pub push_sock: Mutex<Option<ClientSocket>>,
    /// 配信元の PCP のストリーム (`sendPacketUp`)
    pub source_stream: Mutex<Option<Arc<sources::PcpShared>>>,
    /// スレッドが終わったこと (`waitThread`)
    done: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl Default for Channel {
    fn default() -> Self {
        Channel::new()
    }
}

impl Channel {
    pub fn new() -> Channel {
        let mut st = ChanState {
            mount: PcString::default(),
            head_pack: ChanPacket::default(),
            stream_index: 0,
            info: ChanInfo::new(),
            source_host: ChanHit::new(),
            designated_host: ChanHit::new(),
            remote_id: [0; 16],
            source_url: PcString::default(),
            bump: false,
            stay_connected: false,
            icy_meta_interval: 0,
            stream_pos: 0,
            read_delay: false,
            ty: T_NONE,
            src_type: SRC_NONE,
            source: None,
            last_idle_time: 0,
            status: S_NONE,
            last_tracker_update: 0,
            last_meta_update: 0,
            start_time: 0.0,
            ip_version: IP_V4,
            root_host: Vec::new(),
            sock_host: Host::none(),
        };
        st.reset();
        let raw_data = PacketBuffer::new();
        raw_data.init_accept(pb::T_HEAD | pb::T_DATA);
        Channel {
            st: Mutex::new(st),
            raw_data,
            thread: sys::ThreadFlag::new(),
            src_stat: Mutex::new(None),
            sock: Mutex::new(None),
            push_sock: Mutex::new(None),
            source_stream: Mutex::new(None),
            done: Mutex::new(None),
        }
    }

    pub fn st(&self) -> MutexGuard<'_, ChanState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `info` の写し
    pub fn info(&self) -> ChanInfo {
        self.st().info.clone()
    }

    pub fn id(&self) -> [u8; 16] {
        self.st().info.id
    }

    pub fn status(&self) -> i32 {
        self.st().status
    }

    /// `reset` (C++ 版はコンストラクターとスレッドの終わりで呼ぶ)
    pub fn reset(&self) {
        self.st().reset();
        self.raw_data.init_accept(pb::T_HEAD | pb::T_DATA);
        *self.sock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.push_sock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.source_stream.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.src_stat.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn is_active(&self) -> bool {
        self.st().ty != T_NONE
    }

    pub fn is_playing(&self) -> bool {
        let s = self.st().status;
        s == S_RECEIVING || s == S_BROADCASTING
    }

    pub fn is_receiving(&self) -> bool {
        self.st().status == S_RECEIVING
    }

    pub fn is_broadcasting(&self) -> bool {
        self.st().status == S_BROADCASTING
    }

    pub fn not_found(&self) -> bool {
        self.st().status == S_NOTFOUND
    }

    pub fn is_idle(&self) -> bool {
        let st = self.st();
        st.ty != T_NONE && st.status == S_IDLE
    }

    pub fn status_str(&self) -> &'static str {
        STATUS_MSGS.get(self.status() as usize).copied().unwrap_or("NONE")
    }

    pub fn src_type_str(&self) -> &'static str {
        SRC_TYPES.get(self.st().src_type as usize).copied().unwrap_or("NONE")
    }

    /// `resetPlayTime`
    pub fn reset_play_time(&self) {
        self.st().info.last_play_start = sys::get_time();
    }

    /// `setStatus`
    pub fn set_status(&self, pc: &Peercast, s: i32) {
        let (became_broadcasting, info) = {
            let mut st = self.st();
            if s == st.status {
                return;
            }
            let was_playing = st.status == S_RECEIVING || st.status == S_BROADCASTING;
            st.status = s;
            if s == S_RECEIVING || s == S_BROADCASTING {
                st.info.status = ci::S_PLAY;
                st.info.last_play_start = sys::get_time();
            } else {
                if was_playing {
                    st.info.last_play_end = sys::get_time();
                }
                st.info.status = ci::S_UNKNOWN;
            }
            (s == S_BROADCASTING, st.info.clone())
        };
        if became_broadcasting && !pc.chanmgr.has_hitlist_by_id(&info.id) {
            pc.chanmgr.add_hit_list(&info);
        }
    }

    /// `newPacket`: PCP のパケット以外を rawData に書く
    pub fn new_packet(&self, pack: &mut ChanPacket) {
        if pack.ty == pb::T_PCP {
            return;
        }
        self.raw_data.write_packet(pack, true);
    }

    /// `checkIdle`
    pub fn check_idle(&self, pc: &Peercast) -> bool {
        let (uptime, stay, status, id) = {
            let st = self.st();
            (st.info.uptime(pc.chanmgr.max_uptime()), st.stay_connected, st.status, st.info.id)
        };
        uptime > pc.chanmgr.settings().prefetch_time && pc.servmgr.num_streams(pc, &id, super::servent::T_DIRECT, true) == 0 && !stay && status != S_BROADCASTING
    }

    /// `isFull`: チャンネルごとのリレー数の上限に達しているか
    pub fn is_full(&self, pc: &Peercast) -> bool {
        let max = pc.chanmgr.settings().max_relays_per_channel;
        max != 0 && self.local_relays(pc, false) >= max
    }

    /// `canAddRelay`
    pub fn can_add_relay(&self, pc: &Peercast) -> bool {
        let br = self.st().info.bitrate;
        !(pc.servmgr.bitrate_full(pc, br as u32) || pc.servmgr.relays_full(pc) || self.is_full(pc))
    }

    /// `localRelays`
    pub fn local_relays(&self, pc: &Peercast, include_private: bool) -> i32 {
        pc.servmgr.num_streams(pc, &self.id(), super::servent::T_RELAY, include_private) as i32
    }

    /// `localListeners`
    pub fn local_listeners(&self, pc: &Peercast, include_private: bool) -> i32 {
        pc.servmgr.num_streams(pc, &self.id(), super::servent::T_DIRECT, include_private) as i32
    }

    /// `totalRelays`
    pub fn total_relays(&self, pc: &Peercast) -> i32 {
        pc.chanmgr.with_hitlist_by_id(&self.id(), |chl| chl.num_hits()).unwrap_or(0)
    }

    /// `totalListeners`
    pub fn total_listeners(&self, pc: &Peercast) -> i32 {
        self.local_listeners(pc, true) + pc.chanmgr.with_hitlist_by_id(&self.id(), |chl| chl.num_listeners()).unwrap_or(0)
    }

    /// `startGet`: 中継を始める
    pub fn start_get(&self, pc: &Arc<Peercast>) {
        {
            let mut st = self.st();
            st.src_type = SRC_PEERCAST;
            st.ty = T_RELAY;
            st.info.src_protocol = ci::SP_PCP;
            st.source = Some(Source::Peercast);
        }
        self.start_stream(pc);
    }

    /// `startURL`: URL から配信する
    pub fn start_url(&self, pc: &Arc<Peercast>, url: &[u8]) {
        {
            let mut st = self.st();
            st.source_url.assign(url);
            st.src_type = SRC_URL;
            st.ty = T_BROADCAST;
            st.stay_connected = true;
            st.info.last_play_start = sys::get_time();
            st.source = Some(Source::Url(st.source_url.data.clone()));
        }
        self.start_stream(pc);
    }

    /// `startHTTPPush`
    pub fn start_http_push(&self, pc: &Arc<Peercast>, sock: ClientSocket, chunked: bool) {
        {
            let mut st = self.st();
            st.src_type = SRC_HTTPPUSH;
            st.ty = T_BROADCAST;
            st.sock_host = sock.host;
            st.info.src_protocol = ci::SP_HTTP;
            st.source = Some(Source::HttpPush(chunked));
        }
        *self.sock.lock().unwrap_or_else(|e| e.into_inner()) = Some(sock);
        self.start_stream(pc);
    }

    /// `startICY`
    pub fn start_icy(&self, pc: &Arc<Peercast>, mut sock: ClientSocket, src_type: i32) {
        // データが来なくてもつながったままにする
        sock.set_read_timeout(0);
        let idx = {
            let mut s = pc.chanmgr.settings();
            s.icy_index = s.icy_index.wrapping_add(1);
            s.icy_index
        };
        {
            let mut st = self.st();
            st.src_type = src_type;
            st.ty = T_BROADCAST;
            st.sock_host = sock.host;
            st.info.src_protocol = ci::SP_HTTP;
            st.stream_index = idx;
            st.source = Some(Source::Icy);
        }
        *self.sock.lock().unwrap_or_else(|e| e.into_inner()) = Some(sock);
        self.start_stream(pc);
    }

    /// `startStream`: チャンネルのスレッドを始める
    fn start_stream(&self, pc: &Arc<Peercast>) {
        let ch = match pc.chanmgr.channels().into_iter().find(|c| std::ptr::eq(c.as_ref(), self)) {
            Some(c) => c,
            None => {
                self.reset();
                return;
            }
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        *self.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
        let pc = pc.clone();
        if !sys::start_thread(&self.thread, "CHANNEL", move || {
            stream_proc(&pc, &ch);
            let _ = tx.send(());
        }) {
            self.reset();
        }
    }

    /// `sys->waitThread(&thread)`: スレッドが終わるのを待つ (このスレッド自身なら待たない)
    pub fn wait_thread(&self) {
        let rx = self.done.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(rx) = rx {
            if std::thread::current().name() == Some("CHANNEL") && self.thread.active() {
                // 自分自身を待つことはしない (C++ 版の "Self joining avoided")
                crate::log_debug!("waitThread: Self joining avoided");
                return;
            }
            let _ = rx.recv();
        }
    }

    /// `sleepUntil`
    pub fn sleep_until(&self, time: f64) {
        let start = self.st().start_time;
        let mut sleep_time = time - (sys::get_dtime() - start);
        if sleep_time > 0.0 {
            if sleep_time > 60.0 {
                sleep_time = 60.0;
            }
            sys::sleep((sleep_time * 1000.0) as u32);
        }
    }

    /// `checkReadDelay`
    pub fn check_read_delay(&self, len: u32) {
        let (rd, br) = {
            let st = self.st();
            (st.read_delay, st.info.bitrate)
        };
        if let Some(t) = crate::channel::read_delay_ms(rd, len, br) {
            sys::sleep(t);
        }
    }

    /// `acceptGIV`
    pub fn accept_giv(&self, sock: ClientSocket) -> std::result::Result<(), ClientSocket> {
        let mut p = self.push_sock.lock().unwrap_or_else(|e| e.into_inner());
        if p.is_none() {
            *p = Some(sock);
            Ok(())
        } else {
            Err(sock)
        }
    }

    /// `processMp3Metadata`: `StreamTitle='...';StreamUrl='...';` から曲名と URL を取る
    pub fn process_mp3_metadata(&self, pc: &Peercast, s: &[u8]) {
        let mut new_info = self.info();
        let (title, url) = crate::channel::mp3_metadata(s);
        if let Some((pos, len)) = title {
            new_info.track.title.set_unquote(&s[pos..pos + len], StrType::Ascii);
            new_info.track.title.convert_to(StrType::Unicode);
        }
        if let Some((pos, len)) = url {
            new_info.track.contact.set_unquote(&s[pos..pos + len], StrType::Ascii);
            new_info.track.contact.convert_to(StrType::Unicode);
        }
        self.update_info(pc, &new_info);
    }

    /// `writeTrackerUpdateAtom`
    pub fn write_tracker_update_atom(&self, pc: &Peercast, out: &mut AtomBuf) -> super::error::Result<()> {
        let info = self.info();
        if !pc.chanmgr.has_hitlist_by_id(&info.id) {
            return Err(super::error::Error::stream("Broadcast channel has no hitlist"));
        }
        let hit = self.local_hit(pc, true);
        crate::channel::tracker_update_atom(out, &pc.servmgr.session_id, &pc.chanmgr.broadcast_id(), &info.view(), &hit.view());
        Ok(())
    }

    /// トラッカーへの更新やステータスで送る自分のヒット (`ChanHit::initLocal`)
    pub fn local_hit(&self, pc: &Peercast, total: bool) -> ChanHit {
        let (numl, numr) = if total {
            (self.total_listeners(pc), self.total_relays(pc))
        } else {
            (self.local_listeners(pc, true), self.local_relays(pc, true))
        };
        let (skips, uptime, source_host, ipv6) = {
            let st = self.st();
            (st.info.num_skips, st.info.uptime(pc.chanmgr.max_uptime()), st.source_host.host, st.ip_version == IP_V6)
        };
        let playing = self.is_playing();
        let (oldp, newp) = (self.raw_data.oldest_pos(), self.raw_data.latest_pos());
        let mut hit = pc.servmgr.init_local_hit(pc, numl, numr, skips as i32, uptime, playing, oldp, newp, self.can_add_relay(pc), source_host, ipv6);
        hit.tracker = total || self.is_broadcasting();
        hit
    }

    /// `broadcastTrackerUpdate`: 配信しているチャンネルの情報を YP に知らせる (30 秒ごと)
    pub fn broadcast_tracker_update(&self, pc: &Peercast, sv_id: &[u8; 16], force: bool) {
        let ctime = sys::get_time();
        let last = self.st().last_tracker_update;
        if ctime.wrapping_sub(last) > 30 || force {
            let mut out = AtomBuf::default();
            if let Err(e) = self.write_tracker_update_atom(pc, &mut out) {
                crate::log_error!("broadcastTrackerUpdate: {}", e);
                return;
            }
            let mut pack = match ChanPacket::new(pb::T_PCP, &out.0, 0) {
                Ok(p) => p,
                Err(_) => return,
            };
            let cnt = pc.servmgr.broadcast_packet(&mut pack, &[0; 16], &pc.servmgr.session_id, sv_id, super::servent::T_COUT);
            if cnt != 0 {
                crate::log_debug!("Sent tracker update for {} to {} client(s)", String::from_utf8_lossy(&self.st().info.name.data), cnt);
                self.st().last_tracker_update = ctime;
            }
        }
    }

    /// `sendPacketUp`
    pub fn send_packet_up(&self, pack: &ChanPacket, cid: &[u8; 16], sid: &[u8; 16], did: &[u8; 16]) -> bool {
        let (active, id, remote) = {
            let st = self.st();
            (st.ty != T_NONE, st.info.id, st.remote_id)
        };
        if active && (!ci::is_set(cid) || id == *cid) && (!ci::is_set(sid) || remote != *sid) {
            if let Some(ss) = self.source_stream.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return ss.send_packet(pack, did);
            }
        }
        false
    }

    /// `updateInfo`: 情報が変われば true
    pub fn update_info(&self, pc: &Peercast, new_info: &ChanInfo) -> bool {
        let (old_comment, info) = {
            let mut st = self.st();
            let old = st.info.comment.clone();
            if !st.info.update(new_info) {
                return false;
            }
            (old, st.info.clone())
        };
        if old_comment.data != info.comment.data {
            let c = info.comment.converted(StrType::Unicode);
            let mut msg = info.name.data.clone();
            msg.extend_from_slice("「".as_bytes());
            msg.extend_from_slice(&c);
            msg.extend_from_slice("」".as_bytes());
            pc.notify_message(super::notif::NT_PEERCAST, &msg);
        }
        if self.is_broadcasting() {
            let ctime = sys::get_time();
            let due = {
                let mut st = self.st();
                if ctime.wrapping_sub(st.last_meta_update) > 30 {
                    st.last_meta_update = ctime;
                    true
                } else {
                    false
                }
            };
            if due {
                let mut out = AtomBuf::default();
                crate::channel::info_update_atom(&mut out, &pc.servmgr.session_id, &info.view());
                if let Ok(mut pack) = ChanPacket::new(pb::T_PCP, &out.0, 0) {
                    pc.servmgr.broadcast_packet(&mut pack, &info.id, &pc.servmgr.session_id, &[0; 16], super::servent::T_RELAY);
                }
                self.broadcast_tracker_update(pc, &[0; 16], false);
            }
        }
        pc.chanmgr.with_hitlist(&info, |chl| chl.info = info.clone());
        true
    }

    /// `checkBump`
    pub fn check_bump(&self) -> bool {
        let lwt = self.raw_data.last_write_time();
        let mut st = self.st();
        if st.status != S_BROADCASTING && !st.source_host.tracker && lwt != 0 && sys::get_time().wrapping_sub(lwt) > 30 {
            crate::log_error!("Channel Auto bumped");
            st.bump = true;
        }
        if st.bump {
            st.bump = false;
            true
        } else {
            false
        }
    }

    /// `getSourceString`
    pub fn source_string(&self) -> String {
        let st = self.st();
        if st.source_url.is_empty() {
            if st.src_type == SRC_HTTPPUSH {
                st.sock_host.str()
            } else {
                let mut buf = st.source_host.str(true);
                if st.source_host.uphost.ip.is_set() {
                    buf.push_str(" recv. from ");
                    buf.push_str(&st.source_host.uphost.str());
                }
                buf
            }
        } else {
            String::from_utf8_lossy(&st.source_url.data).into_owned()
        }
    }

    /// `getSourceRate` / `getSourceRateAvg` (ICY の配信元は 0)
    pub fn source_rate(&self, avg: bool) -> u32 {
        let icy = matches!(self.st().source, Some(Source::Icy) | None);
        if icy {
            return 0;
        }
        match self.src_stat.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(s) => {
                if avg {
                    s.bytes_in_per_sec_avg()
                } else {
                    s.bytes_in_per_sec()
                }
            }
            None => 0,
        }
    }

    /// `getBufferString`
    pub fn buffer_string(&self) -> Vec<u8> {
        let has_source = self.st().source.is_some();
        let byterate = if has_source { self.source_rate(true) as f64 } else { 0.0 };
        let stat = self.raw_data.statistics();
        crate::channel::buffer_string(
            byterate,
            sys::get_time(),
            self.raw_data.last_write_time(),
            &stat.packet_lengths,
            stat.continuations,
            stat.non_continuations,
        )
    }

    /// `createRelayXML`
    pub fn relay_xml(&self, pc: &Peercast, show_stat: bool) -> XmlNode {
        let mut ststr = self.status_str();
        if !show_stat && self.is_playing() {
            ststr = "OK";
        }
        let hosts = pc.chanmgr.with_hitlist(&self.info(), |chl| chl.num_hits()).unwrap_or(0);
        XmlNode::new(format!(
            "relay listeners=\"{}\" relays=\"{}\" hosts=\"{}\" status=\"{}\"",
            self.local_listeners(pc, true),
            self.local_relays(pc, true),
            hosts,
            ststr
        ))
    }

    /// `getState`
    pub fn state(&self, pc: &Peercast) -> Value {
        let info = self.info();
        let (hits, num_hits) = pc
            .chanmgr
            .with_hitlist_by_id(&info.id, |chl| (chl.hits.iter().map(|h| h.state()).collect::<Vec<_>>(), chl.num_hits()))
            .unwrap_or_default();
        let (keep, stream_pos, head, ipv, root_host) = {
            let st = self.st();
            (st.stay_connected, st.stream_pos, st.head_pack.clone(), st.ip_version, st.root_host.clone())
        };
        let uni = |p: &PcString| s(p.converted(StrType::Unicode));
        let srcrate = if self.st().source.is_some() {
            format!("{:.0}", super::stats::bytes_to_kbps(self.source_rate(false)) as f64)
        } else {
            "0".into()
        };
        obj(vec![
            ("name", uni(&info.name)),
            ("bitrate", s(info.bitrate.to_string())),
            ("srcrate", s(srcrate)),
            ("genre", uni(&info.genre)),
            ("desc", uni(&info.desc)),
            ("comment", uni(&info.comment)),
            (
                "uptime",
                s(if info.last_play_start != 0 {
                    crate::pcstring::from_stopwatch(sys::get_time().wrapping_sub(info.last_play_start))
                } else {
                    b"-".to_vec()
                }),
            ),
            ("type", s(info.type_str())),
            ("typeLong", s(info.type_string_long())),
            ("ext", s(info.type_ext())),
            ("localRelays", s(self.local_relays(pc, true).to_string())),
            ("localListeners", s(self.local_listeners(pc, true).to_string())),
            ("totalRelays", s(self.total_relays(pc).to_string())),
            ("totalListeners", s(self.total_listeners(pc).to_string())),
            ("status", s(self.status_str())),
            ("keep", s(if keep { "Yes" } else { "No" })),
            ("id", s(ci::id_str(&info.id))),
            (
                "track",
                obj(vec![
                    ("title", uni(&info.track.title)),
                    ("artist", uni(&info.track.artist)),
                    ("album", uni(&info.track.album)),
                    ("genre", uni(&info.track.genre)),
                    ("contactURL", uni(&info.track.contact)),
                ]),
            ),
            ("contactURL", s(&info.url.data)),
            ("streamPos", s(crate::strutil::group_digits(stream_pos.to_string().as_bytes(), b","))),
            ("sourceType", s(self.src_type_str())),
            ("sourceProtocol", s(ci::protocol_str(info.src_protocol))),
            ("sourceURL", s(self.source_string())),
            ("headPos", s(crate::strutil::group_digits(head.pos.to_string().as_bytes(), b","))),
            ("headLen", s(crate::strutil::group_digits(head.data.len().to_string().as_bytes(), b","))),
            ("buffer", s(self.buffer_string())),
            ("headDump", s(crate::channel::render_hex_dump(&head.data))),
            ("numHits", s(num_hits.to_string())),
            ("hits", arr(hits)),
            ("authToken", s(pc.chanmgr.auth_token(&info.id))),
            ("plsExt", s(info.playlist_ext())),
            ("ipVersion", s(ipv.to_string())),
            ("rootHost", s(root_host)),
        ])
    }

    /// `readStream`: 入力を読み切るまで、パケットにしてバッファに入れる。誤りなら 0 以外
    pub fn read_stream(&self, pc: &Arc<Peercast>, ch: &Arc<Channel>, input: &mut dyn Stream, source: &mut SourceStream) -> i32 {
        let mut error = 0;
        self.st().info.num_skips = 0;
        if let Err(e) = source.read_header(pc, ch, input) {
            crate::log_error!("readStream: {}", e);
            self.set_status(pc, S_CLOSING);
            return -1;
        }
        self.raw_data.set_last_write_time(0);
        let mut was_broadcasting = false;
        let r: super::error::Result<()> = (|| {
            while self.thread.active() && !pc.is_quitting() {
                if self.check_idle(pc) {
                    crate::log_debug!("Channel idle");
                    break;
                }
                if self.check_bump() {
                    crate::log_debug!("Channel bumped");
                    error = -1;
                    break;
                }
                if input.eof()? {
                    crate::log_debug!("Channel eof");
                    break;
                }
                if input.read_ready(sys::idle_sleep_time()) {
                    error = source.read_packet(pc, ch, input)?;
                    if error != 0 {
                        break;
                    }
                    if self.raw_data.write_pos() > 0 {
                        if self.is_broadcasting() {
                            if sys::get_time().wrapping_sub(self.st().last_tracker_update) >= 120 {
                                self.broadcast_tracker_update(pc, &[0; 16], false);
                            }
                            was_broadcasting = true;
                        } else {
                            if !self.is_receiving() {
                                let mut msg = self.st().info.name.data.clone();
                                msg.extend_from_slice("を受信中です。".as_bytes());
                                pc.notify_message(super::notif::NT_PEERCAST, &msg);
                            }
                            self.set_status(pc, S_RECEIVING);
                        }
                        source.update_status(pc, ch);
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            if e.is_stream() {
                crate::log_error!("readStream: {}", e);
                error = -1;
            } else {
                crate::log_error!("readStream: {}", e);
                error = -1;
            }
        }
        self.set_status(pc, S_CLOSING);
        if was_broadcasting {
            self.broadcast_tracker_update(pc, &[0; 16], true);
        }
        error
    }
}

/// `Channel::stream`: チャンネルのスレッドの本体
fn stream_proc(pc: &Arc<Peercast>, ch: &Arc<Channel>) {
    while ch.thread.active() && !pc.is_quitting() {
        crate::log_info!("Channel started");
        let info = ch.info();
        if !pc.chanmgr.has_hitlist(&info) {
            pc.chanmgr.add_hit_list(&info);
        }
        let source = ch.st().source.clone();
        match source {
            Some(src) => sources::stream(pc, ch, &src),
            None => crate::log_error!("Channel has no source"),
        }
        crate::log_info!("Channel stopped");
        if !ch.st().stay_connected {
            break;
        }
        let diff = {
            let mut st = ch.st();
            if st.info.last_play_end == 0 {
                st.info.last_play_end = sys::get_time();
            }
            sys::get_time().wrapping_sub(st.info.last_play_end).wrapping_add(5)
        };
        for i in 0..diff {
            if !ch.thread.active() || pc.is_quitting() {
                break;
            }
            if i == 0 {
                crate::log_debug!("Channel sleeping for {} seconds", diff);
            }
            sys::sleep(1000);
        }
    }
    // endThread
    ch.reset();
    pc.chanmgr.delete_channel(ch);
    ch.thread.shutdown();
}

/// `PeercastSource::pickFromHitList`
pub fn pick_from_hit_list(pc: &Peercast, ch: &Channel, old: &ChanHit) -> ChanHit {
    let mut res = old.clone();
    let info = ch.info();
    let sid = pc.servmgr.session_id;
    let server_host = pc.servmgr.settings().server_host;
    let mut searches: Vec<ChanHitSearch> = Vec::new();
    let mk = |match_host: Option<Host>, wait: u32, trackers: bool| ChanHitSearch {
        match_host: match_host.unwrap_or_else(Host::none),
        wait_delay: wait,
        exclude_id: sid,
        trackers_only: trackers,
        ..Default::default()
    };
    searches.push(mk(Some(server_host), super::servmgr::MIN_RELAY_RETRY, false));
    searches.push(mk(None, super::servmgr::MIN_RELAY_RETRY, false));
    searches.push(mk(Some(server_host), super::servmgr::MIN_TRACKER_RETRY, true));
    searches.push(mk(None, super::servmgr::MIN_TRACKER_RETRY, true));
    pc.chanmgr.with_hitlist(&info, |chl| {
        for (i, mut chs) in searches.into_iter().enumerate() {
            if i > 0 && res.host.ip.is_set() {
                break;
            }
            if chl.pick_hits(&mut chs) {
                res = chs.best[0].clone();
            }
        }
    });
    res
}

/// ヒットの一覧から、アドレスが `h` のものを探す (`forEachHit`)
pub fn find_hit(pc: &Peercast, info: &ChanInfo, h: &Host) -> Option<ChanHit> {
    pc.chanmgr.with_hitlist(info, |chl| chl.hits.iter().find(|x| x.host == *h).cloned()).flatten()
}

impl Clone for ChanState {
    fn clone(&self) -> Self {
        ChanState {
            mount: self.mount.clone(),
            head_pack: self.head_pack.clone(),
            stream_index: self.stream_index,
            info: self.info.clone(),
            source_host: self.source_host.clone(),
            designated_host: self.designated_host.clone(),
            remote_id: self.remote_id,
            source_url: self.source_url.clone(),
            bump: self.bump,
            stay_connected: self.stay_connected,
            icy_meta_interval: self.icy_meta_interval,
            stream_pos: self.stream_pos,
            read_delay: self.read_delay,
            ty: self.ty,
            src_type: self.src_type,
            source: self.source.clone(),
            last_idle_time: self.last_idle_time,
            status: self.status,
            last_tracker_update: self.last_tracker_update,
            last_meta_update: self.last_meta_update,
            start_time: self.start_time,
            ip_version: self.ip_version,
            root_host: self.root_host.clone(),
            sock_host: self.sock_host,
        }
    }
}

#[allow(dead_code)]
fn _uses(_: &dyn Stream, _: PcString) {}

#[allow(unused_imports)]
use PCP_QUIT as _PCP_QUIT;
