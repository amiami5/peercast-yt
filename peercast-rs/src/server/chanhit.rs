//! チャンネルを中継しているホストとその一覧 (core/common/chanhit.cpp の `ChanHit`、`ChanHitList`、
//! `ChanHitSearch`、gnuid.cpp の `GnuIDList`)。数え上げや選び方は段階 7b の `crate::chanhit`。
//!
//! C++ 版の一覧は先頭に加える連結リストだった。Rust 版は同じ並び (新しいものが前) の `Vec`。

use super::chaninfo::{id_str, ChanInfo};
use super::host::{Host, Ip};
use super::state::{obj, s, Value};
use super::sys;
use super::xmlnode::XmlNode;
use crate::chanhit as ch;
use crate::pcp::write::AtomBuf;

/// `ChanHit`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChanHit {
    pub host: Host,
    pub rhost: [Host; 2],
    pub num_listeners: u32,
    pub num_relays: u32,
    pub num_hops: u32,
    pub time: u32,
    pub up_time: u32,
    pub last_contact: u32,
    pub session_id: [u8; 16],
    pub chan_id: [u8; 16],
    pub version: u32,
    pub oldest_pos: u32,
    pub newest_pos: u32,
    pub firewalled: bool,
    pub stable: bool,
    pub tracker: bool,
    pub recv: bool,
    pub yp: bool,
    pub dead: bool,
    pub direct: bool,
    pub relay: bool,
    pub cin: bool,
    pub uphost: Host,
    pub uphost_hops: u32,
    pub version_vp: u32,
    pub version_ex_prefix: [u8; 2],
    pub version_ex_number: u32,
}

impl Default for ChanHit {
    /// `init`
    fn default() -> Self {
        ChanHit {
            host: Host::none(),
            rhost: [Host::none(), Host::none()],
            num_listeners: 0,
            num_relays: 0,
            num_hops: 0,
            time: 0,
            up_time: 0,
            last_contact: 0,
            session_id: [0; 16],
            chan_id: [0; 16],
            version: 0,
            oldest_pos: 0,
            newest_pos: 0,
            firewalled: false,
            stable: false,
            tracker: false,
            // C++ 版は recv、cin、direct、relay を true にし、そのあと direct を 0 にしている
            recv: true,
            yp: false,
            dead: false,
            direct: false,
            relay: true,
            cin: true,
            uphost: Host::none(),
            uphost_hops: 0,
            version_vp: 0,
            version_ex_prefix: *b"  ",
            version_ex_number: 0,
        }
    }
}

fn to_ch_host(h: &Host) -> ch::Host {
    ch::Host { ip: h.ip.0, port: h.port }
}

impl ChanHit {
    pub fn new() -> ChanHit {
        ChanHit::default()
    }

    /// 段階 7b の判断に渡す形
    pub fn view(&self) -> ch::Hit {
        ch::Hit {
            host: to_ch_host(&self.host),
            rhost: [to_ch_host(&self.rhost[0]), to_ch_host(&self.rhost[1])],
            num_listeners: self.num_listeners,
            num_relays: self.num_relays,
            num_hops: self.num_hops,
            time: self.time,
            up_time: self.up_time,
            last_contact: self.last_contact,
            session_id: self.session_id,
            version: self.version,
            oldest_pos: self.oldest_pos,
            newest_pos: self.newest_pos,
            firewalled: self.firewalled,
            tracker: self.tracker,
            recv: self.recv,
            dead: self.dead,
            direct: self.direct,
            relay: self.relay,
            cin: self.cin,
            uphost: to_ch_host(&self.uphost),
            uphost_hops: self.uphost_hops,
            version_vp: self.version_vp,
            version_ex_prefix: self.version_ex_prefix,
            version_ex_number: self.version_ex_number,
        }
    }

    /// `pickNearestIP`
    pub fn pick_nearest_ip(&mut self, h: &Host) {
        for i in 0..2 {
            if h.global_ip() == self.rhost[i].global_ip() {
                self.host = self.rhost[i];
                break;
            }
        }
    }

    /// `writeAtoms`
    pub fn write_atoms(&self, out: &mut AtomBuf, chan_id: &[u8; 16]) {
        ch::write_atoms(out, &self.view(), chan_id);
    }

    /// `versionString`
    pub fn version_string(&self) -> Vec<u8> {
        ch::version_string(&self.view())
    }

    /// `str`: ホストと版
    pub fn str(&self, with_port: bool) -> String {
        let mut r = if with_port { self.host.str() } else { self.host.ip_str() };
        let v = self.version_string();
        if !v.is_empty() {
            r.push_str(&format!(" ({})", String::from_utf8_lossy(&v)));
        }
        r
    }

    /// `getColor` (0 red、1 purple、2 blue、3 green)
    pub fn color(&self) -> i32 {
        ch::color(&self.view())
    }

    /// `colorToName`
    pub fn color_name(c: i32) -> &'static str {
        match c {
            0 => "red",
            1 => "purple",
            2 => "blue",
            3 => "green",
            _ => "unknown",
        }
    }

    /// `canGiv`
    pub fn can_giv(&self) -> bool {
        ch::can_giv(&self.view())
    }

    /// `getState`
    pub fn state(&self) -> Value {
        let ver = self.version_string();
        obj(vec![
            ("rhost0", s(self.rhost[0].str())),
            ("rhost1", s(self.rhost[1].str())),
            ("numHops", s(self.num_hops.to_string())),
            ("numListeners", s(self.num_listeners.to_string())),
            ("numRelays", s(self.num_relays.to_string())),
            ("uptime", s(crate::pcstring::from_stopwatch(self.up_time))),
            (
                "update",
                s(if self.time != 0 { crate::pcstring::from_stopwatch(sys::get_time().wrapping_sub(self.time)) } else { b"-".to_vec() }),
            ),
            ("isFirewalled", s(if self.firewalled { "1" } else { "0" })),
            ("version", s(if ver.is_empty() { b"-".to_vec() } else { ver })),
            ("tracker", s(if self.tracker { "1" } else { "0" })),
        ])
    }

    /// `createXML`
    pub fn xml(&self) -> XmlNode {
        XmlNode::new(format!(
            "host ip=\"{}\" hops=\"{}\" listeners=\"{}\" relays=\"{}\" uptime=\"{}\" push=\"{}\" relay=\"{}\" direct=\"{}\" cin=\"{}\" stable=\"{}\" version=\"{}\" update=\"{}\" tracker=\"{}\"",
            self.host.str(),
            self.num_hops as i32,
            self.num_listeners as i32,
            self.num_relays as i32,
            self.up_time as i32,
            self.firewalled as i32,
            self.relay as i32,
            self.direct as i32,
            self.cin as i32,
            self.stable as i32,
            self.version as i32,
            sys::get_time().wrapping_sub(self.time) as i32,
            self.tracker as i32
        ))
    }
}

/// `ChanHitSearch`
#[derive(Clone, Debug)]
pub struct ChanHitSearch {
    pub best: Vec<ChanHit>,
    pub match_host: Host,
    pub wait_delay: u32,
    pub use_firewalled: bool,
    pub trackers_only: bool,
    pub use_busy_relays: bool,
    pub use_busy_controls: bool,
    pub exclude_id: [u8; 16],
}

impl Default for ChanHitSearch {
    /// `init`
    fn default() -> Self {
        ChanHitSearch {
            best: Vec::new(),
            match_host: Host::none(),
            wait_delay: 0,
            use_firewalled: false,
            trackers_only: false,
            use_busy_relays: true,
            use_busy_controls: true,
            exclude_id: [0; 16],
        }
    }
}

/// `ChanHitList`
#[derive(Clone, Debug, Default)]
pub struct ChanHitList {
    pub used: bool,
    pub info: ChanInfo,
    /// 新しいものが前
    pub hits: Vec<ChanHit>,
    pub last_hit_time: u32,
}

impl ChanHitList {
    fn views(&self) -> Vec<ch::Hit> {
        self.hits.iter().map(|h| h.view()).collect()
    }

    fn count(&self, op: ch::Count) -> u32 {
        ch::count(&self.views(), op)
    }

    pub fn num_hits(&self) -> i32 {
        self.count(ch::Count::NumHits) as i32
    }
    pub fn num_listeners(&self) -> i32 {
        self.count(ch::Count::NumListeners) as i32
    }
    pub fn num_relays(&self) -> i32 {
        self.count(ch::Count::NumRelays) as i32
    }
    pub fn num_trackers(&self) -> i32 {
        self.count(ch::Count::NumTrackers) as i32
    }
    pub fn num_firewalled(&self) -> i32 {
        self.count(ch::Count::NumFirewalled) as i32
    }
    pub fn closest_hit(&self) -> i32 {
        self.count(ch::Count::ClosestHit) as i32
    }
    pub fn furthest_hit(&self) -> i32 {
        self.count(ch::Count::FurthestHit) as i32
    }
    pub fn newest_hit(&self) -> u32 {
        self.count(ch::Count::NewestHit)
    }
    pub fn total_listeners(&self) -> i32 {
        self.count(ch::Count::TotalListeners) as i32
    }
    pub fn total_relays(&self) -> i32 {
        self.count(ch::Count::TotalRelays) as i32
    }
    pub fn total_firewalled(&self) -> i32 {
        self.count(ch::Count::TotalFirewalled) as i32
    }

    /// `addHit`: 同じアドレスのものがあれば書き換え、なければ先頭に加える。自分なら加えない
    /// (`None`)。加えたか書き換えたものを返す。
    pub fn add_hit(&mut self, h: &ChanHit, my_session_id: &[u8; 16]) -> Option<ChanHit> {
        crate::log_debug!("Add hit: {}/{}", h.rhost[0].str(), h.rhost[1].str());
        let views = self.views();
        let mut del = vec![false; views.len()];
        let r = ch::add(&views, &h.view(), my_session_id, &mut del);
        if r == ch::Add::Own {
            return None;
        }
        self.last_hit_time = sys::get_time();
        let mut h = h.clone();
        h.time = self.last_hit_time;
        match r {
            ch::Add::Replace(i) => {
                self.hits[i] = h.clone();
                Some(h)
            }
            _ => {
                let mut i = 0;
                self.hits.retain(|_| {
                    let keep = !del[i];
                    i += 1;
                    keep
                });
                h.chan_id = self.info.id;
                self.hits.insert(0, h.clone());
                Some(h)
            }
        }
    }

    /// `clearDeadHits`: 古いヒットを消し、残った数を返す
    pub fn clear_dead_hits(&mut self, timeout: u32, clear_trackers: bool) -> i32 {
        let views = self.views();
        let mut del = vec![false; views.len()];
        let cnt = ch::clear_dead(&views, timeout, clear_trackers, sys::get_time(), &mut del);
        let mut i = 0;
        self.hits.retain(|_| {
            let keep = !del[i];
            i += 1;
            keep
        });
        cnt
    }

    /// `deadHit`
    pub fn dead_hit(&mut self, h: &ChanHit) {
        crate::log_debug!("Dead hit: {}/{}", h.rhost[0].str(), h.rhost[1].str());
        let views = self.views();
        let mut same = vec![false; views.len()];
        ch::same_hosts(&views, &h.view(), &mut same);
        for (i, x) in self.hits.iter_mut().enumerate() {
            if same[i] {
                x.dead = true;
            }
        }
    }

    /// `delHit`
    pub fn del_hit(&mut self, h: &ChanHit) {
        crate::log_debug!("Del hit: {}/{}", h.rhost[0].str(), h.rhost[1].str());
        let views = self.views();
        let mut same = vec![false; views.len()];
        ch::same_hosts(&views, &h.view(), &mut same);
        let mut i = 0;
        self.hits.retain(|_| {
            let keep = !same[i];
            i += 1;
            keep
        });
    }

    /// `pickHits`: 条件に合うホストを `chs.best` に加える。選べば true
    pub fn pick_hits(&mut self, chs: &mut ChanHitSearch) -> bool {
        let views = self.views();
        let s = ch::Search {
            match_host: to_ch_host(&chs.match_host),
            wait_delay: chs.wait_delay,
            use_firewalled: chs.use_firewalled,
            trackers_only: chs.trackers_only,
            use_busy_relays: chs.use_busy_relays,
            use_busy_controls: chs.use_busy_controls,
            exclude_id: chs.exclude_id,
            num_results: chs.best.len() as i32,
        };
        let ctime = sys::get_time();
        match ch::pick(&views, &s, ctime) {
            None => false,
            Some(p) => {
                let mut best = self.hits[p.index].clone();
                best.host = best.rhost[if p.lan { 1 } else { 0 }];
                if chs.wait_delay != 0 {
                    self.hits[p.index].last_contact = ctime;
                }
                chs.best.push(best);
                true
            }
        }
    }

    /// `createXML`
    pub fn xml(&self, add_hits: bool) -> XmlNode {
        let mut n = XmlNode::new(format!(
            "hits hosts=\"{}\" listeners=\"{}\" relays=\"{}\" firewalled=\"{}\" closest=\"{}\" furthest=\"{}\" newest=\"{}\"",
            self.num_hits(),
            self.num_listeners(),
            self.num_relays(),
            self.num_firewalled(),
            self.closest_hit(),
            self.furthest_hit(),
            sys::get_time().wrapping_sub(self.newest_hit()) as i32
        ));
        if add_hits {
            for h in &self.hits {
                if h.host.ip.is_set() {
                    n.add(h.xml());
                }
            }
        }
        n
    }

    pub fn id_str(&self) -> String {
        id_str(&self.info.id)
    }
}

/// `GnuIDList`: 最近見た ID (決まった数まで、古いものから入れ替える)
#[derive(Clone, Debug)]
pub struct GnuIdList {
    ids: Vec<([u8; 16], u32)>,
}

impl GnuIdList {
    pub fn new(max: usize) -> GnuIdList {
        GnuIdList { ids: vec![([0; 16], 0); max] }
    }

    pub fn clear(&mut self) {
        for e in self.ids.iter_mut() {
            *e = ([0; 16], 0);
        }
    }

    /// `contains` (空いている欄は 0 の ID として一致する)
    pub fn contains(&self, id: &[u8; 16]) -> bool {
        self.ids.iter().any(|(x, _)| x == id)
    }

    /// `add`
    pub fn add(&mut self, id: &[u8; 16]) {
        let now = sys::get_time();
        let mut min_time = u32::MAX;
        let mut min_index = 0;
        for (i, e) in self.ids.iter_mut().enumerate() {
            if e.0 == *id {
                e.1 = now;
                return;
            }
            if e.1 <= min_time {
                min_time = e.1;
                min_index = i;
            }
        }
        if let Some(e) = self.ids.get_mut(min_index) {
            *e = (*id, now);
        }
    }
}

/// `Host(ip, port)` で IP が `::` のもの (`IP::parse("::")`)
pub fn any_v6(port: u16) -> Host {
    Host::new(Ip([0; 16]), port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(n: u8) -> ChanHit {
        let mut h = ChanHit::new();
        h.host = Host::v4(0x0a000000 + n as u32, 7144);
        h.rhost = [h.host, Host::v4(0xc0a80000 + n as u32, 7144)];
        h.session_id = [n; 16];
        h
    }

    #[test]
    fn list_ops() {
        let mut l = ChanHitList { used: true, ..Default::default() };
        l.info.id = [9; 16];
        assert!(l.add_hit(&hit(1), &[0xff; 16]).is_some());
        assert!(l.add_hit(&hit(2), &[0xff; 16]).is_some());
        assert_eq!(l.num_hits(), 2);
        // 新しいものが前
        assert_eq!(l.hits[0].session_id, [2; 16]);
        assert_eq!(l.hits[0].chan_id, [9; 16]);
        // 自分は加えない
        assert!(l.add_hit(&hit(3), &[3; 16]).is_none());
        // 同じアドレスは書き換え
        let mut h = hit(1);
        h.num_listeners = 5;
        l.add_hit(&h, &[0xff; 16]);
        assert_eq!(l.hits.len(), 2);
        assert_eq!(l.num_listeners(), 5);
        let mut chs = ChanHitSearch { exclude_id: [0xff; 16], ..Default::default() };
        assert!(l.pick_hits(&mut chs));
        assert_eq!(chs.best.len(), 1);
        l.del_hit(&hit(2));
        assert_eq!(l.hits.len(), 1);
        l.dead_hit(&hit(1));
        assert_eq!(l.num_hits(), 0);
        assert_eq!(l.clear_dead_hits(180, true), 0);
        assert!(l.hits.is_empty());
    }

    #[test]
    fn id_list() {
        let mut l = GnuIdList::new(3);
        assert!(l.contains(&[0; 16]));
        l.add(&[1; 16]);
        assert!(l.contains(&[1; 16]));
    }
}
