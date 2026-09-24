//! チャンネルとヒットリストの管理 (core/common/chanmgr.cpp の `ChanMgr`)。
//!
//! C++ 版はチャンネルとヒットリストを先頭に加える連結リストで持っていた。Rust 版は同じ並び
//! (新しいものが前) の `Vec`。ロックの順は、一覧のロックのあとにチャンネルのロック (逆にはしない)。

use std::sync::{Arc, Mutex, MutexGuard};

use super::channel::{self, Channel};
use super::chanhit::{ChanHit, ChanHitList, ChanHitSearch};
use super::chaninfo::{self as ci, ChanInfo};
use super::host::Host;
use super::packetbuf::ChanPacket;
use super::pcstr::PcString;
use super::peercast::Peercast;
use super::state::{arr, n, obj, Value};
use super::sys;

/// `ChanMgr` の設定と状態 (チャンネルとヒットリストの一覧以外)
#[derive(Clone, Debug)]
pub struct ChanMgrSettings {
    pub broadcast_id: [u8; 16],
    pub broadcast_msg: PcString,
    pub broadcast_msg_interval: u32,
    pub max_uptime: u32,
    pub dead_hit_age: u32,
    pub icy_meta_interval: i32,
    pub max_relays_per_channel: i32,
    pub min_broadcast_ttl: i32,
    pub max_broadcast_ttl: i32,
    pub push_timeout: i32,
    pub push_tries: i32,
    pub max_push_hops: i32,
    pub prefetch_time: u32,
    pub last_yp_connect: u32,
    pub icy_index: u32,
    pub host_update_interval: u32,
    pub buffer_time: u32,
    pub curr_find_and_play_channel: [u8; 16],
}

pub const MAX_IDLE_CHANNELS: i32 = 8;

/// `ChanMgr`
pub struct ChanMgr {
    channels: Mutex<Vec<Arc<Channel>>>,
    hitlists: Mutex<Vec<ChanHitList>>,
    pub s: Mutex<ChanMgrSettings>,
}

/// `GnuID::generate`: 乱数の ID (先頭のバイトは旗)
pub fn generate_id(flags: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    for b in id.iter_mut() {
        *b = sys::rnd() as u8;
    }
    id[0] = flags;
    id
}

/// `GnuID::random`: 0 にならない乱数の ID
pub fn random_id() -> [u8; 16] {
    let mut id = [0u8; 16];
    for b in id.iter_mut() {
        *b = sys::rnd() as u8;
    }
    if !ci::is_set(&id) {
        id[15] = 1;
    }
    id
}

impl Default for ChanMgr {
    fn default() -> Self {
        ChanMgr::new()
    }
}

impl ChanMgr {
    pub fn new() -> ChanMgr {
        ChanMgr {
            channels: Mutex::new(Vec::new()),
            hitlists: Mutex::new(Vec::new()),
            s: Mutex::new(ChanMgrSettings {
                broadcast_id: generate_id(0),
                broadcast_msg: PcString::default(),
                broadcast_msg_interval: 10,
                max_uptime: 0,
                dead_hit_age: 600,
                icy_meta_interval: 8192,
                max_relays_per_channel: 0,
                min_broadcast_ttl: 1,
                max_broadcast_ttl: 7,
                push_timeout: 60,
                push_tries: 5,
                max_push_hops: 8,
                prefetch_time: 10,
                last_yp_connect: 0,
                icy_index: 0,
                host_update_interval: 120,
                buffer_time: 5,
                curr_find_and_play_channel: [0; 16],
            }),
        }
    }

    pub fn settings(&self) -> MutexGuard<'_, ChanMgrSettings> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn broadcast_id(&self) -> [u8; 16] {
        self.settings().broadcast_id
    }

    pub fn max_uptime(&self) -> u32 {
        self.settings().max_uptime
    }

    fn chans(&self) -> MutexGuard<'_, Vec<Arc<Channel>>> {
        self.channels.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lists(&self) -> MutexGuard<'_, Vec<ChanHitList>> {
        self.hitlists.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// チャンネルの一覧の写し (新しいものが前)
    pub fn channels(&self) -> Vec<Arc<Channel>> {
        self.chans().clone()
    }

    /// ヒットリストの写し (`used` のもの)
    pub fn hitlists(&self) -> Vec<ChanHitList> {
        self.lists().iter().filter(|l| l.used).cloned().collect()
    }

    /// ヒットリストの一覧を直接触る (新しいものが前。`used` でないものも含む)
    pub fn with_hitlists<R>(&self, f: impl FnOnce(&mut Vec<ChanHitList>) -> R) -> R {
        f(&mut self.lists())
    }

    /// `findHitListByID` のヒットリストを触る (なければ `None`)
    pub fn with_hitlist_by_id<R>(&self, id: &[u8; 16], f: impl FnOnce(&mut ChanHitList) -> R) -> Option<R> {
        let mut l = self.lists();
        l.iter_mut().find(|c| c.used && c.info.id == *id).map(f)
    }

    /// `findHitList`: `matchNameID` で探す
    pub fn with_hitlist<R>(&self, info: &ChanInfo, f: impl FnOnce(&mut ChanHitList) -> R) -> Option<R> {
        let mut l = self.lists();
        l.iter_mut().find(|c| c.used && c.info.match_name_id(info)).map(f)
    }

    pub fn has_hitlist(&self, info: &ChanInfo) -> bool {
        self.with_hitlist(info, |_| ()).is_some()
    }

    pub fn has_hitlist_by_id(&self, id: &[u8; 16]) -> bool {
        self.with_hitlist_by_id(id, |_| ()).is_some()
    }

    /// `quit`
    pub fn quit(&self) {
        crate::log_debug!("ChanMgr is quitting..");
        self.close_all();
    }

    /// `numIdleChannels`
    pub fn num_idle_channels(&self) -> i32 {
        self.channels().iter().filter(|c| c.is_active() && c.thread.active() && c.status() == channel::S_IDLE).count() as i32
    }

    /// `closeIdles`: アイドルのチャンネルを止めて、終わるのを待つ
    pub fn close_idles(&self) {
        for ch in self.channels() {
            if ch.is_idle() {
                ch.thread.shutdown();
                ch.wait_thread();
            }
        }
    }

    /// `closeOldestIdle`
    pub fn close_oldest_idle(&self) {
        let chs = self.channels();
        let idle: Vec<bool> = chs.iter().map(|c| c.is_active() && c.thread.active() && c.status() == channel::S_IDLE).collect();
        let times: Vec<u32> = chs.iter().map(|c| c.st().last_idle_time).collect();
        if let Some(i) = crate::channel::oldest_idle(&idle, &times) {
            chs[i].thread.shutdown();
        }
    }

    /// `closeAll`
    pub fn close_all(&self) {
        for ch in self.channels() {
            if ch.thread.active() {
                ch.thread.shutdown();
            }
        }
    }

    /// `findChannelByNameID`
    pub fn find_channel_by_name_id(&self, info: &ChanInfo) -> Option<Arc<Channel>> {
        self.chans().iter().find(|c| c.is_active() && c.st().info.match_name_id(info)).cloned()
    }

    /// `findChannelByName` (大文字小文字を区別しない)
    pub fn find_channel_by_name(&self, name: &[u8]) -> Option<Arc<Channel>> {
        self.chans().iter().find(|c| c.is_active() && super::ini::stricmp_eq(&c.st().info.name.data, name)).cloned()
    }

    /// `findChannelByIndex`
    pub fn find_channel_by_index(&self, index: i32) -> Option<Arc<Channel>> {
        self.chans().iter().filter(|c| c.is_active()).nth(index.max(0) as usize).filter(|_| index >= 0).cloned()
    }

    /// `findChannelByMount`
    pub fn find_channel_by_mount(&self, mount: &[u8]) -> Option<Arc<Channel>> {
        self.chans().iter().find(|c| c.is_active() && c.st().mount.data == mount).cloned()
    }

    /// `findChannelByID`
    pub fn find_channel_by_id(&self, id: &[u8; 16]) -> Option<Arc<Channel>> {
        self.chans().iter().find(|c| c.is_active() && c.st().info.id == *id).cloned()
    }

    /// `findChannels`
    pub fn find_channels(&self, info: &ChanInfo, max: usize) -> Vec<Arc<Channel>> {
        self.chans().iter().filter(|c| c.is_active() && c.st().info.matches(info)).take(max).cloned().collect()
    }

    /// `findChannelsByStatus`
    pub fn find_channels_by_status(&self, max: usize, status: i32) -> Vec<Arc<Channel>> {
        self.chans().iter().filter(|c| c.is_active() && c.status() == status).take(max).cloned().collect()
    }

    /// `createChannel`
    pub fn create_channel(&self, pc: &Peercast, info: &ChanInfo, mount: Option<&[u8]>) -> Arc<Channel> {
        let nc = Arc::new(Channel::new());
        {
            let mut st = nc.st();
            st.info = info.clone();
            st.info.last_play_start = 0;
            st.info.last_play_end = 0;
            st.info.status = ci::S_UNKNOWN;
            if let Some(m) = mount {
                st.mount.assign(m);
            }
        }
        nc.set_status(pc, channel::S_WAIT);
        {
            let mut st = nc.st();
            st.ty = channel::T_ALLOCATED;
            st.info.created_time = sys::get_time();
            st.root_host = pc.servmgr.settings().root_host.data.clone();
        }
        self.chans().insert(0, nc.clone());
        crate::log_info!("New channel created");
        nc
    }

    /// `deleteChannel`
    pub fn delete_channel(&self, ch: &Arc<Channel>) {
        self.chans().retain(|c| !Arc::ptr_eq(c, ch));
    }

    /// `createRelay`
    pub fn create_relay(&self, pc: &Arc<Peercast>, info: &ChanInfo, stay_connected: bool) -> Option<Arc<Channel>> {
        let c = self.create_channel(pc, info, None);
        c.st().stay_connected = stay_connected;
        c.start_get(pc);
        Some(c)
    }

    /// `findAndRelay`: 最大 1 分、チャンネルが受信を始めるのを待つ
    pub fn find_and_relay(&self, pc: &Arc<Peercast>, info: &ChanInfo) -> Option<Arc<Channel>> {
        crate::log_info!("Searching for: {} ({})", ci::id_str(&info.id), String::from_utf8_lossy(&info.name.data));
        let mut c = self.find_channel_by_name_id(info);
        if c.is_none() {
            let nc = self.create_channel(pc, info, None);
            nc.set_status(pc, channel::S_SEARCHING);
            nc.start_get(pc);
        }
        for _ in 0..600 {
            c = self.find_channel_by_name_id(info);
            match &c {
                None => {
                    pc.notify_message(super::notif::NT_PEERCAST, format!("チャンネル {} は見付かりませんでした。", ch_name(info)).as_bytes());
                    return None;
                }
                Some(ch) => {
                    if ch.is_playing() {
                        break;
                    }
                }
            }
            sys::sleep(100);
        }
        c
    }

    /// `broadcastTrackerUpdate`
    pub fn broadcast_tracker_update(&self, pc: &Peercast, sv_id: &[u8; 16], force: bool) {
        for c in self.channels() {
            if c.is_active() && c.is_broadcasting() {
                c.broadcast_tracker_update(pc, sv_id, force);
            }
        }
    }

    /// `broadcastPacketUp`
    pub fn broadcast_packet_up(&self, pack: &ChanPacket, chan_id: &[u8; 16], src_id: &[u8; 16], dest_id: &[u8; 16]) -> i32 {
        self.channels().iter().filter(|c| c.send_packet_up(pack, chan_id, src_id, dest_id)).count() as i32
    }

    /// `setUpdateInterval`
    pub fn set_update_interval(&self, v: u32) {
        self.settings().host_update_interval = v;
    }

    /// `setBroadcastMsg`
    pub fn set_broadcast_msg(&self, pc: &Peercast, msg: &PcString) {
        {
            let mut s = self.settings();
            if msg.data == s.broadcast_msg.data {
                return;
            }
            s.broadcast_msg = msg.clone();
        }
        for c in self.channels() {
            if c.is_active() && c.is_broadcasting() {
                let mut new_info = c.info();
                new_info.comment = msg.clone();
                c.update_info(pc, &new_info);
            }
        }
    }

    /// `clearHitLists`
    pub fn clear_hit_lists(&self) {
        self.lists().clear();
    }

    /// `pickHits`: 使っているヒットリストのどれかから選ぶ
    pub fn pick_hits(&self, chs: &mut ChanHitSearch) -> bool {
        let mut l = self.lists();
        for chl in l.iter_mut() {
            if chl.used && chl.pick_hits(chs) {
                return true;
            }
        }
        false
    }

    /// `numHitLists`
    pub fn num_hit_lists(&self) -> i32 {
        self.lists().iter().filter(|l| l.used).count() as i32
    }

    /// `addHitList`: 先頭に加える
    pub fn add_hit_list(&self, info: &ChanInfo) {
        let mut chl = ChanHitList { used: true, info: info.clone(), ..Default::default() };
        chl.info.created_time = sys::get_time();
        self.lists().insert(0, chl);
    }

    /// `clearDeadHits`: 古いヒットを消し、空になって使っていないヒットリストも消す
    pub fn clear_dead_hits(&self, clear_trackers: bool) {
        const INTERVAL: u32 = 180;
        let mut empty: Vec<[u8; 16]> = Vec::new();
        {
            let mut l = self.lists();
            for chl in l.iter_mut() {
                if chl.used && chl.clear_dead_hits(INTERVAL, clear_trackers) == 0 {
                    empty.push(chl.info.id);
                }
            }
        }
        // チャンネルを見るので、ヒットリストのロックの外で決める
        let remove: Vec<[u8; 16]> = empty
            .into_iter()
            .filter(|id| !self.is_broadcasting_id(id) && self.find_channel_by_id(id).is_none())
            .collect();
        if !remove.is_empty() {
            // ロックを外していた間にヒットが加わったものは残す
            self.lists().retain(|chl| !(chl.used && remove.contains(&chl.info.id) && chl.hits.iter().all(|h| !h.host.ip.is_set())));
        }
    }

    /// `isBroadcasting(id)`
    pub fn is_broadcasting_id(&self, id: &[u8; 16]) -> bool {
        self.find_channel_by_id(id).map_or(false, |c| c.is_broadcasting())
    }

    /// `isBroadcasting()`
    pub fn is_broadcasting(&self) -> bool {
        self.channels().iter().any(|c| c.is_active() && c.is_broadcasting())
    }

    /// `numChannels`
    pub fn num_channels(&self) -> i32 {
        self.chans().iter().filter(|c| c.is_active()).count() as i32
    }

    /// `deadHit`
    pub fn dead_hit(&self, hit: &ChanHit) {
        self.with_hitlist_by_id(&hit.chan_id, |chl| chl.dead_hit(hit));
    }

    /// `delHit`
    pub fn del_hit(&self, hit: &ChanHit) {
        self.with_hitlist_by_id(&hit.chan_id, |chl| chl.del_hit(hit));
    }

    /// `addHit(Host&, const GnuID&, bool)`
    pub fn add_hit_host(&self, pc: &Peercast, h: Host, id: &[u8; 16], tracker: bool) {
        let mut hit = ChanHit::new();
        hit.host = h;
        hit.rhost[0] = h;
        hit.rhost[1] = Host::none();
        if !h.ip.is_ipv4_mapped() {
            hit.rhost[1].ip = super::host::Ip([0; 16]);
        }
        hit.tracker = tracker;
        hit.recv = true;
        hit.chan_id = *id;
        self.add_hit(pc, &hit);
    }

    /// `addHit(ChanHit&)`: ヒットリストがなければ作る
    pub fn add_hit(&self, pc: &Peercast, h: &ChanHit) -> Option<ChanHit> {
        let sid = pc.servmgr.session_id;
        let mut l = self.lists();
        if !l.iter().any(|c| c.used && c.info.id == h.chan_id) {
            let mut info = ChanInfo::new();
            info.id = h.chan_id;
            let mut chl = ChanHitList { used: true, info, ..Default::default() };
            chl.info.created_time = sys::get_time();
            l.insert(0, chl);
        }
        l.iter_mut().find(|c| c.used && c.info.id == h.chan_id).and_then(|chl| chl.add_hit(h, &sid))
    }

    /// `findAndPlayChannel`: スレッドでチャンネルを探して、見つかったらプレイヤーを起動する
    pub fn find_and_play_channel(&self, pc: &Arc<Peercast>, info: &ChanInfo, keep: bool) {
        let pc = pc.clone();
        let info = info.clone();
        let flag = sys::ThreadFlag::new();
        sys::start_thread(&flag, "findAndPlayChannelProc", move || {
            let cm = &pc.chanmgr;
            let mut ch = cm.find_channel_by_name_id(&info);
            cm.settings().curr_find_and_play_channel = info.id;
            if ch.is_none() {
                ch = cm.find_and_relay(&pc, &info);
            }
            if let Some(ch) = ch {
                let chinfo = ch.info();
                if cm.settings().curr_find_and_play_channel == chinfo.id {
                    cm.play_channel(&pc, &chinfo);
                }
                if keep {
                    ch.st().stay_connected = keep;
                }
            }
        });
    }

    /// `playChannel`: プレイリストを書いてプレイヤーで開く
    pub fn play_channel(&self, pc: &Peercast, info: &ChanInfo) {
        let base = format!("http://localhost:{}", pc.servmgr.settings().server_host.port);
        let (ty, fname) = if info.content_type.data == ci_const::T_OGM {
            (super::playlist::T_RAM, [&pc.app.cache_dir[..], b"/play.ram"].concat())
        } else {
            (super::playlist::T_SCPLS, [&pc.app.cache_dir[..], b"/play.pls"].concat())
        };
        let mut pls = super::playlist::PlayList::new(ty, 1);
        pls.add_channel(pc, base.as_bytes(), info);
        crate::log_debug!("Writing {}", String::from_utf8_lossy(&fname));
        if let Ok(mut f) = super::stream::FileStream::open_write(&fname) {
            let _ = pls.write(&mut f);
        }
        crate::log_debug!("Executing: {}", String::from_utf8_lossy(&fname));
        super::app::execute_file(&fname);
    }

    /// `authToken`
    pub fn auth_token(&self, id: &[u8; 16]) -> Vec<u8> {
        crate::channel::auth_token(&self.broadcast_id(), id)
    }

    /// `getState`
    pub fn state(&self, pc: &Peercast) -> Value {
        let chs: Vec<Value> = self.channels().iter().map(|c| c.state(pc)).collect();
        let s = self.settings().clone();
        obj(vec![
            ("channels", arr(chs)),
            ("numHitLists", n(self.num_hit_lists())),
            ("numChannels", n(self.num_channels())),
            ("djMessage", super::state::s(&s.broadcast_msg.data)),
            ("icyMetaInterval", n(s.icy_meta_interval)),
            ("maxRelaysPerChannel", n(s.max_relays_per_channel)),
            ("hostUpdateInterval", n(s.host_update_interval)),
            ("broadcastID", super::state::s(ci::id_str(&s.broadcast_id))),
        ])
    }
}

/// `chName`: 名前、なければ ID の頭
pub fn ch_name(info: &ChanInfo) -> String {
    if info.name.data.is_empty() {
        format!("{}...", &ci::id_str(&info.id)[..7])
    } else {
        String::from_utf8_lossy(&info.name.data).into_owned()
    }
}

mod ci_const {
    pub use crate::chaninfo::T_OGM;
}
