//! 接続 (core/common/servent.cpp の `Servent`)。HTTP の要求の処理 (servhs.cpp) は `super::servent_http`。
//!
//! ソケットは、その接続のスレッドだけが持つ (`Conn`)。ほかのスレッドが使うもの (状態、送るパケット、
//! 接続を切るためのもの、読み書きの量) は `Servent` に置く。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::chanhit::{ChanHit, ChanHitSearch};
use super::chaninfo::{self as ci, ChanInfo};
use super::cookie::Cookie;
use super::error::{Error, Result};
use super::host::{Host, Ip};
use super::http::{Http, HTTP_SC_OK, HTTP_SC_NOTFOUND, HTTP_SC_UNAVAILABLE, MIME_MP3, MIME_XPCP, PCX_AGENT};
use super::packetbuf::{self as pb, ChanPacket};
use super::pcpconst::*;
use super::pcpstream::{PcpShared, PcpStream};
use super::pcstr::{PcString, StrType};
use super::peercast::Peercast;
use super::servfilter as sf;
use super::socket::{ClientSocket, Closer, ServerSocket};
use super::state::{b, obj, s, Value};
use super::stream::{Stat, Stream, StreamExt, WriteBufferedStream};
use super::sys;
use crate::pcp::write::AtomBuf;

// `Servent::TYPE`
pub const T_NONE: i32 = 0;
pub const T_INCOMING: i32 = 1;
pub const T_SERVER: i32 = 2;
pub const T_RELAY: i32 = 3;
pub const T_DIRECT: i32 = 4;
pub const T_COUT: i32 = 5;
pub const T_CIN: i32 = 6;
pub const T_COMMAND: i32 = 7;

const TYPE_MSGS: [&str; 8] = ["NONE", "INCOMING", "SERVER", "RELAY", "DIRECT", "COUT", "CIN", "COMMAND"];

// `Servent::STATUS`
pub const S_NONE: i32 = 0;
pub const S_CONNECTING: i32 = 1;
pub const S_PROTOCOL: i32 = 2;
pub const S_HANDSHAKE: i32 = 3;
pub const S_CONNECTED: i32 = 4;
pub const S_CLOSING: i32 = 5;
pub const S_LISTENING: i32 = 6;
pub const S_TIMEOUT: i32 = 7;
pub const S_REFUSED: i32 = 8;
pub const S_VERIFIED: i32 = 9;
pub const S_ERROR: i32 = 10;
pub const S_WAIT: i32 = 11;
pub const S_FREE: i32 = 12;

const STATUS_MSGS: [&str; 13] =
    ["NONE", "CONNECTING", "PROTOCOL", "HANDSHAKE", "CONNECTED", "CLOSING", "LISTENING", "TIMEOUT", "REFUSED", "VERIFIED", "ERROR", "WAIT", "FREE"];

// `Servent::ALLOW`
pub const ALLOW_HTML: u32 = 0x01;
pub const ALLOW_BROADCAST: u32 = 0x02;
pub const ALLOW_NETWORK: u32 = 0x04;
pub const ALLOW_DIRECT: u32 = 0x08;
pub const ALLOW_ALL: u32 = 0xff;

const DIRECT_WRITE_TIMEOUT: u32 = 60;

/// `StreamRequestDenialReason`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    None,
    InsufficientBandwidth,
    PerChannelRelayLimit,
    RelayLimit,
    DirectLimit,
    NotPlaying,
    Other,
}

impl Denial {
    fn name(self) -> &'static str {
        match self {
            Denial::None => "None",
            Denial::InsufficientBandwidth => "InsufficientBandwidth",
            Denial::PerChannelRelayLimit => "PerChannelRelayLimit",
            Denial::RelayLimit => "RelayLimit",
            Denial::DirectLimit => "DirectLimit",
            Denial::NotPlaying => "NotPlaying",
            Denial::Other => "Other",
        }
    }
}

/// `Servent` のメンバー
#[derive(Clone, Debug)]
pub struct ServentState {
    pub ty: i32,
    pub status: i32,
    pub last_connect: u32,
    pub last_ping: u32,
    pub last_packet: u32,
    pub agent: PcString,
    pub network_id: [u8; 16],
    pub remote_id: [u8; 16],
    pub chan_id: [u8; 16],
    pub giv_id: [u8; 16],
    pub login_password: PcString,
    pub login_mount: PcString,
    pub priority_connect: bool,
    pub add_metadata: bool,
    pub allow: u32,
    pub send_header: bool,
    pub sync_pos: u32,
    pub stream_pos: u32,
    pub serv_port: i32,
    pub output_protocol: i32,
    pub cookie: Cookie,
    /// `sock->host` (ソケットがなければ `Host(0, 0)`)
    pub sock_host: Host,
    pub ssl: bool,
}

impl ServentState {
    fn reset(&mut self) {
        self.remote_id = [0; 16];
        self.serv_port = 0;
        self.network_id = [0; 16];
        self.chan_id = [0; 16];
        self.output_protocol = ci::SP_UNKNOWN;
        self.agent.clear();
        self.allow = ALLOW_ALL;
        self.sync_pos = 0;
        self.add_metadata = false;
        self.last_connect = 0;
        self.last_ping = 0;
        self.last_packet = 0;
        self.login_password.clear();
        self.login_mount.clear();
        self.priority_connect = false;
        self.send_header = true;
        self.status = S_NONE;
        self.ty = T_NONE;
        self.stream_pos = 0;
        self.cookie = Cookie::empty();
        self.sock_host = Host::v4(0, 0);
        self.ssl = false;
    }
}

/// `Servent`
pub struct Servent {
    pub index: i32,
    st: Mutex<ServentState>,
    pub thread: sys::ThreadFlag,
    /// 送るパケット (PCP の接続のとき)
    pcp: Mutex<Option<Arc<PcpShared>>>,
    closer: Mutex<Option<Closer>>,
    stat: Mutex<Option<Arc<Stat>>>,
    pub push_sock: Mutex<Option<ClientSocket>>,
    has_sock: AtomicBool,
}

impl Servent {
    pub fn new(index: i32) -> Servent {
        let mut st = ServentState {
            ty: T_NONE,
            status: S_NONE,
            last_connect: 0,
            last_ping: 0,
            last_packet: 0,
            agent: PcString::default(),
            network_id: [0; 16],
            remote_id: [0; 16],
            chan_id: [0; 16],
            giv_id: [0; 16],
            login_password: PcString::default(),
            login_mount: PcString::default(),
            priority_connect: false,
            add_metadata: false,
            allow: ALLOW_ALL,
            send_header: true,
            sync_pos: 0,
            stream_pos: 0,
            serv_port: 0,
            output_protocol: ci::SP_UNKNOWN,
            cookie: Cookie::empty(),
            sock_host: Host::v4(0, 0),
            ssl: false,
        };
        st.reset();
        Servent {
            index,
            st: Mutex::new(st),
            thread: sys::ThreadFlag::new(),
            pcp: Mutex::new(None),
            closer: Mutex::new(None),
            stat: Mutex::new(None),
            push_sock: Mutex::new(None),
            has_sock: AtomicBool::new(false),
        }
    }

    pub fn st(&self) -> MutexGuard<'_, ServentState> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn ty(&self) -> i32 {
        self.st().ty
    }

    pub fn status(&self) -> i32 {
        self.st().status
    }

    pub fn last_connect(&self) -> u32 {
        self.st().last_connect
    }

    pub fn is_connected(&self) -> bool {
        self.status() == S_CONNECTED
    }

    pub fn has_sock(&self) -> bool {
        self.has_sock.load(Ordering::SeqCst)
    }

    pub fn sock_stat(&self) -> Option<Arc<Stat>> {
        self.stat.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn type_str(&self) -> &'static str {
        TYPE_MSGS.get(self.ty() as usize).copied().unwrap_or("NONE")
    }

    pub fn status_str(&self) -> &'static str {
        STATUS_MSGS.get(self.status() as usize).copied().unwrap_or("NONE")
    }

    /// ソケットを持ったことを記録する (ほかのスレッドから切ったり量を見たりするため)
    pub fn attach(&self, sock: &ClientSocket) {
        *self.closer.lock().unwrap_or_else(|e| e.into_inner()) = Some(sock.closer());
        *self.stat.lock().unwrap_or_else(|e| e.into_inner()) = Some(sock.shared_stat());
        {
            let mut st = self.st();
            st.sock_host = sock.host;
            st.ssl = sock.is_tls();
        }
        self.has_sock.store(true, Ordering::SeqCst);
    }

    /// ソケットを手放したことを記録する
    pub fn detach(&self) {
        *self.closer.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.stat.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.st().sock_host = Host::v4(0, 0);
        self.has_sock.store(false, Ordering::SeqCst);
    }

    pub fn set_pcp(&self, p: Option<Arc<PcpShared>>) {
        *self.pcp.lock().unwrap_or_else(|e| e.into_inner()) = p;
    }

    /// `reset`
    pub fn reset(&self) {
        self.st().reset();
        self.set_pcp(None);
        *self.push_sock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.detach();
    }

    /// `kill`: スレッドの終わり
    pub fn kill(&self) {
        self.thread.shutdown();
        self.set_status(S_CLOSING);
        self.set_pcp(None);
        if let Some(c) = self.closer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            c.close();
        }
        self.detach();
        *self.push_sock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if self.ty() != T_SERVER {
            self.reset();
            self.set_status(S_FREE);
        }
    }

    /// `abort`: スレッドを止め、接続を切る
    pub fn abort(&self) {
        self.thread.shutdown();
        if let Some(c) = self.closer.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            c.close();
        }
    }

    /// `setStatus`
    pub fn set_status(&self, s: i32) {
        let mut st = self.st();
        if s != st.status {
            st.status = s;
            if s == S_HANDSHAKE || s == S_CONNECTED || s == S_LISTENING {
                st.last_connect = sys::get_time();
            }
        }
    }

    /// `getHost`
    pub fn host(&self) -> Host {
        self.st().sock_host
    }

    /// `isPrivate`: フィルターで private か、自分自身
    pub fn is_private(&self, pc: &Peercast) -> bool {
        let h = self.host();
        pc.servmgr.is_filtered(pc, sf::F_PRIVATE, &h) || is_localhost(&h)
    }

    /// `isAllowed`
    pub fn is_allowed(&self, pc: &Peercast, a: u32) -> bool {
        let h = self.host();
        if pc.servmgr.is_filtered(pc, sf::F_BAN, &h) {
            return false;
        }
        self.st().allow & a != 0
    }

    /// `isFiltered`
    pub fn is_filtered(&self, pc: &Peercast, f: u32) -> bool {
        let h = self.host();
        pc.servmgr.is_filtered(pc, f, &h)
    }

    /// `sendPacket`
    pub fn send_packet(&self, pack: &ChanPacket, cid: &[u8; 16], sid: &[u8; 16], did: &[u8; 16], t: i32) -> bool {
        let (ty, status, chan, remote) = {
            let st = self.st();
            (st.ty, st.status, st.chan_id, st.remote_id)
        };
        if ty == t && status == S_CONNECTED && (!ci::is_set(cid) || chan == *cid) && (!ci::is_set(sid) || *sid != remote) {
            if let Some(p) = self.pcp.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return p.send_packet(pack, did);
            }
        }
        false
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

    /// `getState`
    pub fn state(&self, pc: &Peercast) -> Value {
        let st = self.st().clone();
        let stat = self.sock_stat();
        let (rate, avg) = match &stat {
            Some(s) => (s.bytes_in_per_sec().wrapping_add(s.bytes_out_per_sec()), s.bytes_in_per_sec_avg().wrapping_add(s.bytes_out_per_sec_avg())),
            None => (0, 0),
        };
        obj(vec![
            ("id", s(self.index.to_string())),
            ("type", s(self.type_str())),
            ("status", s(self.status_str())),
            ("address", s(st.sock_host.ip.str())),
            ("endpoint", s(st.sock_host.str())),
            ("agent", s(&st.agent.data)),
            ("bitrate", s(format!("{:.1}", super::stats::bytes_to_kbps(rate) as f64))),
            ("bitrateAvg", s(format!("{:.1}", super::stats::bytes_to_kbps(avg) as f64))),
            (
                "uptime",
                s(if st.last_connect != 0 { crate::pcstring::from_stopwatch(sys::get_time().wrapping_sub(st.last_connect)) } else { b"-".to_vec() }),
            ),
            ("chanID", s(ci::id_str(&st.chan_id))),
            ("remoteID", s(ci::id_str(&st.remote_id))),
            ("isPrivate", s(if self.is_private(pc) { "1" } else { "0" })),
            ("ssl", b(st.ssl)),
        ])
    }
}

/// `Host::isLocalhost`: ループバックか、自分のインターフェースのアドレス
pub fn is_localhost(h: &Host) -> bool {
    h.loopback_ip() || h.ip == Ip::from_v4(super::host::get_ip(&sys::hostname()))
}

// ---------------------------------------------------------------- スレッドの始め方

/// 接続のスレッドの中で使うもの
pub struct Conn<'a> {
    pub pc: &'a Arc<Peercast>,
    pub sv: &'a Arc<Servent>,
    pub sock: Option<ClientSocket>,
}

impl<'a> Conn<'a> {
    pub fn sock(&mut self) -> Result<&mut ClientSocket> {
        self.sock.as_mut().ok_or_else(|| Error::stream("Not connected"))
    }

    pub fn set_sock(&mut self, s: ClientSocket) {
        self.sv.attach(&s);
        self.sock = Some(s);
    }

    /// ソケットをほかに渡す (`sock = nullptr`)
    pub fn take_sock(&mut self) -> Option<ClientSocket> {
        self.sv.detach();
        self.sock.take()
    }
}

fn spawn(pc: &Arc<Peercast>, sv: &Arc<Servent>, name: &str, sock: Option<ClientSocket>, f: fn(&mut Conn)) -> bool {
    let pc2 = pc.clone();
    let sv2 = sv.clone();
    sys::start_thread(&sv.thread, name, move || {
        let mut c = Conn { pc: &pc2, sv: &sv2, sock };
        sys::catch_panic("Servent", || f(&mut c));
        if let Some(mut s) = c.sock.take() {
            s.close();
        }
        sv2.kill();
    })
}

/// `initServer`: 待ち受けを始める
pub fn init_server(pc: &Arc<Peercast>, sv: &Arc<Servent>, h: Host) -> bool {
    sv.set_status(S_WAIT);
    let listener = match ServerSocket::bind(h) {
        Ok(l) => l,
        Err(e) => {
            crate::log_error!("Bad server: {}", e);
            sv.kill();
            return false;
        }
    };
    {
        let mut st = sv.st();
        st.ty = T_SERVER;
        st.sock_host = h;
    }
    sv.has_sock.store(true, Ordering::SeqCst);
    let pc2 = pc.clone();
    let sv2 = sv.clone();
    let ok = sys::start_thread(&sv.thread, "LISTEN", move || {
        sys::catch_panic("Server", || server_proc(&pc2, &sv2, listener));
        sv2.kill();
    });
    if !ok {
        crate::log_error!("Bad server: Can`t start thread");
        sv.kill();
    }
    ok
}

/// `initIncoming`: 受け付けた接続の処理を始める
pub fn init_incoming(pc: &Arc<Peercast>, sv: &Arc<Servent>, sock: ClientSocket, allow: u32) {
    {
        let mut st = sv.st();
        st.ty = T_INCOMING;
        st.allow = allow;
    }
    sv.attach(&sock);
    sv.set_status(S_PROTOCOL);
    crate::log_debug!("Incoming from {}", sock.host.str());
    if !spawn(pc, sv, "INCOMING", Some(sock), incoming_proc) {
        sv.kill();
        crate::log_error!("INCOMING FAILED: Can`t start thread");
    }
}

/// `initOutgoing`: COUT を始める
pub fn init_outgoing(pc: &Arc<Peercast>, sv: &Arc<Servent>, ty: i32) {
    if sv.has_sock() || sv.thread.active() {
        crate::log_error!("Unable to start outgoing: Socket already set");
        return;
    }
    sv.st().ty = ty;
    if !spawn(pc, sv, "COUT", None, outgoing_proc) {
        crate::log_error!("Unable to start outgoing: Can`t start thread");
        sv.kill();
    }
}

/// `initGIV`: 相手につないで GIV を送り、要求を受け付ける
pub fn init_giv(pc: &Arc<Peercast>, sv: &Arc<Servent>, h: Host, id: [u8; 16]) {
    sv.st().giv_id = id;
    let r = (|| -> Result<ClientSocket> {
        sv.st().sock_host = h;
        if !sv.is_allowed(pc, ALLOW_NETWORK) {
            return Err(Error::stream("Servent not allowed"));
        }
        let mut sock = ClientSocket::new();
        sock.connect(h)?;
        Ok(sock)
    })();
    match r {
        Ok(sock) => {
            sv.attach(&sock);
            sv.st().ty = T_RELAY;
            if !spawn(pc, sv, "Servent GIV", Some(sock), giv_proc) {
                crate::log_error!("GIV error to {}: Can`t start thread", h.str());
                sv.kill();
            }
        }
        Err(e) => {
            crate::log_error!("GIV error to {}: {}", h.str(), e);
            sv.kill();
        }
    }
}

// ---------------------------------------------------------------- PCP のハンドシェイク

/// `writeHeloAtom`
pub fn write_helo_atom(out: &mut AtomBuf, send_port: bool, send_ping: bool, send_bcid: bool, session_id: &[u8; 16], port: u16, broadcast_id: &[u8; 16]) {
    out.parent(PCP_HELO, 3 + send_port as i32 + send_ping as i32 + send_bcid as i32);
    out.string(PCP_HELO_AGENT, PCX_AGENT.as_bytes());
    out.int(PCP_HELO_VERSION, PCP_CLIENT_VERSION as i32);
    out.bytes(PCP_HELO_SESSIONID, session_id);
    if send_port {
        out.short(PCP_HELO_PORT, port as i16);
    }
    if send_ping {
        out.short(PCP_HELO_PING, port as i16);
    }
    if send_bcid {
        out.bytes(PCP_HELO_BCID, broadcast_id);
    }
}

fn write_quit(io: &mut dyn Stream, code: i32) -> Result<()> {
    let mut out = AtomBuf::default();
    out.int(PCP_QUIT, code);
    io.write(&out.0)
}

fn read_hello(pc: &Peercast, io: &mut dyn Stream, kind: crate::pcp::handshake::Kind, init_sid: [u8; 16]) -> (crate::pcp::handshake::Hello, Option<Error>) {
    let mut r = super::stream::StreamReader::new(io);
    let mut atom = crate::pcp::atom::AtomStream::new(crate::pcp::handshake::StreamIo { r: &mut r });
    let mut h = crate::pcp::handshake::Hello { ping_sid_init: init_sid, ..Default::default() };
    let mut log = |m: &[u8]| super::log::add_log(super::log::Level::Debug, m);
    let res = crate::pcp::handshake::read_hello(&mut atom, kind, &pc.servmgr.session_id, &mut log, &mut h);
    let err = match res {
        Ok(()) => None,
        Err(crate::pcp::Error::Abort) => Some(r.take_error()),
        Err(crate::pcp::Error::Stream(m)) => Some(Error::stream(m)),
    };
    (h, err)
}

fn to_ip(a: &crate::pcp::atom::Ip) -> Ip {
    match a {
        crate::pcp::atom::Ip::V4(v) => Ip::from_v4(*v),
        crate::pcp::atom::Ip::V6(b) => Ip(*b),
    }
}

/// `handshakeOutgoingPCP`: helo を送り、oleh を読む。相手のセッション ID とエージェント名を返す
pub fn handshake_outgoing_pcp(pc: &Arc<Peercast>, io: &mut dyn Stream, rhost: Host, trusted: bool) -> Result<([u8; 16], PcString)> {
    let sm = &pc.servmgr;
    let ipv = if rhost.ip.is_ipv4_mapped() { 4 } else { 6 };
    let fw = sm.get_firewall(ipv);
    let send_bcid = trusted && pc.chanmgr.is_broadcasting();
    let send_port = if sm.flags.get("sendPortAtomWhenFirewallUnknown") { fw != super::servmgr::FW_ON } else { fw == super::servmgr::FW_OFF };
    let test_fw = fw == super::servmgr::FW_UNKNOWN;
    let port = sm.settings().server_host.port;
    let mut out = AtomBuf::default();
    write_helo_atom(&mut out, send_port, test_fw, send_bcid, &sm.session_id, port, &pc.chanmgr.broadcast_id());
    io.write(&out.0)?;
    crate::log_debug!("PCP outgoing waiting for OLEH..");

    let (h, err) = read_hello(pc, io, crate::pcp::handshake::Kind::Oleh, [0; 16]);
    if let Some(id) = h.unexpected {
        crate::log_debug!("PCP outgoing reply: {}", String::from_utf8_lossy(crate::pcp::atom::id_str(&id)));
        write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_BADRESPONSE)?;
        return Err(Error::stream("Got unexpected PCP response"));
    }
    let mut rid = [0u8; 16];
    let mut agent = PcString::default();
    let mut this_host = Host::none();
    let mut disable = 0;
    if h.header_ok {
        if let Some(a) = &h.agent {
            agent.assign(a);
        }
        if let Some(ip) = &h.remote_ip {
            this_host.ip = to_ip(ip);
        }
        if let Some(p) = h.port {
            this_host.port = p as u16;
        }
        if let Some(d) = h.disable {
            disable = d;
        }
        if let Some(s) = h.session_id {
            rid = s;
        }
    }
    if let Some(e) = err {
        return Err(e);
    }

    if trusted {
        if this_host.is_valid() {
            let (sip, fip) = {
                let s = sm.settings();
                (s.server_host.ip, s.force_ip.is_empty())
            };
            if sip != this_host.ip && fip {
                crate::log_debug!("Got new ip: {}", this_host.str());
                sm.update_ip_address(this_host.ip);
            }
            if sm.get_firewall(ipv) == super::servmgr::FW_UNKNOWN {
                if this_host.port != 0 && this_host.global_ip() {
                    sm.set_firewall(ipv, super::servmgr::FW_OFF);
                } else {
                    sm.set_firewall(ipv, super::servmgr::FW_ON);
                }
            }
        }
        if disable == 1 {
            crate::log_error!("client disabled: {}", disable);
            sm.settings().is_disabled = true;
        } else {
            sm.settings().is_disabled = false;
        }
    }
    if !ci::is_set(&rid) {
        write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_NOTIDENTIFIED)?;
        return Err(Error::stream("Remote host not identified"));
    }
    crate::log_debug!("PCP Outgoing handshake complete.");
    Ok((rid, agent))
}

/// `continuationPacketSupportStatus`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}

/// エージェント名から、続きのパケットに対応しているか
pub fn continuation_packet_support(agent: &[u8]) -> Support {
    use super::regex::Regex;
    let caps = |p: &str| Regex::new(p.as_bytes()).map(|r| r.exec(agent)).unwrap_or_default();
    let m = caps("^PeerCastStation/([0-9.]+)$");
    if !m.is_empty() {
        let ver: Vec<i32> = crate::strutil::split(&m[1], b".").iter().map(|x| crate::http::atoi(x)).collect();
        return if ver >= vec![2, 8, 0] { Support::Supported } else { Support::Unsupported };
    }
    let m = caps("^PeerCast/0.1218 \\(YT(\\d+)\\)$");
    if !m.is_empty() {
        return if crate::http::atoi(&m[1]) < 15 { Support::Unsupported } else { Support::Supported };
    }
    let m = caps("^PeerCast/0.1218\\(IM(\\d+)\\)$");
    if !m.is_empty() {
        return if crate::http::atoi(&m[1]) <= 51 { Support::Unsupported } else { Support::Unknown };
    }
    let m = caps("^PeerCast/0.1218\\(VP(\\d+)\\)$");
    if !m.is_empty() {
        return if crate::http::atoi(&m[1]) <= 27 { Support::Unsupported } else { Support::Unknown };
    }
    if agent == b"PeerCast/0.1218-J" || agent == b"PeerCast/0.1218" {
        return Support::Unsupported;
    }
    Support::Unknown
}

/// `pingHost`: 相手の待ち受けのポートにつないで、セッション ID が同じか確かめる。だめなら port を 0 に
pub fn ping_host(pc: &Arc<Peercast>, rhost: &mut Host, rsid: &[u8; 16]) -> bool {
    let ipstr = rhost.str();
    crate::log_debug!("Ping host {}: trying..", ipstr);
    let mut ok = false;
    let mut sock = ClientSocket::new();
    sock.set_read_timeout(15000);
    sock.set_write_timeout(15000);
    let r = (|| -> Result<()> {
        sock.connect(*rhost)?;
        let mut out = AtomBuf::default();
        out.int(PCP_CONNECT, if rhost.ip.is_ipv4_mapped() { 1 } else { 100 });
        out.parent(PCP_HELO, 1);
        out.bytes(PCP_HELO_SESSIONID, &pc.servmgr.session_id);
        sock.write(&out.0)?;
        let (h, err) = read_hello(pc, &mut sock, crate::pcp::handshake::Kind::Ping, [0; 16]);
        let sid = h.session_id.unwrap_or([0; 16]);
        if let Some(e) = err {
            return Err(e);
        }
        if let Some(id) = h.unexpected {
            crate::log_debug!("Ping response: {}", String::from_utf8_lossy(crate::pcp::atom::id_str(&id)));
            return Err(Error::stream("Bad ping response"));
        }
        if sid != *rsid {
            return Err(Error::stream("SIDs don`t match"));
        }
        ok = true;
        crate::log_debug!("Ping host {}: OK", ipstr);
        write_quit(&mut sock, PCP_ERROR_QUIT)
    })();
    if let Err(e) = r {
        crate::log_debug!("Ping host {}: {}", ipstr, e);
    }
    sock.close();
    if !ok {
        rhost.port = 0;
    }
    true
}

/// `handshakeIncomingPCP`: 相手の helo を読み、oleh を返す。相手のホストの port と、セッション ID、
/// エージェント名を書き換える
pub fn handshake_incoming_pcp(pc: &Arc<Peercast>, io: &mut dyn Stream, rhost: &mut Host, rid: &mut [u8; 16], agent: &mut PcString) -> Result<()> {
    let sm = &pc.servmgr;
    let (h, err) = read_hello(pc, io, crate::pcp::handshake::Kind::Helo, [0; 16]);
    if let Some(id) = h.unexpected {
        crate::log_debug!("PCP incoming reply: {}", String::from_utf8_lossy(crate::pcp::atom::id_str(&id)));
        write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_BADRESPONSE)?;
        return Err(Error::stream("Got unexpected PCP response"));
    }
    let mut version = 0;
    let mut ping_port = 0;
    if h.header_ok {
        rhost.port = 0;
        if let Some(a) = &h.agent {
            agent.assign(a);
        }
        if let Some(v) = h.version {
            version = v;
        }
        if let Some(s) = h.session_id {
            *rid = s;
        }
        if let Some(p) = h.port {
            rhost.port = p as u16;
        }
        if let Some(p) = h.ping {
            ping_port = p;
        }
    }
    if let Some(e) = err {
        return Err(e);
    }
    if version != 0 {
        crate::log_debug!("Incoming PCP is {} : v{}", String::from_utf8_lossy(&agent.data), version);
    }
    let sh = sm.settings().server_host;
    if !rhost.global_ip() && sh.global_ip() {
        rhost.ip = sh.ip;
    }
    if ping_port != 0 {
        crate::log_debug!("Incoming firewalled test request: {} ", rhost.str());
        rhost.port = ping_port as u16;
        if !rhost.global_ip() || !ping_host(pc, rhost, rid) {
            rhost.port = 0;
        }
    }
    let mut out = AtomBuf::default();
    out.parent(PCP_OLEH, 5);
    out.string(PCP_HELO_AGENT, PCX_AGENT.as_bytes());
    out.bytes(PCP_HELO_SESSIONID, &sm.session_id);
    out.int(PCP_HELO_VERSION, PCP_CLIENT_VERSION as i32);
    out.address(PCP_HELO_REMOTEIP, &rhost.ip.0);
    out.short(PCP_HELO_PORT, rhost.port as i16);
    io.write(&out.0)?;
    if version != 0 && version < PCP_CLIENT_MINVERSION {
        write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_BADAGENT)?;
        return Err(Error::stream("Agent is not valid"));
    }
    if sm.flags.get("requireContinuationPacketSupportFromPeer") {
        match continuation_packet_support(&agent.data) {
            Support::Unsupported => {
                crate::log_debug!("requireContinuationPacketSupportFromPeer: Denied {} {}", String::from_utf8_lossy(&agent.data), rhost.str());
                write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_BADAGENT)?;
                return Err(Error::stream("Agent is not valid (continuation packets unsupported)"));
            }
            Support::Unknown => crate::log_warn!(
                "requireContinuationPacketSupportFromPeer: Allowing agent of unknown status {}",
                String::from_utf8_lossy(&crate::inspect::inspect(&agent.data))
            ),
            Support::Supported => {
                crate::log_debug!("requireContinuationPacketSupportFromPeer: Allowed {} {}", String::from_utf8_lossy(&agent.data), rhost.str())
            }
        }
    }
    if !ci::is_set(rid) {
        write_quit(io, PCP_ERROR_QUIT + PCP_ERROR_NOTIDENTIFIED)?;
        return Err(Error::stream("Remote host not identified"));
    }
    if sm.is_root() {
        let mut out = AtomBuf::default();
        sm.write_root_atoms(pc, &mut out, false);
        io.write(&out.0)?;
    }
    crate::log_debug!("PCP Incoming handshake complete.");
    Ok(())
}

/// ヒットを最大 8 個書く (中継できないときに知らせるほかのホスト)
fn search_hits(pc: &Arc<Peercast>, chan: Option<&ChanInfo>, rhost: &Host, remote_id: &[u8; 16], trackers: bool, use_busy_controls: bool) -> Vec<ChanHit> {
    let server_host = pc.servmgr.settings().server_host;
    let mut found = Vec::new();
    for _ in 0..8 {
        let mut best: Option<ChanHit> = None;
        let mut tries: Vec<ChanHitSearch> = Vec::new();
        let mk = |m: Option<Host>| ChanHitSearch {
            match_host: m.unwrap_or_else(Host::none),
            wait_delay: 2,
            exclude_id: *remote_id,
            trackers_only: trackers,
            use_busy_controls,
            ..Default::default()
        };
        if !rhost.global_ip() {
            tries.push(mk(Some(server_host)));
        }
        tries.push(mk(Some(*rhost)));
        tries.push(mk(None));
        for mut chs in tries {
            if best.is_some() {
                break;
            }
            let ok = match chan {
                Some(info) => pc.chanmgr.with_hitlist(info, |chl| chl.pick_hits(&mut chs)).unwrap_or(false),
                None => pc.chanmgr.pick_hits(&mut chs),
            };
            if ok {
                best = Some(chs.best[0].clone());
            }
        }
        match best {
            Some(b) if b.host.ip.is_set() => found.push(b),
            _ => break,
        }
    }
    found
}

/// `processIncomingPCP`: CIN (ほかのノードからの制御の接続)
pub fn process_incoming_pcp(c: &mut Conn, suggest_others: bool) -> Result<()> {
    let pc = c.pc;
    let sv = c.sv;
    let sm = &pc.servmgr;
    super::pcpstream::read_version(c.sock()?)?;
    let mut rhost = c.sock()?.host;
    let (mut rid, mut agent) = {
        let st = sv.st();
        (st.remote_id, st.agent.clone())
    };
    let r = handshake_incoming_pcp(pc, c.sock()?, &mut rhost, &mut rid, &mut agent);
    {
        let mut st = sv.st();
        st.remote_id = rid;
        st.agent = agent;
    }
    r?;
    let already = sm.find_connection(T_COUT, &rid).is_some() || sm.find_connection(T_CIN, &rid).is_some();
    let unavailable = sm.controls_in_full(pc);
    let offair = !sm.is_root() && !pc.chanmgr.is_broadcasting();
    let rstr = rhost.str();
    if unavailable || already || offair {
        let error = if already {
            PCP_ERROR_QUIT + PCP_ERROR_ALREADYCONNECTED
        } else if unavailable {
            PCP_ERROR_QUIT + PCP_ERROR_UNAVAILABLE
        } else {
            PCP_ERROR_QUIT + PCP_ERROR_OFFAIR
        };
        if suggest_others {
            let hits = search_hits(pc, None, &rhost, &rid, true, false);
            let mut out = AtomBuf::default();
            for h in &hits {
                h.write_atoms(&mut out, &[0; 16]);
            }
            c.sock()?.write(&out.0)?;
            if !hits.is_empty() {
                crate::log_debug!("Sent {} tracker(s) to {}", hits.len(), rstr);
            } else if rhost.port != 0 {
                let mut chs = ChanHitSearch { wait_delay: 30, exclude_id: rid, trackers_only: true, use_firewalled: true, use_busy_controls: false, ..Default::default() };
                if pc.chanmgr.pick_hits(&mut chs) {
                    let best = chs.best[0].clone();
                    let cnt = sm.broadcast_push_request(&best, &rhost, &[0; 16], T_CIN);
                    crate::log_debug!("Broadcasted tracker push request to {} clients for {}", cnt, rstr);
                }
            } else {
                crate::log_debug!("No available trackers");
            }
        }
        crate::log_error!("Sending QUIT to incoming: {}", error);
        write_quit(c.sock()?, error)?;
        return Ok(());
    }

    sv.st().ty = T_CIN;
    sv.set_status(S_CONNECTED);
    let mut out = AtomBuf::default();
    out.int(PCP_OK, 0);
    out.parent(PCP_ROOT, 1);
    out.parent(PCP_ROOT_UPDATE, 0);
    c.sock()?.write(&out.0)?;

    let mut pcp = PcpStream::new(rid);
    sv.set_pcp(Some(pcp.shared.clone()));
    let mut error = 0;
    let mut bcs = crate::pcp::BroadcastState::default();
    while error == 0 && sv.thread.active() && !c.sock()?.eof()? {
        error = pcp.read_packet(pc, c.sock()?, &mut bcs);
        sys::sleep_idle();
        if !sm.is_root() && !pc.chanmgr.is_broadcasting() {
            error = PCP_ERROR_OFFAIR;
        }
        if pc.is_quitting() {
            error = PCP_ERROR_SHUTDOWN;
        }
    }
    pcp.flush(c.sock()?)?;
    error += PCP_ERROR_QUIT;
    write_quit(c.sock()?, error)?;
    crate::log_debug!("PCP Incoming to {} closed: {}", rstr, error);
    Ok(())
}

/// `outgoingProc`: 配信中、YP (ルート) への COUT
fn outgoing_proc(c: &mut Conn) {
    let pc = c.pc;
    let sv = c.sv;
    let sm = &pc.servmgr;
    crate::log_debug!("COUT started");
    let mut pcp = PcpStream::new([0; 16]);
    sv.set_pcp(Some(pcp.shared.clone()));
    while sv.thread.active() {
        sv.set_status(S_WAIT);
        if pc.chanmgr.is_broadcasting() && sm.settings().auto_serve {
            let mut best;
            loop {
                best = ChanHit::new();
                let rh = sm.settings().root_host.data.clone();
                if rh.is_empty() {
                    break;
                }
                if let Some(ps) = sv.push_sock.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    best.host = ps.host;
                    c.set_sock(ps);
                    break;
                }
                let sid = sm.session_id;
                let server_host = sm.settings().server_host;
                let picked = pc.chanmgr.with_hitlist_by_id(&[0; 16], |chl| {
                    let mut chs = ChanHitSearch { match_host: server_host, wait_delay: MIN_TRACKER_RETRY, exclude_id: sid, trackers_only: true, ..Default::default() };
                    if !chl.pick_hits(&mut chs) {
                        chs = ChanHitSearch { wait_delay: MIN_TRACKER_RETRY, exclude_id: sid, trackers_only: true, ..Default::default() };
                        chl.pick_hits(&mut chs);
                    }
                    chs.best.first().cloned()
                });
                if let Some(Some(h)) = picked {
                    best = h;
                }
                let ctime = sys::get_time();
                if !best.host.ip.is_set() {
                    let mut cs = pc.chanmgr.settings();
                    if ctime.wrapping_sub(cs.last_yp_connect) > super::servmgr::MIN_YP_RETRY {
                        cs.last_yp_connect = ctime;
                        drop(cs);
                        best.host = Host::from_str_name(&rh, super::servmgr::DEFAULT_PORT);
                        best.yp = true;
                    }
                }
                sys::sleep_idle();
                if best.host.ip.is_set() || !sv.thread.active() {
                    break;
                }
            }
            if !best.host.ip.is_set() {
                crate::log_error!("COUT giving up");
                break;
            }
            let ip_str = best.host.str();
            let mut error = 0;
            let r = (|| -> Result<()> {
                crate::log_debug!("COUT to {}: Connecting..", ip_str);
                if c.sock.is_none() {
                    sv.set_status(S_CONNECTING);
                    let mut s = ClientSocket::new();
                    s.connect(best.host)?;
                    c.set_sock(s);
                }
                c.sock()?.set_read_timeout(30000);
                sv.set_status(S_HANDSHAKE);
                let rhost = c.sock()?.host;
                let mut out = AtomBuf::default();
                out.int(PCP_CONNECT, 1);
                c.sock()?.write(&out.0)?;
                let (rid, agent) = handshake_outgoing_pcp(pc, c.sock()?, rhost, best.yp)?;
                {
                    let mut st = sv.st();
                    st.remote_id = rid;
                    st.agent = agent;
                }
                sv.set_status(S_CONNECTED);
                crate::log_debug!("COUT to {}: OK", ip_str);
                pcp.init(rid);
                let mut bcs = crate::pcp::BroadcastState::default();
                error = 0;
                while error == 0 && sv.thread.active() && !c.sock()?.eof()? && sm.settings().auto_serve {
                    error = pcp.read_packet(pc, c.sock()?, &mut bcs);
                    sys::sleep_idle();
                    if !pc.chanmgr.is_broadcasting() {
                        error = PCP_ERROR_OFFAIR;
                    }
                    if pc.is_quitting() {
                        error = PCP_ERROR_SHUTDOWN;
                    }
                    if pcp.next_root_packet != 0 && sys::get_time() > pcp.next_root_packet.wrapping_add(30) {
                        error = PCP_ERROR_NOROOT;
                    }
                }
                sv.set_status(S_CLOSING);
                pcp.flush(c.sock()?)?;
                error += PCP_ERROR_QUIT;
                write_quit(c.sock()?, error)?;
                crate::log_error!("COUT to {} closed: {}", ip_str, error);
                Ok(())
            })();
            if let Err(e) = r {
                if e.is_timeout() {
                    crate::log_error!("COUT to {}: timeout ({})", ip_str, e);
                    sv.set_status(S_TIMEOUT);
                } else {
                    crate::log_error!("COUT to {}: {}", ip_str, e);
                    sv.set_status(S_ERROR);
                }
            }
            if let Some(mut s) = c.take_sock() {
                s.close();
            }
            if error != PCP_ERROR_QUIT + PCP_ERROR_OFFAIR {
                pc.chanmgr.dead_hit(&best);
            }
        }
        sys::sleep_idle();
    }
    crate::log_debug!("COUT ended");
}

/// `givProc`
fn giv_proc(c: &mut Conn) {
    let id = c.sv.st().giv_id;
    let r = (|| -> Result<()> {
        let s = c.sock()?;
        if ci::is_set(&id) {
            s.write_line(format!("GIV /{}", ci::id_str(&id)))?;
        } else {
            s.write_line("GIV")?;
        }
        s.write_line("")?;
        super::servent_http::handshake_incoming(c)
    })();
    if let Err(e) = r {
        crate::log_error!("GIV: {}", e);
    }
}

/// `incomingProc`
fn incoming_proc(c: &mut Conn) {
    let ip_str = c.sock.as_ref().map(|s| s.host.str()).unwrap_or_default();
    if let Err(e) = super::servent_http::handshake_incoming(c) {
        if e.is_http() {
            // HTTPException: 状態の行と本文を返す
            let r = (|| -> Result<()> {
                let s = c.sock()?;
                s.write_line(&e.msg)?;
                if e.err == 401 {
                    s.write_line("WWW-Authenticate: Basic realm=\"PeerCast\"")?;
                }
                let content = format!("{}\n\n{}", e.msg, e.detail);
                s.write_line("Content-Type: text/plain; charset=utf-8")?;
                s.write_line(format!("Content-Length: {}", content.len()))?;
                s.write_line("")?;
                s.write(content.as_bytes())
            })();
            let _ = r;
        }
        crate::log_error!("Incoming from {}: {}", ip_str, e);
    }
}

/// IP アドレスごとの、要求を読み終えていない接続の数
static HANDSHAKES: crate::servhs::HandshakeCounter = crate::servhs::HandshakeCounter::new();

/// `serverProc`: 接続を受け付けてサーバントに渡す
fn server_proc(pc: &Arc<Peercast>, sv: &Arc<Servent>, listener: ServerSocket) {
    let port = listener.host.port;
    sv.set_status(S_LISTENING);
    if pc.servmgr.is_root() {
        crate::log_info!("Root Server started on port {}", port);
    } else {
        crate::log_info!("Server started on port {}", port);
    }
    while sv.thread.active() {
        let mut cs = match listener.accept() {
            Some(cs) => cs,
            None => {
                listener.wait(100);
                continue;
            }
        };
        let (max_in, timeout, max_hs) = {
            let s = pc.servmgr.settings();
            (s.max_serv_in, s.handshake_timeout, s.max_handshakes_per_ip)
        };
        // 接続数が上限なら切る。ループバックからは、上限でも受け付ける (管理画面を開けるように)。
        // C++ 版は上限のあいだ受け付けるのをやめていたので、ゆっくり送る接続で埋められると、
        // 誰もつなげなくなっていた
        let loopback = cs.host.loopback_ip();
        if !loopback && pc.servmgr.num_active_on_port(port as i32) >= max_in {
            crate::log_debug!("Server full, closing connection from {}", cs.host.str());
            continue;
        }
        // 要求を読み終えていない接続は、IP アドレスごとに数を抑える (ループバックは数えない)
        let slot = if loopback {
            None
        } else {
            match HANDSHAKES.acquire(cs.host.ip.str().as_bytes(), max_hs) {
                Some(s) => Some(Box::new(s) as Box<dyn std::any::Any + Send>),
                None => {
                    crate::log_debug!("Too many unfinished requests from {}", cs.host.ip.str());
                    continue;
                }
            }
        };
        cs.begin_handshake(timeout.saturating_mul(1000), slot);
        crate::log_trace!("accepted incoming");
        let ns = pc.servmgr.alloc_servent();
        let (net, allow) = {
            let mut s = pc.servmgr.settings();
            s.last_incoming = sys::get_time();
            (s.network_id, sv.st().allow)
        };
        {
            let mut st = ns.st();
            st.serv_port = port as i32;
            st.network_id = net;
        }
        init_incoming(pc, &ns, cs, allow);
    }
    crate::log_info!("Server stopped: {}", sv.host().str());
}

// ---------------------------------------------------------------- ストリーム

/// `canStream`: 中継かダイレクトで流せるか
pub fn can_stream(pc: &Peercast, sv: &Servent, ch: Option<&Arc<super::channel::Channel>>) -> std::result::Result<(), Denial> {
    let ch = ch.ok_or(Denial::Other)?;
    if pc.servmgr.settings().is_disabled {
        return Err(Denial::Other);
    }
    if !sv.is_private(pc) {
        let ty = sv.ty();
        if pc.servmgr.bitrate_full(pc, ch.st().info.bitrate as u32) {
            crate::log_debug!("Unable to stream because there is not enough bandwidth left");
            return Err(Denial::InsufficientBandwidth);
        }
        if ty == T_RELAY && pc.servmgr.relays_full(pc) {
            crate::log_debug!("Unable to stream because server already has max. number of relays");
            return Err(Denial::RelayLimit);
        }
        if ty == T_DIRECT && pc.servmgr.direct_full(pc) {
            crate::log_debug!("Unable to stream because server already has max. number of directs");
            return Err(Denial::DirectLimit);
        }
        if !ch.is_playing() {
            crate::log_debug!("Unable to stream because channel is not playing");
            return Err(Denial::NotPlaying);
        }
        if ty == T_RELAY && ch.is_full(pc) {
            crate::log_debug!("Unable to stream because channel already has max. number of relays");
            return Err(Denial::PerChannelRelayLimit);
        }
    }
    Ok(())
}

/// `isTerminationCandidate`
pub fn is_termination_candidate(hit: &ChanHit) -> bool {
    (!hit.relay && hit.num_relays == 0) || (hit.host.port == 0 && !hit.can_giv())
}

/// 同時にストリームの要求を判断しない (`streamRequestMutex`)
static STREAM_REQUEST: Mutex<()> = Mutex::new(());

/// `handshakeStream`: /stream/ と /channel/ の要求に答える。流せるなら true
fn handshake_stream(c: &mut Conn, info: &ChanInfo) -> Result<bool> {
    let pc = c.pc;
    let sv = c.sv;
    // ヘッダーを読む
    let mut got_pcp = false;
    let mut req_pos: u32 = 0;
    {
        let mut http = Http::new(c.sock()?);
        while http.next_header()? {
            let arg = match http.arg_str() {
                Some(a) => a.to_vec(),
                None => continue,
            };
            if http.is_header(PCX_HS_PCP) {
                got_pcp = crate::http::atoi(&arg) != 0;
            } else if http.is_header(PCX_HS_POS) {
                req_pos = crate::http::atoi(&arg) as u32;
            } else if http.is_header("icy-metadata") {
                sv.st().add_metadata = crate::http::atoi(&arg) > 0;
            } else if http.is_header("User-Agent:") {
                sv.st().agent.assign(&arg);
            }
            crate::log_debug!("Stream: {}", String::from_utf8_lossy(&http.cmd_line));
        }
    }

    let mut chan_ready = false;
    let ch = pc.chanmgr.find_channel_by_id(&info.id);
    if let Some(ch) = &ch {
        let sp = if req_pos != 0 { ch.raw_data.find_oldest_pos(req_pos) } else { ch.raw_data.latest_pos() };
        {
            let mut st = sv.st();
            st.send_header = true;
            st.stream_pos = sp;
        }
        let mut auto_manage_tried = false;
        loop {
            let reason = {
                let _g = STREAM_REQUEST.lock().unwrap_or_else(|e| e.into_inner());
                match can_stream(pc, sv, Some(ch)) {
                    Ok(()) => {
                        chan_ready = true;
                        sv.st().chan_id = info.id;
                        sv.set_status(S_CONNECTED);
                        Denial::None
                    }
                    Err(r) => r,
                }
            };
            crate::log_debug!("chanReady = {}; reason = {}", chan_ready as i32, reason.name());
            if chan_ready {
                break;
            } else if !auto_manage_tried
                && sv.ty() == T_RELAY
                && matches!(reason, Denial::InsufficientBandwidth | Denial::RelayLimit | Denial::PerChannelRelayLimit)
            {
                auto_manage_tried = true;
                crate::log_debug!("Auto-manage relays");
                let hits = match pc.chanmgr.with_hitlist(info, |chl| chl.hits.clone()) {
                    Some(h) => h,
                    None => break,
                };
                for s in pc.servmgr.servents() {
                    if Arc::ptr_eq(&s, sv) {
                        continue;
                    }
                    let (sty, schan, srid) = {
                        let st = s.st();
                        (st.ty, st.chan_id, st.remote_id)
                    };
                    let hit = hits.iter().find(|h| h.session_id == srid);
                    if sty == T_RELAY && schan == info.id && !s.is_private(pc) {
                        if let Some(hit) = hit {
                            if is_termination_candidate(hit) {
                                crate::log_info!(
                                    "Terminating relay connection to {} (color={})",
                                    s.host().str(),
                                    ChanHit::color_name(hit.color())
                                );
                                s.abort();
                                sys::sleep(200);
                                break;
                            }
                        }
                    }
                }
            } else {
                break;
            }
            if !(sv.thread.active() && c.sock.as_ref().map_or(false, |s| s.active())) {
                break;
            }
        }
    }

    let has_chl = pc.chanmgr.has_hitlist(info);
    let rhost = c.sock()?.host;
    let out_proto = sv.st().output_protocol;
    if !has_chl {
        let s = c.sock()?;
        s.write_line(HTTP_SC_NOTFOUND)?;
        s.write_line("")?;
        crate::log_debug!("Sending channel not found");
        return Ok(false);
    }
    if !chan_ready {
        if out_proto == ci::SP_PCP {
            {
                let s = c.sock()?;
                s.write_line(HTTP_SC_UNAVAILABLE)?;
                s.write_line(format!("Content-Type: {}", MIME_XPCP))?;
                s.write_line("")?;
            }
            let mut rh = rhost;
            let (mut rid, mut agent) = {
                let st = sv.st();
                (st.remote_id, st.agent.clone())
            };
            let r = handshake_incoming_pcp(pc, c.sock()?, &mut rh, &mut rid, &mut agent);
            {
                let mut st = sv.st();
                st.remote_id = rid;
                st.agent = agent;
            }
            r?;
            crate::log_debug!("Sending channel unavailable");
            return_hits(c, info, &rh, &rid)?;
            return Ok(false);
        } else {
            crate::log_debug!("Sending channel unavailable");
            let s = c.sock()?;
            s.write_line(HTTP_SC_UNAVAILABLE)?;
            s.write_line("")?;
            return Ok(false);
        }
    }
    return_stream_headers(c, info)?;
    if got_pcp {
        let mut rh = rhost;
        let (mut rid, mut agent) = {
            let st = sv.st();
            (st.remote_id, st.agent.clone())
        };
        let r = handshake_incoming_pcp(pc, c.sock()?, &mut rh, &mut rid, &mut agent);
        {
            let mut st = sv.st();
            st.remote_id = rid;
            st.agent = agent;
        }
        r?;
        let mut out = AtomBuf::default();
        out.int(PCP_OK, 0);
        c.sock()?.write(&out.0)?;
    }
    Ok(true)
}

/// `handshakeStream_returnStreamHeaders`
fn return_stream_headers(c: &mut Conn, info: &ChanInfo) -> Result<()> {
    let pc = c.pc;
    let sv = c.sv;
    let (mut add_metadata, out_proto, stream_pos) = {
        let st = sv.st();
        (st.add_metadata, st.output_protocol, st.stream_pos)
    };
    if info.content_type.data != ci::T_MP3 {
        add_metadata = false;
        sv.st().add_metadata = false;
    }
    let icy_meta = pc.chanmgr.settings().icy_meta_interval;
    let s = c.sock()?;
    let line = |s: &mut ClientSocket, parts: &[&[u8]]| s.write_line(parts.concat());
    if add_metadata && out_proto == ci::SP_HTTP {
        s.write_line(ICY_OK)?;
        s.write_line(format!("Server: {}", PCX_AGENT))?;
        line(s, &[b"icy-name:", &info.name.data])?;
        s.write_line(format!("icy-br:{}", info.bitrate))?;
        line(s, &[b"icy-genre:", &info.genre.data])?;
        line(s, &[b"icy-url:", &info.url.data])?;
        s.write_line(format!("icy-metaint:{}", icy_meta))?;
        s.write_line(format!("{} {}", PCX_HS_CHANNELID, ci::id_str(&info.id)))?;
        s.write_line(format!("Content-Type: {}", MIME_MP3))?;
    } else {
        s.write_line(HTTP_SC_OK)?;
        s.write_line(format!("Server: {}", PCX_AGENT))?;
        s.write_line("Accept-Ranges: none")?;
        line(s, &[b"x-audiocast-name: ", &info.name.data])?;
        s.write_line(format!("x-audiocast-bitrate: {}", info.bitrate))?;
        line(s, &[b"x-audiocast-genre: ", &info.genre.data])?;
        line(s, &[b"x-audiocast-description: ", &info.desc.data])?;
        line(s, &[b"x-audiocast-url: ", &info.url.data])?;
        s.write_line(format!("{} {}", PCX_HS_CHANNELID, ci::id_str(&info.id)))?;
        if out_proto == ci::SP_HTTP {
            if info.content_type.data == ci::T_MOV {
                s.write_line("Connection: close")?;
                s.write_line("Content-Length: 10000000")?;
            }
            s.write_line("Access-Control-Allow-Origin: *")?;
            line(s, &[b"Content-Type: ", &info.mime()])?;
        } else if out_proto == ci::SP_PCP {
            s.write_line(format!("{} {}", PCX_HS_POS, stream_pos))?;
            s.write_line(format!("Content-Type: {}", MIME_XPCP))?;
        }
    }
    s.write_line("")
}

/// `handshakeStream_returnHits`: 流せないとき、ほかのホストを知らせる
fn return_hits(c: &mut Conn, info: &ChanInfo, rhost: &Host, remote_id: &[u8; 16]) -> Result<()> {
    let pc = c.pc;
    let sm = &pc.servmgr;
    let chan_id = info.id;
    let mut out = AtomBuf::default();
    if pc.chanmgr.has_hitlist(info) {
        let hits = search_hits(pc, Some(info), rhost, remote_id, false, true);
        for h in &hits {
            h.write_atoms(&mut out, &chan_id);
        }
        let mut best_found = !hits.is_empty();
        if !hits.is_empty() {
            crate::log_debug!("Sent {} channel hit(s) to {}", hits.len(), rhost.str());
        } else if rhost.port != 0 {
            let mut chs = ChanHitSearch { wait_delay: 30, use_firewalled: true, exclude_id: *remote_id, ..Default::default() };
            if let Some(true) = pc.chanmgr.with_hitlist(info, |chl| chl.pick_hits(&mut chs)) {
                let best = chs.best[0].clone();
                best_found = best.host.ip.is_set();
                let id = pc.chanmgr.with_hitlist(info, |chl| chl.info.id).unwrap_or(chan_id);
                let cnt = sm.broadcast_push_request(&best, rhost, &id, T_RELAY);
                crate::log_debug!("Broadcasted channel push request to {} clients for {}", cnt, rhost.str());
            }
        }
        if !best_found {
            // トラッカーを探す
            let server_host = sm.settings().server_host;
            let mut tries = Vec::new();
            let mk = |m: Option<Host>| ChanHitSearch { match_host: m.unwrap_or_else(Host::none), trackers_only: true, exclude_id: *remote_id, ..Default::default() };
            if !rhost.global_ip() {
                tries.push(mk(Some(server_host)));
            }
            tries.push(mk(Some(*rhost)));
            tries.push(mk(None));
            let mut best: Option<ChanHit> = None;
            for mut chs in tries {
                if best.is_some() {
                    break;
                }
                if let Some(true) = pc.chanmgr.with_hitlist(info, |chl| chl.pick_hits(&mut chs)) {
                    let b = chs.best[0].clone();
                    if b.host.ip.is_set() {
                        best = Some(b);
                    }
                }
            }
            match best {
                Some(b) => {
                    b.write_atoms(&mut out, &chan_id);
                    crate::log_debug!("Sent 1 tracker hit to {}", rhost.str());
                }
                None => {
                    if rhost.port != 0 {
                        let mut chs = ChanHitSearch {
                            use_firewalled: true,
                            trackers_only: true,
                            exclude_id: *remote_id,
                            wait_delay: 30,
                            ..Default::default()
                        };
                        if let Some(true) = pc.chanmgr.with_hitlist(info, |chl| chl.pick_hits(&mut chs)) {
                            let b = chs.best[0].clone();
                            let id = pc.chanmgr.with_hitlist(info, |chl| chl.info.id).unwrap_or(chan_id);
                            let cnt = sm.broadcast_push_request(&b, rhost, &id, T_CIN);
                            crate::log_debug!("Broadcasted tracker push request to {} clients for {}", cnt, rhost.str());
                        }
                    }
                }
            }
        }
    }
    out.int(PCP_QUIT, PCP_ERROR_QUIT + PCP_ERROR_UNAVAILABLE);
    c.sock()?.write(&out.0)
}

/// `triggerChannel`: 指定されたチャンネルを流す
pub fn trigger_channel(c: &mut Conn, s: &[u8], proto: i32, relay: bool) -> Result<()> {
    let pc = c.pc;
    let (info, _) = pc.servmgr.get_channel(pc, s, relay);
    {
        let mut st = c.sv.st();
        st.ty = if proto == ci::SP_PCP { T_RELAY } else { T_DIRECT };
        st.output_protocol = proto;
    }
    process_stream(c, &info)
}

/// `processStream`
fn process_stream(c: &mut Conn, info: &ChanInfo) -> Result<()> {
    let pc = c.pc;
    let sv = c.sv;
    sv.set_status(S_HANDSHAKE);
    if !handshake_stream(c, info)? {
        return Ok(());
    }
    if ci::is_set(&info.id) {
        sv.st().chan_id = info.id;
        let proto = sv.st().output_protocol;
        crate::log_info!("Sending channel: {} ", String::from_utf8_lossy(ci::protocol_str(proto)));
        if !wait_for_channel_header(c, info) {
            return Err(Error::stream("Channel not ready"));
        }
        pc.servmgr.total_streams.fetch_add(1, Ordering::SeqCst);
        if pc.chanmgr.find_channel_by_id(&info.id).is_none() {
            return Err(Error::stream("Channel not found"));
        }
        if proto == ci::SP_HTTP {
            let add_meta = sv.st().add_metadata;
            let icy = pc.chanmgr.settings().icy_meta_interval;
            if add_meta && icy != 0 {
                send_raw_meta_channel(c, icy);
            } else {
                send_raw_channel(c, true, true);
            }
        } else if proto == ci::SP_PCP {
            send_pcp_channel(c)?;
        }
    }
    sv.set_status(S_CLOSING);
    Ok(())
}

/// `waitForChannelHeader`: 最大 30 秒、チャンネルが受信を始めるのを待つ
fn wait_for_channel_header(c: &mut Conn, info: &ChanInfo) -> bool {
    for _ in 0..300 {
        let ch = match c.pc.chanmgr.find_channel_by_id(&info.id) {
            Some(ch) => ch,
            None => return false,
        };
        if ch.is_playing() && ch.raw_data.write_pos() > 0 {
            return true;
        }
        if !c.sv.thread.active() || !c.sock.as_ref().map_or(false, |s| s.active()) {
            break;
        }
        sys::sleep(100);
    }
    false
}

/// `sendRawChannel`: ダイレクトで中身をそのまま送る
fn send_raw_channel(c: &mut Conn, send_head: bool, send_data: bool) {
    let pc = c.pc;
    let sv = c.sv;
    let chan_id = sv.st().chan_id;
    let r = (|| -> Result<()> {
        let sock = c.sock.as_mut().ok_or_else(|| Error::stream("Not connected"))?;
        sock.set_write_timeout(DIRECT_WRITE_TIMEOUT * 1000);
        let mut bsock = WriteBufferedStream::new(sock);
        let ch = pc.chanmgr.find_channel_by_id(&chan_id).ok_or_else(|| Error::stream("Channel not found"))?;
        let name = ch.st().info.name.data.clone();
        crate::log_debug!("Starting Raw stream of {} at {}", String::from_utf8_lossy(&name), sv.st().stream_pos);
        if send_head {
            let head = ch.st().head_pack.clone();
            head.write_raw(&mut bsock)?;
            let mut sp = head.pos.wrapping_add(head.len());
            let ncpos = ch.raw_data.latest_non_continuation_pos();
            if ncpos != 0 && sp < ncpos {
                sp = ncpos;
            }
            sv.st().stream_pos = sp;
            crate::log_debug!("Sent {} bytes header ", head.len());
        }
        if send_data {
            let mut stream_index = ch.st().stream_index;
            let mut last_write_time = sys::get_time();
            let mut skip_cont = pc.servmgr.flags.get("startPlayingFromKeyFrame");
            while sv.thread.active() && bsock.inner.eof().map(|e| !e).unwrap_or(false) {
                let ch = pc.chanmgr.find_channel_by_id(&chan_id).ok_or_else(|| Error::stream("Channel not found"))?;
                let (si, hp) = {
                    let st = ch.st();
                    (st.stream_index, st.head_pack.pos)
                };
                if stream_index != si {
                    stream_index = si;
                    sv.st().stream_pos = hp;
                    crate::log_debug!("sendRaw got new stream index {}", stream_index);
                }
                loop {
                    let sp = sv.st().stream_pos;
                    let raw = match ch.raw_data.find_packet(sp) {
                        Some(p) => p,
                        None => break,
                    };
                    {
                        let mut st = sv.st();
                        if st.sync_pos != raw.sync {
                            crate::log_error!("Send skip: {}", raw.sync.wrapping_sub(st.sync_pos) as i32);
                        }
                        st.sync_pos = raw.sync.wrapping_add(1);
                    }
                    if raw.ty == pb::T_DATA || raw.ty == pb::T_HEAD {
                        if !skip_cont || !raw.cont {
                            skip_cont = false;
                            raw.write_raw(&mut bsock)?;
                            last_write_time = sys::get_time();
                        } else {
                            crate::log_debug!(
                                "raw: skip continuation {} packet pos={}",
                                if raw.ty == pb::T_DATA { "DATA" } else { "HEAD" },
                                raw.pos
                            );
                        }
                    }
                    if raw.pos < sp {
                        crate::log_debug!("raw: skip back {}", raw.pos.wrapping_sub(sp) as i32);
                    }
                    sv.st().stream_pos = raw.pos.wrapping_add(raw.len());
                }
                if sys::get_time().wrapping_sub(last_write_time) > DIRECT_WRITE_TIMEOUT {
                    return Err(Error::timeout());
                }
                bsock.flush()?;
                sys::sleep(200);
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        crate::log_error!("Stream channel: {}", e);
    }
}

/// `sendRawMetaChannel`: ICY のメタデータを挟んで送る (MP3)
fn send_raw_meta_channel(c: &mut Conn, interval: i32) {
    let pc = c.pc;
    let sv = c.sv;
    let chan_id = sv.st().chan_id;
    let r = (|| -> Result<()> {
        let mut ch = pc.chanmgr.find_channel_by_id(&chan_id).ok_or_else(|| Error::stream("Channel not found"))?;
        let sock = c.sock.as_mut().ok_or_else(|| Error::stream("Not connected"))?;
        sock.set_write_timeout(DIRECT_WRITE_TIMEOUT * 1000);
        let name = ch.st().info.name.data.clone();
        crate::log_debug!("Starting Raw Meta stream of {} (metaint: {}) at {}", String::from_utf8_lossy(&name), interval, sv.st().stream_pos);
        let mut last_title = PcString::default();
        let mut last_url = PcString::default();
        let mut last_msg_time = sys::get_time();
        let mut show_msg = true;
        if interval > 16384 || interval < 1 {
            return Err(Error::stream("Bad ICY Meta Interval value"));
        }
        let interval = interval as usize;
        let mut buf = vec![0u8; 16384];
        let mut buf_pos = 0usize;
        let mut last_write_time = sys::get_time();
        sv.st().stream_pos = 0;
        while sv.thread.active() && sock.active() {
            ch = pc.chanmgr.find_channel_by_id(&chan_id).ok_or_else(|| Error::stream("Channel not found"))?;
            let sp = sv.st().stream_pos;
            if let Some(raw) = ch.raw_data.find_packet(sp) {
                {
                    let mut st = sv.st();
                    if st.sync_pos != raw.sync {
                        crate::log_error!("Send skip: {}", raw.sync.wrapping_sub(st.sync_pos) as i32);
                    }
                    st.sync_pos = raw.sync.wrapping_add(1);
                }
                if raw.ty == pb::T_DATA {
                    let mut p = &raw.data[..];
                    while !p.is_empty() {
                        let rl = p.len().min(interval - buf_pos);
                        buf[buf_pos..buf_pos + rl].copy_from_slice(&p[..rl]);
                        buf_pos += rl;
                        p = &p[rl..];
                        if buf_pos >= interval {
                            buf_pos = 0;
                            sock.write(&buf[..interval])?;
                            last_write_time = sys::get_time();
                            let bmi = pc.chanmgr.settings().broadcast_msg_interval;
                            if bmi != 0 && sys::get_time().wrapping_sub(last_msg_time) >= bmi {
                                show_msg = !show_msg;
                                last_msg_time = sys::get_time();
                            }
                            let info = ch.info();
                            let meta_title = if !info.comment.is_empty() && show_msg { &info.comment } else { &info.track.title };
                            if meta_title.data != last_title.data || info.url.data != last_url.data {
                                let mut title = meta_title.clone();
                                let mut url = info.url.clone();
                                title.convert_to(StrType::Meta);
                                url.convert_to(StrType::Meta);
                                let tmp = [&b"StreamTitle='"[..], &title.data, b"';StreamUrl='", &url.data, b"';"].concat();
                                let len = (tmp.len() + 15 + 1) / 16;
                                let mut padded = tmp.clone();
                                padded.push(0);
                                padded.resize(len * 16, 0);
                                sock.write(&[len as u8])?;
                                sock.write(&padded)?;
                                last_title = meta_title.clone();
                                last_url = info.url.clone();
                                crate::log_debug!(
                                    "StreamTitle: {}, StreamURL: {}",
                                    String::from_utf8_lossy(&last_title.data),
                                    String::from_utf8_lossy(&last_url.data)
                                );
                            } else {
                                sock.write(&[0])?;
                            }
                        }
                    }
                }
                sv.st().stream_pos = raw.pos.wrapping_add(raw.len());
            }
            if sys::get_time().wrapping_sub(last_write_time) > DIRECT_WRITE_TIMEOUT {
                return Err(Error::timeout());
            }
            sys::sleep_idle();
        }
        Ok(())
    })();
    if let Err(e) = r {
        crate::log_error!("Stream channel: {}", e);
    }
}

/// `sendPCPChannel`: 中継で PCP のパケットにして送る
fn send_pcp_channel(c: &mut Conn) -> Result<()> {
    let pc = c.pc;
    let sv = c.sv;
    let (chan_id, remote_id, send_header) = {
        let st = sv.st();
        (st.chan_id, st.remote_id, st.send_header)
    };
    let ch = pc.chanmgr.find_channel_by_id(&chan_id).ok_or_else(|| Error::stream("Channel not found"))?;
    let mut pcp = PcpStream::new(remote_id);
    sv.set_pcp(Some(pcp.shared.clone()));
    let mut error = 0;
    let sock = c.sock.as_mut().ok_or_else(|| Error::stream("Not connected"))?;
    let r = (|| -> Result<()> {
        crate::log_debug!("Starting PCP stream of channel at {}", sv.st().stream_pos);
        let (info, head, mut stream_index) = {
            let st = ch.st();
            (st.info.clone(), st.head_pack.clone(), st.stream_index)
        };
        let mut out = AtomBuf::default();
        out.parent(PCP_CHAN, 3 + send_header as i32);
        out.bytes(PCP_CHAN_ID, &chan_id);
        info.write_info_atoms(&mut out);
        info.write_track_atoms(&mut out);
        if send_header {
            out.parent(PCP_CHAN_PKT, 3);
            out.bytes(PCP_CHAN_PKT_TYPE, &PCP_CHAN_PKT_HEAD);
            out.int(PCP_CHAN_PKT_POS, head.pos as i32);
            out.bytes(PCP_CHAN_PKT_DATA, &head.data);
            sv.st().stream_pos = head.pos.wrapping_add(head.len());
            crate::log_debug!("Sent {} bytes header", head.len());
        }
        sock.write(&out.0)?;
        while sv.thread.active() {
            let ch = match pc.chanmgr.find_channel_by_id(&chan_id) {
                Some(c) => c,
                None => {
                    error = PCP_ERROR_QUIT + PCP_ERROR_OFFAIR;
                    break;
                }
            };
            let (si, hp) = {
                let st = ch.st();
                (st.stream_index, st.head_pack.pos)
            };
            if stream_index != si {
                stream_index = si;
                sv.st().stream_pos = hp;
                crate::log_debug!("sendPCPStream got new stream index {}", stream_index);
            }
            let mut out = AtomBuf::default();
            loop {
                let sp = sv.st().stream_pos;
                let raw = match ch.raw_data.find_packet(sp) {
                    Some(p) => p,
                    None => break,
                };
                if raw.ty == pb::T_HEAD {
                    out.parent(PCP_CHAN, 2);
                    out.bytes(PCP_CHAN_ID, &chan_id);
                    out.parent(PCP_CHAN_PKT, 3);
                    out.bytes(PCP_CHAN_PKT_TYPE, &PCP_CHAN_PKT_HEAD);
                    out.int(PCP_CHAN_PKT_POS, raw.pos as i32);
                    out.bytes(PCP_CHAN_PKT_DATA, &raw.data);
                } else if raw.ty == pb::T_DATA {
                    out.parent(PCP_CHAN, 2);
                    out.bytes(PCP_CHAN_ID, &chan_id);
                    out.parent(PCP_CHAN_PKT, if raw.cont { 4 } else { 3 });
                    out.bytes(PCP_CHAN_PKT_TYPE, &PCP_CHAN_PKT_DATA);
                    out.int(PCP_CHAN_PKT_POS, raw.pos as i32);
                    if raw.cont {
                        out.char(PCP_CHAN_PKT_CONTINUATION, 1);
                    }
                    out.bytes(PCP_CHAN_PKT_DATA, &raw.data);
                }
                if raw.pos < sp {
                    crate::log_debug!("pcp: skip back {}", raw.pos.wrapping_sub(sp) as i32);
                }
                sv.st().stream_pos = raw.pos.wrapping_add(raw.len());
            }
            sock.write(&out.0)?;
            let mut bcs = crate::pcp::BroadcastState::default();
            error = pcp.read_packet(pc, sock, &mut bcs);
            if error != 0 {
                return Err(Error::stream("PCP exception"));
            }
            sys::sleep(200);
        }
        crate::log_debug!("PCP channel stream closed normally.");
        Ok(())
    })();
    if let Err(e) = r {
        crate::log_error!("Stream channel: {}", e);
    }
    let _ = write_quit(sock, error);
    Ok(())
}

/// `readICYHeader`: ICY のヘッダーの行から、チャンネルの情報とパスワードを読む
pub fn read_icy_header(http: &Http, info: &mut ChanInfo, pwd: Option<&mut PcString>) {
    use crate::servhs::IcyHeader;
    let arg = match http.arg_str() {
        Some(a) => a.to_vec(),
        None => return,
    };
    match crate::servhs::icy_header(&http.cmd_line) {
        IcyHeader::Name => {
            info.name.set(&arg, StrType::Ascii);
            info.name.convert_to(StrType::Unicode);
        }
        IcyHeader::Url => info.url.set(&arg, StrType::Ascii),
        IcyHeader::Bitrate => info.bitrate = crate::http::atoi(&arg),
        IcyHeader::Genre => {
            info.genre.set(&arg, StrType::Ascii);
            info.genre.convert_to(StrType::Unicode);
        }
        IcyHeader::Desc => {
            info.desc.set(&arg, StrType::Ascii);
            info.desc.convert_to(StrType::Unicode);
        }
        IcyHeader::Authorization => {
            if let Some(p) = pwd {
                let (_, pass) = http.auth_user_pass();
                p.data = pass[..pass.len().min(255)].to_vec();
            }
        }
        IcyHeader::ChannelId => info.id = crate::gnuid::from_str(&arg),
        IcyHeader::Password => {
            if let Some(p) = pwd {
                if arg.len() < 64 {
                    p.data = arg.clone();
                }
            }
        }
        IcyHeader::ContentType => {
            if let Some(t) = crate::servhs::icy_content_type(&arg) {
                match t {
                    b"PCP" => info.src_protocol = ci::SP_PCP,
                    b"OGG" => info.content_type.assign(ci::T_OGG),
                    b"MP3" => info.content_type.assign(ci::T_MP3),
                    b"RAW" => info.content_type.assign(ci::T_RAW),
                    _ => info.content_type.assign(ci::T_PLS),
                }
            }
        }
        _ => {}
    }
}

pub use super::servmgr::MIN_TRACKER_RETRY;

#[allow(dead_code)]
fn _unused(_: PcString, _: &mut dyn Stream) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_support() {
        assert_eq!(continuation_packet_support(b"PeerCastStation/2.9.0"), Support::Supported);
        assert_eq!(continuation_packet_support(b"PeerCastStation/2.7.9"), Support::Unsupported);
        assert_eq!(continuation_packet_support(b"PeerCast/0.1218 (YT14)"), Support::Unsupported);
        assert_eq!(continuation_packet_support(b"PeerCast/0.1218 (YT50)"), Support::Supported);
        assert_eq!(continuation_packet_support(b"PeerCast/0.1218(VP28)"), Support::Unknown);
        assert_eq!(continuation_packet_support(b"PeerCast/0.1218"), Support::Unsupported);
        assert_eq!(continuation_packet_support(b"foo"), Support::Unknown);
    }
}
