//! 接続とサーバーの設定の管理 (core/common/servmgr.cpp の `ServMgr`)。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::chanhit::ChanHit;
use super::chaninfo::{self as ci, ChanInfo};
use super::cookie::{Cookie, CookieList};
use super::directory::{ChannelDirectory, RtmpServerMonitor, UptestServiceRegistry};
use super::error::{Error, Result};
use super::flag::FlagRegistry;
use super::host::{Host, Ip};
use super::ini::{IniReader, Section};
use super::packetbuf::{self as pb, ChanPacket};
use super::pcpconst::*;
use super::pcstr::{PcString, StrType};
use super::peercast::Peercast;
use super::servent::{self, Servent};
use super::servfilter::{self as sf, ServFilter};
use super::socket::ClientSocket;
use super::state::{arr, flag, n, obj, s, Value};
use super::stream::{FileStream, Stream};
use super::sys;
use crate::pcp::write::AtomBuf;

pub const DEFAULT_PORT: u16 = 7144;
pub const MIN_YP_RETRY: u32 = 20;
pub const MIN_TRACKER_RETRY: u32 = 10;
pub const MIN_RELAY_RETRY: u32 = 5;

pub const MAX_HOSTCACHE: usize = 100;
pub const MIN_RELAYS: u32 = 2;
pub const MAX_FILTERS: usize = 50;

/// `FW_STATE`
pub const FW_OFF: i32 = 0;
pub const FW_ON: i32 = 1;
pub const FW_UNKNOWN: i32 = 2;

/// `AUTH_TYPE`
pub const AUTH_COOKIE: i32 = 0;
pub const AUTH_HTTPBASIC: i32 = 1;

/// `ServHost::TYPE`
pub const SH_NONE: i32 = 0;
pub const SH_STREAM: i32 = 1;
pub const SH_CHANNEL: i32 = 2;
pub const SH_SERVENT: i32 = 3;
pub const SH_TRACKER: i32 = 4;

/// `ServHost`
#[derive(Clone, Copy, Debug)]
pub struct ServHost {
    pub ty: i32,
    pub host: Host,
    pub time: u32,
}

impl Default for ServHost {
    fn default() -> Self {
        ServHost { ty: SH_NONE, host: Host::none(), time: 0 }
    }
}

/// `ServHost::getTypeStr`
pub fn serv_host_type_str(t: i32) -> &'static str {
    match t {
        SH_NONE => "NONE",
        SH_STREAM => "STREAM",
        SH_CHANNEL => "CHANNEL",
        SH_SERVENT => "SERVENT",
        SH_TRACKER => "TRACKER",
        _ => "UNKNOWN",
    }
}

/// `ServHost::getTypeFromStr`
pub fn serv_host_type_from_str(s: &[u8]) -> i32 {
    for (t, n) in [(SH_NONE, "NONE"), (SH_SERVENT, "SERVENT"), (SH_STREAM, "STREAM"), (SH_CHANNEL, "CHANNEL"), (SH_TRACKER, "TRACKER")] {
        if super::ini::stricmp_eq(s, n.as_bytes()) {
            return t;
        }
    }
    SH_NONE
}

/// `getFirewallStateString`
pub fn firewall_state_str(s: i32) -> &'static str {
    match s {
        FW_ON => "ON",
        FW_OFF => "OFF",
        _ => "UNKNOWN",
    }
}

/// ServMgr の設定と状態 (C++ 版のメンバー)
#[derive(Clone, Debug)]
pub struct ServSettings {
    pub password: Vec<u8>,
    pub max_bitrate_out: u32,
    pub max_control: u32,
    pub max_relays: u32,
    pub max_direct: u32,
    pub max_serv_in: u32,
    /// 接続を受け付けてから要求を読み終えるまでの期限 (秒。0 なら期限なし)。Rust 版で足した
    pub handshake_timeout: u32,
    /// 同じ IP アドレスからの、要求を読み終えていない接続の数の上限 (0 なら上限なし)。Rust 版で足した
    pub max_handshakes_per_ip: u32,
    pub is_disabled: bool,
    pub server_host: Host,
    pub server_host_ipv6: Host,
    pub root_host: PcString,
    pub server_local_ip: Ip,
    pub server_local_ipv6: Ip,
    pub server_ip_addresses: VecDeque<Ip>,
    pub download_url: Vec<u8>,
    pub root_msg: PcString,
    pub force_ip: PcString,
    pub network_id: [u8; 16],
    pub firewall_timeout: u32,
    pub force_normal: bool,
    pub use_flow_control: bool,
    pub last_incoming: u32,
    pub allow_direct: bool,
    pub auto_connect: bool,
    pub auto_serve: bool,
    pub force_lookup: bool,
    pub allow_server1: u32,
    pub start_time: u32,
    pub tryout_delay: u32,
    pub refresh_html: u32,
    pub relay_broadcast: u32,
    pub notify_mask: u32,
    /// `filters[0..numFilters]` と、その次の作業用の 1 つ (`CMD_apply` が使う)
    pub filters: Vec<ServFilter>,
    pub auth_type: i32,
    pub html_path: Vec<u8>,
    pub chan_log: PcString,
    pub public_directory_enabled: bool,
    pub firewalled: i32,
    pub firewalled_ipv6: i32,
    pub server_name: PcString,
    pub transcoding_enabled: bool,
    pub preset: Vec<u8>,
    pub audio_codec: Vec<u8>,
    /// flv.cgi (トランスコード) を localhost 以外から同時に使える数 (Rust 版で足した)
    pub max_transcodes: u32,
    /// パスワードを続けて間違えたら締め出す数 (0 なら締め出さない)。Rust 版で足した
    pub auth_fail_limit: u32,
    /// 最初に締め出す秒数。間違え続けると倍にしていく
    pub auth_lock_seconds: u32,
    pub rtmp_port: u16,
    pub default_channel_info: ChanInfo,
    pub chat: bool,
    pub preferred_theme: Vec<u8>,
    pub accent_color: Vec<u8>,
}

/// `ServMgr`
pub struct ServMgr {
    servents: Mutex<Vec<Arc<Servent>>>,
    serv_num: AtomicI32,
    s: Mutex<ServSettings>,
    host_cache: Mutex<Vec<ServHost>>,
    pub cookies: Mutex<CookieList>,
    pub flags: FlagRegistry,
    pub channel_directory: ChannelDirectory,
    pub uptest: UptestServiceRegistry,
    pub rtmp_monitor: RtmpServerMonitor,
    pub session_id: [u8; 16],
    is_root: AtomicBool,
    pub total_streams: AtomicI32,
    pub shutdown_timer: AtomicI32,
    pub restart_server: AtomicBool,
    pub server_thread: sys::ThreadFlag,
    pub idle_thread: sys::ThreadFlag,
    start_time: AtomicU32,
}

impl ServMgr {
    /// `ServMgr()`。`rtmp_server_path` は rtmp-server の実行ファイル
    pub fn new(rtmp_server_path: &[u8]) -> ServMgr {
        let mut filters = Vec::new();
        ensure_catchall(&mut filters);
        let sm = ServMgr {
            servents: Mutex::new(Vec::new()),
            serv_num: AtomicI32::new(0),
            s: Mutex::new(ServSettings {
                password: Vec::new(),
                max_bitrate_out: 0,
                max_control: 3,
                max_relays: MIN_RELAYS,
                max_direct: 0,
                max_serv_in: 50,
                handshake_timeout: 15,
                max_handshakes_per_ip: 8,
                is_disabled: false,
                server_host: Host::from_str_ip(b"127.0.0.1", DEFAULT_PORT),
                server_host_ipv6: Host::new(Ip::parse(b"::1").unwrap_or_default(), DEFAULT_PORT),
                root_host: PcString::new(b"yp.pcgw.pgw.jp:7146"),
                server_local_ip: Ip::parse(b"127.0.0.1").unwrap_or_default(),
                server_local_ipv6: Ip::parse(b"::1").unwrap_or_default(),
                server_ip_addresses: VecDeque::from(vec![Ip::parse(b"127.0.0.1").unwrap_or_default(), Ip::parse(b"::1").unwrap_or_default()]),
                download_url: Vec::new(),
                root_msg: PcString::default(),
                force_ip: PcString::default(),
                network_id: [0; 16],
                firewall_timeout: 30,
                force_normal: false,
                use_flow_control: true,
                last_incoming: 0,
                allow_direct: true,
                auto_connect: true,
                auto_serve: true,
                force_lookup: true,
                allow_server1: servent::ALLOW_ALL,
                start_time: sys::get_time(),
                tryout_delay: 10,
                refresh_html: 5,
                relay_broadcast: 30,
                notify_mask: 0xffff,
                filters,
                auth_type: AUTH_COOKIE,
                html_path: b"html/en".to_vec(),
                chan_log: PcString::default(),
                public_directory_enabled: false,
                firewalled: FW_UNKNOWN,
                firewalled_ipv6: FW_UNKNOWN,
                server_name: PcString::default(),
                transcoding_enabled: false,
                preset: b"veryfast".to_vec(),
                audio_codec: b"mp3".to_vec(),
                max_transcodes: 2,
                auth_fail_limit: 5,
                auth_lock_seconds: 60,
                rtmp_port: 1935,
                default_channel_info: ChanInfo::new(),
                chat: true,
                preferred_theme: b"system".to_vec(),
                accent_color: b"blue".to_vec(),
            }),
            host_cache: Mutex::new(vec![ServHost::default(); MAX_HOSTCACHE]),
            cookies: Mutex::new(CookieList::default()),
            flags: FlagRegistry::servmgr_default(),
            channel_directory: ChannelDirectory::default(),
            uptest: UptestServiceRegistry::default(),
            rtmp_monitor: RtmpServerMonitor::new(rtmp_server_path),
            session_id: super::chanmgr::generate_id(0),
            is_root: AtomicBool::new(false),
            total_streams: AtomicI32::new(0),
            shutdown_timer: AtomicI32::new(0),
            restart_server: AtomicBool::new(false),
            server_thread: sys::ThreadFlag::new(),
            idle_thread: sys::ThreadFlag::new(),
            start_time: AtomicU32::new(sys::get_time()),
        };
        super::log::set_level(super::log::Level::Info as i32);
        sm.channel_directory.add_feed(b"http://yp.pcgw.pgw.jp/index.txt");
        let _ = sm.uptest.add_url(b"http://bayonet.ddo.jp/sp/yp4g.xml");
        sm
    }

    pub fn settings(&self) -> MutexGuard<'_, ServSettings> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cache(&self) -> MutexGuard<'_, Vec<ServHost>> {
        self.host_cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn svs(&self) -> MutexGuard<'_, Vec<Arc<Servent>>> {
        self.servents.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cookies(&self) -> MutexGuard<'_, CookieList> {
        self.cookies.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// サーバントの一覧の写し (新しいものが前)
    pub fn servents(&self) -> Vec<Arc<Servent>> {
        self.svs().clone()
    }

    pub fn is_root(&self) -> bool {
        self.is_root.load(Ordering::Relaxed)
    }

    pub fn set_root(&self, v: bool) {
        self.is_root.store(v, Ordering::Relaxed);
    }

    pub fn uptime(&self) -> u32 {
        sys::get_time().wrapping_sub(self.start_time.load(Ordering::Relaxed))
    }

    /// `updateIPAddress`: 見つけたアドレスを覚え、よりよいものならサーバーのアドレスにする
    pub fn update_ip_address(&self, new_ip: Ip) {
        let mut s = self.settings();
        if s.server_ip_addresses.contains(&new_ip) {
            return;
        }
        s.server_ip_addresses.push_front(new_ip);
        let addrs: Vec<Ip> = s.server_ip_addresses.iter().copied().collect();

        let score4 = |ip: &Ip| if ip.is_ipv4_loopback() { 0 } else if ip.is_ipv4_private() { 1 } else { 2 };
        let score6 = |ip: &Ip| {
            if ip.is_ipv6_loopback() {
                0
            } else if ip.is_ipv6_link_local() {
                1
            } else if ip.is_ipv6_unique_local() {
                2
            } else {
                3
            }
        };
        // IPv4
        let old = score4(&s.server_host.ip);
        if let Some(ip) = addrs.iter().find(|ip| ip.is_ipv4_mapped() && old <= score4(ip)) {
            if s.server_host.ip != *ip {
                crate::log_info!("Server IPv4 address changed to {} (was {})", ip.str(), s.server_host.ip.str());
                s.server_host.ip = *ip;
            }
        }
        // IPv4 local
        let old = score4(&s.server_local_ip);
        if let Some(ip) = addrs.iter().find(|ip| ip.is_ipv4_mapped() && old <= score4(ip) && score4(ip) < 2) {
            if s.server_local_ip != *ip {
                crate::log_info!("Server local IPv4 address changed to {} (was {})", ip.str(), s.server_local_ip.str());
                s.server_local_ip = *ip;
            }
        }
        // IPv6
        let old = score6(&s.server_host_ipv6.ip);
        if let Some(ip) = addrs.iter().find(|ip| !ip.is_ipv4_mapped() && old <= score6(ip)) {
            if s.server_host_ipv6.ip != *ip {
                crate::log_info!("Server IPv6 address changed to {} (was {})", ip.str(), s.server_host_ipv6.ip.str());
                s.server_host_ipv6.ip = *ip;
            }
        }
        // IPv6 local
        let old = score6(&s.server_local_ipv6);
        if let Some(ip) = addrs.iter().find(|ip| !ip.is_ipv4_mapped() && old <= score6(ip) && score6(ip) < 3) {
            if s.server_local_ipv6 != *ip {
                crate::log_info!("Server local IPv6 address changed to {} (was {})", ip.str(), s.server_local_ipv6.str());
                s.server_local_ipv6 = *ip;
            }
        }
    }

    /// `connectBroadcaster`: 配信中は YP への COUT をつなぐ
    pub fn connect_broadcaster(&self, pc: &Arc<Peercast>) {
        if self.settings().root_host.is_empty() {
            return;
        }
        if self.num_used(servent::T_COUT) == 0 {
            let sv = self.alloc_servent();
            servent::init_outgoing(pc, &sv, servent::T_COUT);
            sys::sleep(3000);
        }
    }

    /// `seenHost`
    pub fn seen_host(&self, h: &Host, ty: i32, time: u32) -> bool {
        let t = sys::get_time().wrapping_sub(time);
        self.cache().iter().any(|c| c.ty == ty && c.host.ip == h.ip && c.time >= t)
    }

    /// `addHost`
    pub fn add_host(&self, h: Host, ty: i32, time: u32) {
        if !h.is_valid() {
            return;
        }
        let mut c = self.cache();
        let mut idx = c.iter().position(|x| x.ty == ty && x.host == h);
        if idx.is_none() {
            crate::log_debug!("New host: {} - {}", h.str(), serv_host_type_str(ty));
        } else {
            crate::log_debug!("Old host: {} - {}", h.str(), serv_host_type_str(ty));
        }
        if idx.is_none() {
            idx = c.iter().position(|x| x.ty == SH_NONE);
        }
        if idx.is_none() {
            // 一番古いものと入れ替える
            let mut best: Option<usize> = None;
            for (i, x) in c.iter().enumerate() {
                if x.ty != SH_NONE && best.map_or(true, |b| x.time < c[b].time) {
                    best = Some(i);
                }
            }
            idx = best;
        }
        if let Some(i) = idx {
            c[i] = ServHost { ty, host: h, time: if time != 0 { time } else { sys::get_time() } };
        }
    }

    /// `deadHost`
    pub fn dead_host(&self, h: &Host, ty: i32) {
        for x in self.cache().iter_mut() {
            if x.ty == ty && x.host.ip == h.ip && x.host.port == h.port {
                *x = ServHost::default();
            }
        }
    }

    /// `clearHostCache`
    pub fn clear_host_cache(&self, ty: i32) {
        for x in self.cache().iter_mut() {
            if x.ty == ty || ty == SH_NONE {
                *x = ServHost::default();
            }
        }
    }

    /// `numHosts`
    pub fn num_hosts(&self, ty: i32) -> u32 {
        self.cache().iter().filter(|x| x.ty == ty || ty == SH_NONE).count() as u32
    }

    pub fn host_cache(&self) -> Vec<ServHost> {
        self.cache().clone()
    }

    /// `findServentByIndex`
    pub fn find_servent_by_index(&self, id: i32) -> Option<Arc<Servent>> {
        if id < 0 {
            return None;
        }
        self.svs().get(id as usize).cloned()
    }

    /// `findServentByID`
    pub fn find_servent_by_id(&self, id: i32) -> Option<Arc<Servent>> {
        self.svs().iter().find(|s| s.index == id).cloned()
    }

    /// `findServent(TYPE)`
    pub fn find_servent_by_type(&self, t: i32) -> Option<Arc<Servent>> {
        self.svs().iter().find(|s| s.ty() == t).cloned()
    }

    /// `allocServent`: 空いているサーバントを使うか、新しく作る
    pub fn alloc_servent(&self) -> Arc<Servent> {
        let mut l = self.svs();
        if let Some(s) = l.iter().find(|s| s.status() == servent::S_FREE).cloned() {
            crate::log_trace!("reused servent {}", s.index);
            s.reset();
            return s;
        }
        let num = self.serv_num.fetch_add(1, Ordering::SeqCst) + 1;
        let s = Arc::new(Servent::new(num));
        l.insert(0, s.clone());
        crate::log_trace!("allocated servent {}", num);
        s.reset();
        s
    }

    /// `closeConnections`
    pub fn close_connections(&self, ty: i32) {
        for sv in self.servents() {
            if sv.is_connected() && sv.ty() == ty {
                sv.thread.shutdown();
            }
        }
    }

    /// `numConnected(type, priv, uptime)`
    pub fn num_connected_priv(&self, pc: &Peercast, ty: i32, private: bool, uptime: u32) -> u32 {
        let ctime = sys::get_time();
        self.servents()
            .iter()
            .filter(|s| s.thread.active() && s.is_connected() && s.ty() == ty && s.is_private(pc) == private && ctime.wrapping_sub(s.last_connect()) >= uptime)
            .count() as u32
    }

    /// `numConnected(type, uptime)`: 両方
    pub fn num_connected_type(&self, pc: &Peercast, ty: i32) -> u32 {
        self.num_connected_priv(pc, ty, false, 0) + self.num_connected_priv(pc, ty, true, 0)
    }

    /// `numConnected()`
    pub fn num_connected(&self) -> u32 {
        self.servents().iter().filter(|s| s.thread.active() && s.is_connected()).count() as u32
    }

    /// `numServents`
    pub fn num_servents(&self) -> u32 {
        self.svs().len() as u32
    }

    /// `numUsed`
    pub fn num_used(&self, ty: i32) -> u32 {
        self.servents().iter().filter(|s| s.ty() == ty).count() as u32
    }

    /// `numActiveOnPort`
    pub fn num_active_on_port(&self, port: i32) -> u32 {
        self.servents().iter().filter(|s| s.thread.active() && s.has_sock() && s.st().serv_port == port).count() as u32
    }

    /// `numActive`
    pub fn num_active(&self, ty: i32) -> u32 {
        self.servents().iter().filter(|s| s.thread.active() && s.has_sock() && s.ty() == ty).count() as u32
    }

    /// `totalOutput`: 送っている量 (バイト/秒)
    pub fn total_output(&self, pc: &Peercast, all: bool) -> u32 {
        self.servents()
            .iter()
            .filter(|s| s.is_connected() && (all || !s.is_private(pc)))
            .filter_map(|s| s.sock_stat())
            .fold(0u32, |a, st| a.wrapping_add(st.bytes_out_per_sec()))
    }

    /// `numStreams(cid, type, all)`
    pub fn num_streams(&self, pc: &Peercast, cid: &[u8; 16], ty: i32, all: bool) -> u32 {
        self.servents()
            .iter()
            .filter(|s| s.is_connected() && s.ty() == ty && s.st().chan_id == *cid && (all || !s.is_private(pc)))
            .count() as u32
    }

    /// `numStreams(type, all)`
    pub fn num_streams_type(&self, pc: &Peercast, ty: i32, all: bool) -> u32 {
        self.servents().iter().filter(|s| s.is_connected() && s.ty() == ty && (all || !s.is_private(pc))).count() as u32
    }

    pub fn controls_in_full(&self, pc: &Peercast) -> bool {
        self.num_connected_type(pc, servent::T_CIN) >= self.settings().max_control
    }

    pub fn relays_full(&self, pc: &Peercast) -> bool {
        self.num_streams_type(pc, servent::T_RELAY, false) >= self.settings().max_relays
    }

    pub fn direct_full(&self, pc: &Peercast) -> bool {
        self.num_streams_type(pc, servent::T_DIRECT, false) >= self.settings().max_direct
    }

    /// `bitrateFull`: 帯域の上限を超えるか
    pub fn bitrate_full(&self, pc: &Peercast, br: u32) -> bool {
        let max = self.settings().max_bitrate_out;
        if max == 0 {
            return false;
        }
        let out = self.servents().iter().filter(|s| s.is_connected() && !s.is_private(pc)).filter_map(|s| s.sock_stat()).fold(0u32, |a, st| a.wrapping_add(st.bytes_out_per_sec()));
        (super::stats::bytes_to_kbps(out) + br as f32) > max as f32
    }

    /// `quit`
    pub fn quit(&self) {
        crate::log_debug!("ServMgr is quitting..");
        self.server_thread.shutdown();
        self.idle_thread.shutdown();
        crate::log_debug!("Disabling RMTP server..");
        self.rtmp_monitor.disable();
        for s in self.servents() {
            if s.thread.active() {
                s.thread.shutdown();
            }
        }
    }

    /// `checkForceIP`
    pub fn check_force_ip(&self) -> bool {
        let fip = self.settings().force_ip.data.clone();
        if fip.is_empty() {
            return false;
        }
        let new_ip = Ip::from_v4(super::host::get_ip(&fip));
        let mut s = self.settings();
        if s.server_host.ip != new_ip {
            s.server_host.ip = new_ip;
            crate::log_debug!("Server IP changed to {}", s.server_host.str());
            return true;
        }
        false
    }

    /// `checkFirewall`: YP に PCP でつなぎ、自分のグローバル IP とファイアウォールを確かめる
    pub fn check_firewall(&self, pc: &Arc<Peercast>) -> Result<()> {
        let rh = self.settings().root_host.data.clone();
        if rh.is_empty() {
            crate::log_error!("Root server is not set.");
            return Ok(());
        }
        crate::log_debug!("Checking firewall..");
        let host = Host::from_str_name(&rh, DEFAULT_PORT);
        if !host.ip.is_ipv4_mapped() {
            return Err(Error::general("Not an IPv4 addresss"));
        }
        crate::log_debug!("Contacting {} ({}) ...", String::from_utf8_lossy(&rh), host.ip.str());
        let mut sock = ClientSocket::new();
        sock.set_read_timeout(30000);
        sock.connect(host)?;
        let mut out = AtomBuf::default();
        out.int(PCP_CONNECT, 1);
        sock.write(&out.0)?;
        let rhost = sock.host;
        servent::handshake_outgoing_pcp(pc, &mut sock, rhost, true)?;
        let mut out = AtomBuf::default();
        out.int(PCP_QUIT, PCP_ERROR_QUIT);
        sock.write(&out.0)?;
        sock.close();
        Ok(())
    }

    /// `getFirewall`
    pub fn get_firewall(&self, ipv: i32) -> i32 {
        if self.flags.get("forceFirewalled") {
            return FW_ON;
        }
        let s = self.settings();
        if ipv == 6 {
            s.firewalled_ipv6
        } else {
            s.firewalled
        }
    }

    /// `setFirewall`
    pub fn set_firewall(&self, ipv: i32, state: i32) {
        let mut s = self.settings();
        let st = firewall_state_str(state);
        if ipv == 4 {
            if s.firewalled != state {
                crate::log_debug!("Firewall is set to {} (IPv4)", st);
                s.firewalled = state;
            }
        } else if s.firewalled_ipv6 != state {
            crate::log_debug!("Firewall is set to {} (IPv6)", st);
            s.firewalled_ipv6 = state;
        }
    }

    /// `checkFirewallIPv6`
    pub fn check_firewall_ipv6(&self) {
        crate::log_debug!("Checking firewall.. (IPv6)");
        let port = self.settings().server_host.port;
        match super::directory::ipv6_port_check(&self.session_id, port) {
            Ok(r) => {
                self.set_firewall(6, if r.ports.is_empty() { FW_ON } else { FW_OFF });
                self.update_ip_address(r.ip);
            }
            Err(e) => {
                crate::log_error!("checkFirewallIPv6: {}", e);
                self.set_firewall(6, FW_UNKNOWN);
            }
        }
    }

    /// `isBlacklisted`: 配信中、ほかの配信者のトラッカーからの視聴を禁じる
    pub fn is_blacklisted(&self, pc: &Peercast, h: &Host) -> bool {
        if self.flags.get("banTrackersWhileBroadcasting") && pc.chanmgr.is_broadcasting() {
            for e in self.channel_directory.channels() {
                let t = Host::from_string(&e.tip, 0);
                if t.ip.is_set() && t.ip == h.ip {
                    crate::log_info!("{}({}) is being blocked", h.ip.str(), String::from_utf8_lossy(&e.name));
                    return true;
                }
            }
        }
        false
    }

    /// `isFiltered`
    pub fn is_filtered(&self, pc: &Peercast, fl: u32, h: &Host) -> bool {
        if fl & sf::F_BAN != 0 && self.is_blacklisted(pc, h) {
            return true;
        }
        let filters = self.settings().filters.clone();
        let n = filters.len().saturating_sub(1);
        filters[..n].iter().any(|f| f.matches(fl, h))
    }

    /// `hasUnsafeFilterSettings`
    pub fn has_unsafe_filter_settings(&self) -> bool {
        let s = self.settings();
        let n = s.filters.len().saturating_sub(1);
        s.filters[..n].iter().any(|f| f.is_global() && f.flags & sf::F_PRIVATE != 0)
    }

    /// `setMaxRelays`
    pub fn set_max_relays(&self, max: i32) {
        self.settings().max_relays = (max.max(MIN_RELAYS as i32)) as u32;
    }

    /// `ChanHit::initLocal`: 自分のヒット
    #[allow(clippy::too_many_arguments)]
    pub fn init_local_hit(&self, pc: &Peercast, numl: i32, numr: i32, _nums: i32, uptm: u32, connected: bool, oldp: u32, newp: u32, can_add_relay: bool, source_host: Host, ipv6: bool) -> ChanHit {
        let mut h = ChanHit::new();
        h.firewalled = self.get_firewall(if ipv6 { 6 } else { 4 }) != FW_OFF;
        h.num_listeners = numl as u32;
        h.num_relays = numr as u32;
        h.up_time = uptm;
        h.stable = self.total_streams.load(Ordering::Relaxed) > 0;
        h.session_id = self.session_id;
        h.recv = connected;
        h.direct = !self.direct_full(pc);
        h.relay = can_add_relay;
        h.cin = !self.controls_in_full(pc);
        let s = self.settings();
        h.host = if ipv6 { s.server_host_ipv6 } else { s.server_host };
        h.version = PCP_CLIENT_VERSION;
        h.version_vp = PCP_CLIENT_VERSION_VP;
        h.version_ex_prefix = *PCP_CLIENT_VERSION_EX_PREFIX;
        h.version_ex_number = PCP_CLIENT_VERSION_EX_NUMBER;
        h.rhost[0] = h.host;
        h.rhost[1] = Host::new(if ipv6 { s.server_local_ipv6 } else { s.server_local_ip }, h.host.port);
        if h.firewalled {
            h.rhost[0].port = 0;
        }
        h.oldest_pos = oldp;
        h.newest_pos = newp;
        h.uphost = source_host;
        h.uphost_hops = 1;
        h
    }

    /// `broadcastPacket`: 条件に合うサーバントに送る。送った数を返す
    pub fn broadcast_packet(&self, pack: &mut ChanPacket, chan_id: &[u8; 16], src_id: &[u8; 16], dest_id: &[u8; 16], ty: i32) -> i32 {
        self.servents().iter().filter(|sv| sv.send_packet(pack, chan_id, src_id, dest_id, ty)).count() as i32
    }

    /// `broadcastPushRequest`
    pub fn broadcast_push_request(&self, hit: &ChanHit, to: &Host, chan_id: &[u8; 16], ty: i32) -> i32 {
        let mut out = AtomBuf::default();
        out.parent(PCP_BCST, 10);
        out.char(PCP_BCST_GROUP, 0xff);
        out.char(PCP_BCST_HOPS, 0);
        out.char(PCP_BCST_TTL, 7);
        out.bytes(PCP_BCST_DEST, &hit.session_id);
        out.bytes(PCP_BCST_FROM, &self.session_id);
        out.int(PCP_BCST_VERSION, PCP_CLIENT_VERSION as i32);
        out.int(PCP_BCST_VERSION_VP, PCP_CLIENT_VERSION_VP as i32);
        out.bytes(PCP_BCST_VERSION_EX_PREFIX, PCP_CLIENT_VERSION_EX_PREFIX);
        out.short(PCP_BCST_VERSION_EX_NUMBER, PCP_CLIENT_VERSION_EX_NUMBER as i16);
        out.parent(PCP_PUSH, 3);
        out.address(PCP_PUSH_IP, &to.ip.0);
        out.short(PCP_PUSH_PORT, to.port as i16);
        out.bytes(PCP_PUSH_CHANID, chan_id);
        match ChanPacket::new(pb::T_PCP, &out.0, 0) {
            Ok(mut p) => self.broadcast_packet(&mut p, &[0; 16], &self.session_id, &hit.session_id, ty),
            Err(_) => 0,
        }
    }

    /// `writeRootAtoms`
    pub fn write_root_atoms(&self, pc: &Peercast, out: &mut AtomBuf, get_update: bool) {
        let hui = pc.chanmgr.settings().host_update_interval as i32;
        let msg = self.settings().root_msg.data.clone();
        out.parent(PCP_ROOT, 5 + get_update as i32);
        out.int(PCP_ROOT_UPDINT, hui);
        out.string(PCP_ROOT_URL, b"download.php");
        out.int(PCP_ROOT_CHECKVER, PCP_ROOT_VERSION);
        out.int(PCP_ROOT_NEXT, hui);
        out.string(PCP_MESG_ASCII, &msg);
        if get_update {
            out.parent(PCP_ROOT_UPDATE, 0);
        }
    }

    /// `broadcastRootSettings`
    pub fn broadcast_root_settings(&self, pc: &Peercast, get_update: bool) {
        if !self.is_root() {
            return;
        }
        let mut out = AtomBuf::default();
        out.parent(PCP_BCST, 9);
        out.char(PCP_BCST_GROUP, PCP_BCST_GROUP_TRACKERS as u8);
        out.char(PCP_BCST_HOPS, 0);
        out.char(PCP_BCST_TTL, 7);
        out.bytes(PCP_BCST_FROM, &self.session_id);
        out.int(PCP_BCST_VERSION, PCP_CLIENT_VERSION as i32);
        out.int(PCP_BCST_VERSION_VP, PCP_CLIENT_VERSION_VP as i32);
        out.bytes(PCP_BCST_VERSION_EX_PREFIX, PCP_CLIENT_VERSION_EX_PREFIX);
        out.short(PCP_BCST_VERSION_EX_NUMBER, PCP_CLIENT_VERSION_EX_NUMBER as i16);
        self.write_root_atoms(pc, &mut out, get_update);
        if let Ok(mut p) = ChanPacket::new(pb::T_PCP, &out.0, 0) {
            self.broadcast_packet(&mut p, &[0; 16], &self.session_id, &[0; 16], servent::T_CIN);
        }
    }

    /// `findConnection`
    pub fn find_connection(&self, ty: i32, sid: &[u8; 16]) -> Option<Arc<Servent>> {
        self.servents().into_iter().find(|s| s.is_connected() && s.ty() == ty && s.st().remote_id == *sid)
    }

    /// `acceptGIV`: COUT のサーバントに渡す
    pub fn accept_giv(&self, sock: ClientSocket) -> std::result::Result<(), ClientSocket> {
        let mut sock = sock;
        for sv in self.servents() {
            if sv.ty() == servent::T_COUT {
                match sv.accept_giv(sock) {
                    Ok(()) => return Ok(()),
                    Err(s) => sock = s,
                }
            }
        }
        Err(sock)
    }

    /// `procConnectArgs`: "[チャンネルID]?ip=...&tip=..." から、チャンネルの情報とヒットを作る
    pub fn proc_connect_args(&self, pc: &Peercast, s: &[u8]) -> ChanInfo {
        let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
        let (idpart, args) = match s.iter().position(|&c| c == b'?') {
            Some(i) => (&s[..i], &s[i + 1..]),
            None => (s, &b""[..]),
        };
        let info = ChanInfo::init_name_id(idpart);
        let q = crate::servhs::Query::new(args);
        let ip = q.get(b"ip");
        let tip = q.get(b"tip");
        if !ip.is_empty() {
            let h = Host::from_str_name(&ip, DEFAULT_PORT);
            let mut hit = ChanHit::new();
            hit.host = h;
            hit.rhost[0] = h;
            hit.rhost[1] = Host::none();
            hit.chan_id = info.id;
            hit.recv = true;
            pc.chanmgr.add_hit(pc, &hit);
        } else if !tip.is_empty() {
            let h = Host::from_string(&tip, DEFAULT_PORT);
            let (sh4, sh6) = {
                let st = self.settings();
                (st.server_host.ip, st.server_host_ipv6.ip)
            };
            if h.port == 0 {
                crate::log_debug!("ポート0のトラッカーIPはホストキャッシュに登録しない。");
            } else if h.ip == sh4 || h.ip == sh6 {
                crate::log_debug!("'tip' parameter is server global IP({}). Ignored.", h.ip.str());
            } else {
                pc.chanmgr.add_hit_host(pc, h, &info.id, true);
            }
        }
        info
    }

    /// `getChannel`: チャンネルを探し、`relay` なら中継を始めて待つ。見つかれば情報を返す
    /// `getChannel`: 見付からなければ false と、引数から読んだ情報
    pub fn get_channel(&self, pc: &Arc<Peercast>, s: &[u8], relay: bool) -> (ChanInfo, bool) {
        let info = self.proc_connect_args(pc, s);
        match self.find_playing_channel(pc, &info, relay) {
            Some(i) => (i, true),
            None => (info, false),
        }
    }

    fn find_playing_channel(&self, pc: &Arc<Peercast>, info: &ChanInfo, relay: bool) -> Option<ChanInfo> {
        let mut ch = pc.chanmgr.find_channel_by_name_id(&info);
        match &ch {
            Some(c) => {
                if !c.is_playing() {
                    if relay {
                        {
                            let mut st = c.st();
                            st.info.last_play_start = 0;
                            st.info.last_play_end = 0;
                        }
                        for _ in 0..100 {
                            let ci = ch.as_ref().map(|c| c.info()).unwrap_or_else(ChanInfo::new);
                            ch = pc.chanmgr.find_channel_by_name_id(&ci);
                            match &ch {
                                None => return None,
                                Some(c) => {
                                    if c.is_playing() {
                                        break;
                                    }
                                }
                            }
                            sys::sleep(100);
                        }
                    } else {
                        return None;
                    }
                }
                ch.map(|c| c.info())
            }
            None => {
                if relay {
                    pc.chanmgr.find_and_relay(pc, &info).map(|c| c.info())
                } else {
                    None
                }
            }
        }
    }

    // ---------------------------------------------------------------- 設定のファイル

    /// `getSettings`
    pub fn get_settings(&self, pc: &Peercast) -> Vec<Section> {
        let s = self.settings().clone();
        let cs = pc.chanmgr.settings().clone();
        let cookies_never = self.cookies().never_expire;
        let mut doc = Vec::new();
        doc.push(
            Section::new("Server", false)
                .key("serverName", &s.server_name.data[..])
                .key("serverPort", s.server_host.port)
                .key("autoServe", s.auto_serve)
                .key("forceIP", &s.force_ip.data[..])
                .key("isRoot", self.is_root())
                .key("maxBitrateOut", s.max_bitrate_out)
                .key("maxRelays", s.max_relays)
                .key("maxDirect", s.max_direct)
                .key("maxRelaysPerChannel", cs.max_relays_per_channel)
                .key("firewallTimeout", s.firewall_timeout)
                .key("forceNormal", s.force_normal)
                .key("rootMsg", &s.root_msg.data[..])
                .key("authType", if s.auth_type == AUTH_COOKIE { "cookie" } else { "http-basic" })
                .key("cookiesExpire", if cookies_never { "never" } else { "session" })
                .key("htmlPath", &s.html_path[..])
                .key("maxServIn", s.max_serv_in)
                .key("handshakeTimeout", s.handshake_timeout)
                .key("maxHandshakesPerIP", s.max_handshakes_per_ip)
                .key("chanLog", &s.chan_log.data[..])
                .key("publicDirectory", s.public_directory_enabled)
                .key("networkID", ci::id_str(&s.network_id)),
        );
        doc.push(
            Section::new("Broadcast", false)
                .key("broadcastMsgInterval", cs.broadcast_msg_interval)
                .key("broadcastMsg", &cs.broadcast_msg.data[..])
                .key("icyMetaInterval", cs.icy_meta_interval)
                .key("broadcastID", ci::id_str(&cs.broadcast_id))
                .key("hostUpdateInterval", cs.host_update_interval)
                .key("maxControlConnections", s.max_control)
                .key("rootHost", &s.root_host.data[..]),
        );
        doc.push(
            Section::new("Client", false)
                .key("refreshHTML", s.refresh_html)
                .key("chat", s.chat)
                .key("relayBroadcast", s.relay_broadcast)
                .key("minBroadcastTTL", cs.min_broadcast_ttl)
                .key("maxBroadcastTTL", cs.max_broadcast_ttl)
                .key("pushTries", cs.push_tries)
                .key("pushTimeout", cs.push_timeout)
                .key("maxPushHops", cs.max_push_hops)
                .key("transcodingEnabled", s.transcoding_enabled)
                .key("preset", &s.preset[..])
                .key("audioCodec", &s.audio_codec[..])
                .key("maxTranscodes", s.max_transcodes)
                .key("preferredTheme", &s.preferred_theme[..])
                .key("accentColor", &s.accent_color[..]),
        );
        doc.push(
            Section::new("Privacy", false)
                .key("password", &s.password[..])
                .key("maxUptime", cs.max_uptime)
                .key("authFailLimit", s.auth_fail_limit)
                .key("authLockSeconds", s.auth_lock_seconds),
        );
        let nf = s.filters.len().saturating_sub(1);
        for f in &s.filters[..nf] {
            doc.push(
                Section::new("Filter", true)
                    .key("ip", f.pattern())
                    .key("private", f.flags & sf::F_PRIVATE != 0)
                    .key("ban", f.flags & sf::F_BAN != 0)
                    .key("network", f.flags & sf::F_NETWORK != 0)
                    .key("direct", f.flags & sf::F_DIRECT != 0),
            );
        }
        for feed in self.channel_directory.feeds() {
            doc.push(Section::new("Feed", true).key("url", feed.url));
        }
        for url in self.uptest.urls() {
            doc.push(Section::new("Uptest", true).key("url", url));
        }
        doc.push(
            Section::new("Notify", true)
                .key("PeerCast", s.notify_mask & super::notif::NT_PEERCAST != 0)
                .key("Broadcasters", s.notify_mask & super::notif::NT_BROADCASTERS != 0)
                .key("TrackInfo", s.notify_mask & super::notif::NT_TRACKINFO != 0),
        );
        doc.push(
            Section::new("Server1", true)
                .key("allowHTML", s.allow_server1 & servent::ALLOW_HTML != 0)
                .key("allowBroadcast", s.allow_server1 & servent::ALLOW_BROADCAST != 0)
                .key("allowNetwork", s.allow_server1 & servent::ALLOW_NETWORK != 0)
                .key("allowDirect", s.allow_server1 & servent::ALLOW_DIRECT != 0),
        );
        doc.push(
            Section::new("Debug", false)
                .key("logLevel", super::log::level())
                .key("pauseLog", super::log::paused())
                .key("idleSleepTime", sys::idle_sleep_time()),
        );
        for c in pc.chanmgr.channels() {
            if c.is_active() && c.st().stay_connected {
                doc.push(relay_channel_section(pc, &c));
            }
        }
        for sh in self.host_cache() {
            if sh.ty != SH_NONE {
                doc.push(Section::new("Host", true).key("type", serv_host_type_str(sh.ty)).key("address", sh.host.str()).key("time", sh.time));
            }
        }
        let mut flags = Section::new("Flags", true);
        for f in self.flags.sorted() {
            flags.push(f.name, f.get());
        }
        doc.push(flags);
        doc
    }

    /// `saveSettings`: 一時ファイルに書いてから名前を変える
    pub fn save_settings(&self, pc: &Peercast, fn_: &[u8]) {
        let doc = self.get_settings(pc);
        let tmp = [fn_, b".tmp"].concat();
        match FileStream::open_write(&tmp) {
            Ok(mut f) => {
                crate::log_debug!("Saving settings to: {}", String::from_utf8_lossy(fn_));
                let _ = f.write(&super::ini::dump(&doc));
                f.close();
            }
            Err(_) => crate::log_error!("Unable to open ini file"),
        }
        if let (Some(a), Some(b)) = (sys::bytes_to_path(&tmp), sys::bytes_to_path(fn_)) {
            if let Err(e) = std::fs::rename(a, b) {
                crate::log_error!("rename failed: rename: {}", super::error::strerror(&e));
            }
        }
        self.save_token_list(pc);
    }

    /// `saveTokenList`
    pub fn save_token_list(&self, pc: &Peercast) {
        if !self.flags.get("persistTokenList") {
            return;
        }
        let fname = pc.app.token_list_filename.clone();
        let tmp = [&fname[..], b".tmp"].concat();
        let body = match self.cookies().state().inspect() {
            Ok(b) => b,
            Err(_) => return,
        };
        let p = match sys::bytes_to_path(&tmp) {
            Some(p) => p,
            None => return,
        };
        if std::fs::write(&p, body).is_err() {
            crate::log_error!("saveTokenList: Failed to open {} for writing", String::from_utf8_lossy(&tmp));
            return;
        }
        if let Some(b) = sys::bytes_to_path(&fname) {
            if let Err(e) = std::fs::rename(&p, b) {
                crate::log_error!("rename failed: rename: {}", super::error::strerror(&e));
            }
        }
    }

    /// `loadTokenList`
    pub fn load_token_list(&self, pc: &Peercast) {
        if !self.flags.get("persistTokenList") {
            return;
        }
        let fname = pc.app.token_list_filename.clone();
        let data = match sys::bytes_to_path(&fname).and_then(|p| std::fs::read(p).ok()) {
            Some(d) => d,
            None => {
                crate::log_error!("loadTokenList: Failed to open {}", String::from_utf8_lossy(&fname));
                return;
            }
        };
        let r = (|| -> std::result::Result<Vec<Cookie>, String> {
            let arr = crate::json::parse(&data).map_err(|e| String::from_utf8_lossy(&e.what()).into_owned())?;
            let items = match arr {
                crate::json::Value::Array(a) => a,
                _ => return Err("json format error: array expected".into()),
            };
            let mut cookies = Vec::new();
            for obj in items {
                if !matches!(obj, crate::json::Value::Object(_)) {
                    return Err("json format error: object expected".into());
                }
                let id = match obj.get(b"id") {
                    Some(crate::json::Value::Str(s)) => s.clone(),
                    _ => return Err("[json.exception.out_of_range.403] key 'id' not found".into()),
                };
                let ip = match obj.get(b"ip") {
                    Some(crate::json::Value::Str(s)) => s.clone(),
                    _ => return Err("[json.exception.out_of_range.403] key 'ip' not found".into()),
                };
                match Ip::parse(&ip) {
                    Some(ip) => cookies.push(Cookie::new(&id, ip)),
                    None => {
                        crate::log_error!("Failed to parse an entry in token list. Skipping");
                        cookies.push(Cookie::empty());
                    }
                }
            }
            Ok(cookies)
        })();
        match r {
            Ok(cookies) => {
                let mut cl = self.cookies();
                let ne = cl.never_expire;
                cl.init();
                cl.never_expire = ne;
                for c in cookies {
                    cl.add(c);
                }
            }
            Err(e) => crate::log_error!("loadTokenList: {}", e),
        }
    }

    /// `loadSettings`
    pub fn load_settings(&self, pc: &Arc<Peercast>, fn_: &[u8]) {
        if FileStream::open_read(fn_).is_err() {
            self.save_settings(pc, fn_);
        }
        // numFilters = 0 (作業用の 1 つだけ)
        self.settings().filters = vec![ServFilter::default()];
        self.uptest.clear();
        self.channel_directory.clear_feeds();

        if let Ok(mut f) = FileStream::open_read(fn_) {
            let mut r = IniReader::new(&mut f);
            while r.read_next() {
                self.load_line(pc, &mut r);
            }
        }
        ensure_catchall(&mut self.settings().filters);
    }

    fn load_line(&self, pc: &Arc<Peercast>, r: &mut IniReader) {
        let cm = &pc.chanmgr;
        let v = r.str_value().to_vec();
        let iv = r.int_value();
        let bv = r.bool_value();
        macro_rules! set {
            ($f:expr) => {{
                let mut s = self.settings();
                $f(&mut s);
            }};
        }
        let name = r.name().to_vec();
        let is = |n: &str| super::ini::stricmp_eq(&name, n.as_bytes());
        if is("serverName") {
            set!(|s: &mut ServSettings| s.server_name.assign(&v));
        } else if is("serverPort") {
            set!(|s: &mut ServSettings| {
                s.server_host.port = iv as u16;
                s.server_host_ipv6.port = iv as u16;
            });
        } else if is("autoServe") {
            set!(|s: &mut ServSettings| s.auto_serve = bv);
        } else if is("autoConnect") {
            set!(|s: &mut ServSettings| s.auto_connect = bv);
        } else if is("icyPassword") || is("password") {
            set!(|s: &mut ServSettings| s.password = v[..v.len().min(63)].to_vec());
        } else if is("forceIP") {
            set!(|s: &mut ServSettings| s.force_ip.assign(&v));
        } else if is("isRoot") {
            self.set_root(bv);
        } else if is("broadcastID") {
            cm.settings().broadcast_id = crate::gnuid::from_str(&v);
        } else if is("htmlPath") {
            if crate::servhs::is_valid_html_path(&v) {
                set!(|s: &mut ServSettings| s.html_path = v[..v.len().min(127)].to_vec());
            } else {
                crate::log_warn!("Ignoring invalid htmlPath in ini file");
            }
        } else if is("maxControlConnections") {
            set!(|s: &mut ServSettings| s.max_control = iv as u32);
        } else if is("maxBitrateOut") {
            set!(|s: &mut ServSettings| s.max_bitrate_out = iv as u32);
        } else if is("maxStreamsOut") || is("maxRelays") {
            self.set_max_relays(iv);
        } else if is("maxDirect") {
            set!(|s: &mut ServSettings| s.max_direct = iv as u32);
        } else if is("maxStreamsPerChannel") || is("maxRelaysPerChannel") {
            cm.settings().max_relays_per_channel = iv;
        } else if is("firewallTimeout") {
            set!(|s: &mut ServSettings| s.firewall_timeout = iv as u32);
        } else if is("forceNormal") {
            set!(|s: &mut ServSettings| s.force_normal = bv);
        } else if is("broadcastMsgInterval") {
            cm.settings().broadcast_msg_interval = iv as u32;
        } else if is("broadcastMsg") {
            cm.settings().broadcast_msg.set(&v, StrType::Ascii);
        } else if is("hostUpdateInterval") {
            cm.settings().host_update_interval = iv as u32;
        } else if is("icyMetaInterval") {
            cm.settings().icy_meta_interval = iv;
        } else if is("maxServIn") {
            set!(|s: &mut ServSettings| s.max_serv_in = iv as u32);
        } else if is("handshakeTimeout") {
            set!(|s: &mut ServSettings| s.handshake_timeout = iv.max(0) as u32);
        } else if is("maxHandshakesPerIP") {
            set!(|s: &mut ServSettings| s.max_handshakes_per_ip = iv.max(0) as u32);
        } else if is("chanLog") {
            set!(|s: &mut ServSettings| s.chan_log.set(&v, StrType::Ascii));
        } else if is("publicDirectory") {
            set!(|s: &mut ServSettings| s.public_directory_enabled = bv);
        } else if is("rootMsg") {
            set!(|s: &mut ServSettings| s.root_msg.assign(&v));
        } else if is("networkID") {
            set!(|s: &mut ServSettings| s.network_id = crate::gnuid::from_str(&v));
        } else if is("authType") {
            if super::ini::stricmp_eq(&v, b"cookie") {
                set!(|s: &mut ServSettings| s.auth_type = AUTH_COOKIE);
            } else if super::ini::stricmp_eq(&v, b"http-basic") {
                set!(|s: &mut ServSettings| s.auth_type = AUTH_HTTPBASIC);
            }
        } else if is("cookiesExpire") {
            if super::ini::stricmp_eq(&v, b"never") {
                self.cookies().never_expire = true;
            } else if super::ini::stricmp_eq(&v, b"session") {
                self.cookies().never_expire = false;
            }
        } else if is("maxUptime") {
            cm.settings().max_uptime = iv as u32;
        } else if is("rootHost") {
            set!(|s: &mut ServSettings| s.root_host.assign(&v));
        } else if is("deadHitAge") {
            cm.settings().dead_hit_age = iv as u32;
        } else if is("tryoutDelay") {
            set!(|s: &mut ServSettings| s.tryout_delay = iv as u32);
        } else if is("refreshHTML") {
            set!(|s: &mut ServSettings| s.refresh_html = iv as u32);
        } else if is("chat") {
            set!(|s: &mut ServSettings| s.chat = bv);
        } else if is("relayBroadcast") {
            set!(|s: &mut ServSettings| s.relay_broadcast = (iv as u32).max(30));
        } else if is("minBroadcastTTL") {
            cm.settings().min_broadcast_ttl = iv;
        } else if is("maxBroadcastTTL") {
            cm.settings().max_broadcast_ttl = iv;
        } else if is("pushTimeout") {
            cm.settings().push_timeout = iv;
        } else if is("pushTries") {
            cm.settings().push_tries = iv;
        } else if is("maxPushHops") {
            cm.settings().max_push_hops = iv;
        } else if is("transcodingEnabled") {
            set!(|s: &mut ServSettings| s.transcoding_enabled = bv);
        } else if is("preset") {
            set!(|s: &mut ServSettings| s.preset = v.clone());
        } else if is("audioCodec") {
            set!(|s: &mut ServSettings| s.audio_codec = v.clone());
        } else if is("maxTranscodes") {
            set!(|s: &mut ServSettings| s.max_transcodes = iv.max(0) as u32);
        } else if is("authFailLimit") {
            set!(|s: &mut ServSettings| s.auth_fail_limit = iv.max(0) as u32);
        } else if is("authLockSeconds") {
            set!(|s: &mut ServSettings| s.auth_lock_seconds = iv.max(0) as u32);
        } else if is("preferredTheme") {
            set!(|s: &mut ServSettings| s.preferred_theme = v.clone());
        } else if is("accentColor") {
            set!(|s: &mut ServSettings| s.accent_color = v.clone());
        } else if is("logLevel") {
            super::log::set_level(iv);
        } else if is("pauseLog") {
            super::log::set_paused(bv);
        } else if is("idleSleepTime") {
            sys::set_idle_sleep_time(iv as u32);
        } else if is("[Server1]") {
            let mut a = self.settings().allow_server1;
            while r.read_next() {
                let set_bit = |a: u32, bit: u32, on: bool| if on { a | bit } else { a & !bit };
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("allowHTML") {
                    a = set_bit(a, servent::ALLOW_HTML, r.bool_value());
                } else if r.is_name("allowDirect") {
                    a = set_bit(a, servent::ALLOW_DIRECT, r.bool_value());
                } else if r.is_name("allowNetwork") {
                    a = set_bit(a, servent::ALLOW_NETWORK, r.bool_value());
                } else if r.is_name("allowBroadcast") {
                    a = set_bit(a, servent::ALLOW_BROADCAST, r.bool_value());
                }
            }
            self.settings().allow_server1 = a;
        } else if is("[Filter]") {
            let mut f = ServFilter::default();
            while r.read_next() {
                let fl = |f: &mut ServFilter, bit: u32, on: bool| f.flags = (f.flags & !bit) | if on { bit } else { 0 };
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("ip") {
                    f.set_pattern(r.str_value());
                } else if r.is_name("private") {
                    fl(&mut f, sf::F_PRIVATE, r.bool_value());
                } else if r.is_name("ban") {
                    fl(&mut f, sf::F_BAN, r.bool_value());
                } else if r.is_name("allow") || r.is_name("network") {
                    fl(&mut f, sf::F_NETWORK, r.bool_value());
                } else if r.is_name("direct") {
                    fl(&mut f, sf::F_DIRECT, r.bool_value());
                }
            }
            // C++ 版は filters[numFilters] に読み、上限に達していなければ numFilters を進める
            // (最後の要素が filters[numFilters])
            let mut s = self.settings();
            let n = s.filters.len() - 1;
            s.filters[n] = f;
            if n < MAX_FILTERS - 1 {
                s.filters.push(ServFilter::default());
            }
        } else if is("[Feed]") {
            while r.read_next() {
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("url") {
                    self.channel_directory.add_feed(r.str_value());
                }
            }
        } else if is("[Uptest]") {
            while r.read_next() {
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("url") {
                    let _ = self.uptest.add_url(r.str_value());
                }
            }
        } else if is("[Notify]") {
            let mut m = super::notif::NT_UPGRADE;
            while r.read_next() {
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("PeerCast") && r.bool_value() {
                    m |= super::notif::NT_PEERCAST;
                } else if r.is_name("Broadcasters") && r.bool_value() {
                    m |= super::notif::NT_BROADCASTERS;
                } else if r.is_name("TrackInfo") && r.bool_value() {
                    m |= super::notif::NT_TRACKINFO;
                }
            }
            self.settings().notify_mask = m;
        } else if is("[RelayChannel]") {
            self.load_relay_channel(pc, r);
        } else if is("[Host]") {
            let mut h = Host::none();
            let mut ty = SH_NONE;
            let mut time = 0;
            while r.read_next() {
                if r.is_name("[End]") {
                    break;
                } else if r.is_name("address") {
                    h = Host::from_str_ip(r.str_value(), DEFAULT_PORT);
                } else if r.is_name("type") {
                    ty = serv_host_type_from_str(r.str_value());
                } else if r.is_name("time") {
                    time = r.int_value() as u32;
                }
            }
            self.add_host(h, ty, time);
        } else if is("[Flags]") {
            while r.read_next() {
                if r.is_name("[End]") {
                    break;
                }
                match self.flags.find(r.name()) {
                    Some(f) => f.set(r.bool_value()),
                    None => crate::log_error!("Flag {} not found", String::from_utf8_lossy(r.name())),
                }
            }
        }
    }

    fn load_relay_channel(&self, pc: &Arc<Peercast>, r: &mut IniReader) {
        let mut info = ChanInfo::new();
        let mut stay = false;
        let mut source_url = PcString::default();
        let mut ipv = super::channel::IP_V4;
        while r.read_next() {
            let v = r.str_value().to_vec();
            if r.is_name("[End]") {
                break;
            } else if r.is_name("name") {
                info.name.assign(&v);
            } else if r.is_name("desc") {
                info.desc.assign(&v);
            } else if r.is_name("genre") {
                info.genre.assign(&v);
            } else if r.is_name("contactURL") {
                info.url.assign(&v);
            } else if r.is_name("comment") {
                info.comment.assign(&v);
            } else if r.is_name("id") {
                info.id = crate::gnuid::from_str(&v);
            } else if r.is_name("sourceType") {
                info.src_protocol = ci::protocol_from_str(&v);
            } else if r.is_name("contentType") {
                info.content_type.assign(&v);
            } else if r.is_name("MIMEType") {
                info.mime_type.assign(&v);
            } else if r.is_name("streamExt") {
                info.stream_ext.assign(&v);
            } else if r.is_name("stayConnected") {
                stay = r.bool_value();
            } else if r.is_name("sourceURL") {
                source_url.assign(&v);
            } else if r.is_name("bitrate") {
                info.bitrate = crate::http::atoi(&v);
            } else if r.is_name("tracker") {
                let mut hit = ChanHit::new();
                hit.tracker = true;
                hit.host = Host::from_str_name(&v, DEFAULT_PORT);
                hit.rhost[0] = hit.host;
                hit.rhost[1] = hit.host;
                hit.chan_id = info.id;
                hit.recv = true;
                pc.chanmgr.add_hit(pc, &hit);
            } else if r.is_name("trackContact") {
                info.track.contact.assign(&v);
            } else if r.is_name("trackTitle") {
                info.track.title.assign(&v);
            } else if r.is_name("trackArtist") {
                info.track.artist.assign(&v);
            } else if r.is_name("trackAlbum") {
                info.track.album.assign(&v);
            } else if r.is_name("trackGenre") {
                info.track.genre.assign(&v);
            } else if r.is_name("ipVersion") {
                ipv = if r.int_value() == 6 { super::channel::IP_V6 } else { super::channel::IP_V4 };
            }
        }
        if source_url.is_empty() {
            pc.chanmgr.create_relay(pc, &info, stay);
        } else {
            info.bc_id = pc.chanmgr.broadcast_id();
            let c = pc.chanmgr.create_channel(pc, &info, None);
            c.st().ip_version = ipv;
            c.start_url(pc, &source_url.data);
        }
    }

    // ---------------------------------------------------------------- スレッド

    /// `start`: サーバーとアイドルのスレッドを始める
    pub fn start(&self, pc: &Arc<Peercast>) -> bool {
        crate::log_info!("Peercast {}, {}", super::http::PCX_VERSTRING, PCX_OS_LINUX);
        crate::log_info!("SessionID: {}", ci::id_str(&self.session_id));
        crate::log_info!("BroadcastID: {}", ci::id_str(&pc.chanmgr.broadcast_id()));
        for a in sys::all_ip_addresses() {
            match Ip::parse(&a) {
                Some(ip) => {
                    crate::log_debug!("New address discovered: {}", String::from_utf8_lossy(&a));
                    self.update_ip_address(ip);
                }
                None => crate::log_debug!("\"{}\" could not be parsed", String::from_utf8_lossy(&a)),
            }
        }
        self.check_force_ip();
        let p = pc.clone();
        if !sys::start_thread(&self.server_thread, "SERVER", move || server_proc(&p)) {
            return false;
        }
        let p = pc.clone();
        if !sys::start_thread(&self.idle_thread, "IDLE", move || idle_proc(&p)) {
            return false;
        }
        true
    }

    /// `getState`
    pub fn state(&self, pc: &Peercast) -> Value {
        let s = self.settings().clone();
        let nf = s.filters.len().saturating_sub(1);
        let filters: Vec<Value> = s.filters[..nf].iter().map(|f| f.state()).collect();
        let servents: Vec<Value> = self.servents().iter().map(|sv| sv.state(pc)).collect();
        let addrs: Vec<Value> = s.server_ip_addresses.iter().map(|ip| super::state::s(ip.str())).collect();
        let fw4 = self.get_firewall(4);
        let fw6 = self.get_firewall(6);
        let cookies = self.cookies().clone();
        let install_dir = match sys::real_path(&pc.app.html_path) {
            Some(p) => p,
            None => {
                crate::log_error!("installationDirectory: realPath: No such file or directory");
                b"[Error]".to_vec()
            }
        };
        let ts = |v: u32| super::state::s(v.to_string());
        obj(vec![
            ("version", super::state::s(super::http::PCX_VERSTRING)),
            ("buildDateTime", super::state::s(super::app::BUILD_DATE_TIME)),
            ("uptime", super::state::s(crate::pcstring::from_stopwatch(self.uptime()))),
            ("numRelays", ts(self.num_streams_type(pc, servent::T_RELAY, true))),
            ("numDirect", ts(self.num_streams_type(pc, servent::T_DIRECT, true))),
            ("totalConnected", ts(self.num_connected())),
            ("numServHosts", ts(self.num_hosts(SH_SERVENT))),
            ("numServents", ts(self.num_servents())),
            ("servents", arr(servents)),
            ("serverName", super::state::s(&s.server_name.data)),
            ("serverPort", ts(s.server_host.port as u32)),
            ("serverIP", super::state::s(s.server_host.ip_str())),
            ("serverIPv6", super::state::s(s.server_host_ipv6.ip_str())),
            ("ypAddress", super::state::s(&s.root_host.data)),
            ("password", super::state::s(&s.password)),
            ("isFirewalled", flag(fw4 == FW_ON)),
            ("firewallKnown", flag(fw4 != FW_UNKNOWN)),
            ("isFirewalledIPv6", flag(fw6 == FW_ON)),
            ("firewallKnownIPv6", flag(fw6 != FW_UNKNOWN)),
            ("rootMsg", super::state::s(&s.root_msg.data)),
            ("isRoot", flag(self.is_root())),
            ("isPrivate", super::state::s("0")),
            ("forceYP", super::state::s("0")),
            ("refreshHTML", ts(if s.refresh_html != 0 { s.refresh_html } else { 0x0fff_ffff })),
            ("maxRelays", ts(s.max_relays)),
            ("maxDirect", ts(s.max_direct)),
            ("maxBitrateOut", ts(s.max_bitrate_out)),
            ("maxControlsIn", ts(s.max_control)),
            ("maxServIn", ts(s.max_serv_in)),
            ("handshakeTimeout", ts(s.handshake_timeout)),
            ("maxHandshakesPerIP", ts(s.max_handshakes_per_ip)),
            ("numFilters", super::state::s((nf as i32 + 1).to_string())),
            ("filters", arr(filters)),
            ("numActive1", ts(self.num_active_on_port(s.server_host.port as i32))),
            ("numCIN", ts(self.num_connected_type(pc, servent::T_CIN))),
            ("numCOUT", ts(self.num_connected_type(pc, servent::T_COUT))),
            ("numIncoming", ts(self.num_active(servent::T_INCOMING))),
            ("disabled", flag(s.is_disabled)),
            ("serverPort1", ts(s.server_host.port as u32)),
            ("serverLocalIP", super::state::s(s.server_local_ip.str())),
            ("serverLocalIPv6", super::state::s(s.server_local_ipv6.str())),
            ("upgradeURL", super::state::s(&s.download_url)),
            (
                "allow",
                obj(vec![
                    ("HTML1", flag(s.allow_server1 & servent::ALLOW_HTML != 0)),
                    ("broadcasting1", flag(s.allow_server1 & servent::ALLOW_BROADCAST != 0)),
                    ("network1", flag(s.allow_server1 & servent::ALLOW_NETWORK != 0)),
                    ("direct1", flag(s.allow_server1 & servent::ALLOW_DIRECT != 0)),
                ]),
            ),
            (
                "auth",
                obj(vec![
                    ("useCookies", flag(s.auth_type == AUTH_COOKIE)),
                    ("useHTTP", flag(s.auth_type == AUTH_HTTPBASIC)),
                    ("useSessionCookies", flag(!cookies.never_expire)),
                ]),
            ),
            ("log", obj(vec![("level", super::state::s(super::log::level().to_string()))])),
            ("lang", super::state::s(s.html_path.get(5..).unwrap_or(b""))),
            ("numExternalChannels", ts(self.channel_directory.num_channels() as u32)),
            ("numChannelFeedsPlusOne", n(self.channel_directory.num_feeds() + 1)),
            ("numChannelFeeds", n(self.channel_directory.num_feeds())),
            ("channelDirectory", self.channel_directory.state()),
            ("uptestServiceRegistry", self.uptest.state()),
            ("publicDirectoryEnabled", flag(s.public_directory_enabled)),
            ("transcodingEnabled", flag(s.transcoding_enabled)),
            ("preset", super::state::s(&s.preset)),
            ("audioCodec", super::state::s(&s.audio_codec)),
            ("maxTranscodes", ts(s.max_transcodes)),
            ("authFailLimit", ts(s.auth_fail_limit)),
            ("authLockSeconds", ts(s.auth_lock_seconds)),
            ("defaultChannelInfo", s.default_channel_info.state()),
            ("rtmpServerMonitor", self.rtmp_monitor.state()),
            ("rtmpPort", ts(s.rtmp_port as u32)),
            ("hasUnsafeFilterSettings", flag(self.has_unsafe_filter_settings())),
            ("chat", flag(s.chat)),
            ("randomizeBroadcastingChannelID", flag(self.flags.get("randomizeBroadcastingChannelID"))),
            ("flags", self.flags.state()),
            ("installationDirectory", super::state::s(install_dir)),
            ("configurationFile", super::state::s(&pc.app.ini_filename)),
            ("preferredTheme", super::state::s(&s.preferred_theme)),
            ("accentColor", super::state::s(&s.accent_color)),
            ("serverIPAddresses", arr(addrs)),
        ])
    }

    /// `rtmpServerMonitor.update` に渡す起動の引数 (`-p port url`)
    pub fn rtmp_server_args(&self, ip_version: i32) -> Vec<Vec<u8>> {
        let s = self.settings();
        let info = &s.default_channel_info;
        let mut q: Vec<(&[u8], Vec<u8>)> = vec![
            (b"comment", info.comment.data.clone()),
            (b"desc", info.desc.data.clone()),
            (b"genre", info.genre.data.clone()),
            (b"ipv", ip_version.to_string().into_bytes()),
            (b"name", info.name.data.clone()),
            (b"type", b"FLV".to_vec()),
            (b"url", info.url.data.clone()),
        ];
        // cgi::Query::str は名前の順
        q.sort_by(|a, b| a.0.cmp(b.0));
        let qs: Vec<Vec<u8>> = q.iter().map(|(k, v)| [&crate::cgi::escape(k)[..], b"=", &crate::cgi::escape(v)].concat()).collect();
        let url = [format!("http://localhost:{}/?", s.server_host.port).as_bytes(), &crate::strutil::join(b"&", &qs)].concat();
        vec![b"-p".to_vec(), s.rtmp_port.to_string().into_bytes(), url]
    }
}

/// `ensureCatchallFilters`: IPv4 と IPv6 のすべてのアドレスのフィルターがなければ足す。
/// 最後の要素は作業用 (`filters[numFilters]`)。
fn ensure_catchall(filters: &mut Vec<ServFilter>) {
    // 作業用の要素を除いた本物のフィルター
    let mut real: Vec<ServFilter> = if filters.is_empty() { Vec::new() } else { filters[..filters.len() - 1].to_vec() };
    let has4 = real.iter().any(|f| f.pattern() == b"255.255.255.255");
    let has6 = real.iter().any(|f| f.pattern() == b"::/0");
    if !has4 {
        let mut f = ServFilter::default();
        f.set_pattern(b"255.255.255.255");
        f.flags = sf::F_NETWORK | sf::F_DIRECT;
        real.push(f);
    }
    if !has6 {
        let mut f = ServFilter::default();
        f.set_pattern(b"::/0");
        f.flags = sf::F_NETWORK | sf::F_DIRECT;
        real.push(f);
    }
    real.push(ServFilter::default());
    *filters = real;
}

/// `writeRelayChannel`
fn relay_channel_section(pc: &Peercast, c: &super::channel::Channel) -> Section {
    let (info, src_url, stay, ipv) = {
        let st = c.st();
        (st.info.clone(), st.source_url.clone(), st.stay_connected, st.ip_version)
    };
    let mut sec = Section::new("RelayChannel", true);
    sec.push("name", &info.name.data[..]);
    sec.push("desc", &info.desc.data[..]);
    sec.push("genre", &info.genre.data[..]);
    sec.push("contactURL", &info.url.data[..]);
    sec.push("comment", &info.comment.data[..]);
    if !src_url.is_empty() {
        sec.push("sourceURL", &src_url.data[..]);
    }
    sec.push("sourceProtocol", ci::protocol_str(info.src_protocol));
    sec.push("contentType", info.type_str());
    sec.push("MIMEType", &info.mime_type.data[..]);
    sec.push("streamExt", &info.stream_ext.data[..]);
    sec.push("bitrate", info.bitrate);
    sec.push("id", ci::id_str(&info.id));
    sec.push("stayConnected", stay);
    let tracker = pc.chanmgr.with_hitlist_by_id(&info.id, |chl| {
        let mut chs = super::chanhit::ChanHitSearch { trackers_only: true, ..Default::default() };
        if chl.pick_hits(&mut chs) {
            Some(chs.best[0].host.str())
        } else {
            None
        }
    });
    if let Some(Some(t)) = tracker {
        sec.push("tracker", t);
    }
    sec.push("trackContact", &info.track.contact.data[..]);
    sec.push("trackTitle", &info.track.title.data[..]);
    sec.push("trackArtist", &info.track.artist.data[..]);
    sec.push("trackAlbum", &info.track.album.data[..]);
    sec.push("trackGenre", &info.track.genre.data[..]);
    sec.push("ipVersion", ipv);
    sec
}

/// `idleProc`
fn idle_proc(pc: &Arc<Peercast>) {
    let sm = &pc.servmgr;
    let mut last_broadcast_connect = 0u32;
    let mut last_root_broadcast = 0u32;
    let mut last_force_ip_check = 0u32;
    while sm.idle_thread.active() {
        super::stats::update();
        let ctime = sys::get_time();
        if !sm.settings().force_ip.is_empty() && ctime.wrapping_sub(last_force_ip_check) > 60 {
            if sm.check_force_ip() {
                pc.chanmgr.broadcast_tracker_update(pc, &[0; 16], true);
            }
            last_force_ip_check = ctime;
        }
        if pc.chanmgr.is_broadcasting() && ctime.wrapping_sub(last_broadcast_connect) > 30 {
            sm.connect_broadcaster(pc);
            last_broadcast_connect = ctime;
        }
        if sm.is_root() && ctime.wrapping_sub(last_root_broadcast) > pc.chanmgr.settings().host_update_interval {
            sm.broadcast_root_settings(pc, true);
            last_root_broadcast = ctime;
        }
        pc.chanmgr.clear_dead_hits(true);
        if sm.shutdown_timer.load(Ordering::SeqCst) != 0 && sm.shutdown_timer.fetch_sub(1, Ordering::SeqCst) - 1 <= 0 {
            pc.save_settings();
            pc.quit();
            super::app::exit(0);
        }
        if pc.chanmgr.num_idle_channels() > super::chanmgr::MAX_IDLE_CHANNELS {
            pc.chanmgr.close_oldest_idle();
        }
        let port = sm.settings().server_host.port;
        sm.channel_directory.update(port, super::directory::UpdateMode::Auto);
        sm.rtmp_monitor.update(|ipv| sm.rtmp_server_args(ipv));
        sm.uptest.update();
        sys::sleep(500);
    }
}

/// `serverProc`: 待ち受けのサーバントを動かし続ける
fn server_proc(pc: &Arc<Peercast>) {
    let sm = &pc.servmgr;
    let serv = sm.alloc_servent();
    while sm.server_thread.active() {
        if sm.restart_server.swap(false, Ordering::SeqCst) {
            serv.abort();
        }
        let (auto_serve, allow, force_normal, host) = {
            let s = sm.settings();
            (s.auto_serve, s.allow_server1, s.force_normal, s.server_host)
        };
        if auto_serve {
            serv.st().allow = allow;
            if !serv.has_sock() && !serv.thread.active() {
                crate::log_debug!("Starting servers");
                sm.set_firewall(4, if force_normal { FW_OFF } else { FW_UNKNOWN });
                if !servent::init_server(pc, &serv, host) {
                    crate::log_error!("Failed to start server on port {}. Exitting...", host.port);
                    pc.quit();
                    super::app::exit(0);
                }
            }
        } else {
            serv.abort();
            for s in sm.servents() {
                if s.ty() == servent::T_INCOMING {
                    s.thread.shutdown();
                }
            }
            sm.set_firewall(4, FW_ON);
            sm.set_firewall(6, FW_ON);
        }
        sys::sleep_idle();
    }
}

#[allow(dead_code)]
fn _unused(_: &dyn Stream) -> Option<String> {
    let _ = s("");
    None
}
