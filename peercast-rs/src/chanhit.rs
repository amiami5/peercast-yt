//! チャンネルを中継しているホスト (core/common/chanhit.cpp の `ChanHit` と `ChanHitList`) のうち、
//! 状態を持たない部分。host atom の組み立て、版の文字列、色、ホストの一覧の数え上げ、次につなぐ
//! ホストの選び方 (`pickHits`)、追加・削除するホストの判断。
//!
//! 一覧 (`ChanHitList::hit` の連結リスト) は、今は C++ 側にある。Rust はその並びどおりの配列を
//! 受け取り、どれをどうするかを返す。

use crate::pcp::write::{is_ipv4_mapped, AtomBuf};
use crate::pcp::*;

/// `Host` (IP アドレスは `IP` の 16 バイトの中身)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Host {
    pub ip: [u8; 16],
    pub port: u16,
}

impl Host {
    /// `IP::isAny` (全部 0 か、`::ffff:0.0.0.0`)
    pub fn ip_is_any(&self) -> bool {
        self.ip.iter().all(|&b| b == 0) || (is_ipv4_mapped(&self.ip) && self.ip[12..].iter().all(|&b| b == 0))
    }

    /// `if (host.ip)` と `Host::isValid`
    pub fn ip_set(&self) -> bool {
        !self.ip_is_any()
    }
}

/// `ChanHit` のうち、ここで使う欄
#[derive(Clone, Copy, Debug, Default)]
pub struct Hit {
    pub host: Host,
    pub rhost: [Host; 2],
    pub num_listeners: u32,
    pub num_relays: u32,
    pub num_hops: u32,
    pub time: u32,
    pub up_time: u32,
    pub last_contact: u32,
    pub session_id: [u8; 16],
    pub version: u32,
    pub oldest_pos: u32,
    pub newest_pos: u32,
    pub firewalled: bool,
    pub tracker: bool,
    pub recv: bool,
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

fn is_set(id: &[u8; 16]) -> bool {
    id.iter().any(|&b| b != 0)
}

const FLAGS1_TRACKER: u8 = 0x01;
const FLAGS1_RELAY: u8 = 0x02;
const FLAGS1_DIRECT: u8 = 0x04;
const FLAGS1_PUSH: u8 = 0x08;
const FLAGS1_RECV: u8 = 0x10;
const FLAGS1_CIN: u8 = 0x20;

fn flags1(h: &Hit) -> u8 {
    let mut f = 0;
    for (on, bit) in [
        (h.recv, FLAGS1_RECV),
        (h.relay, FLAGS1_RELAY),
        (h.direct, FLAGS1_DIRECT),
        (h.cin, FLAGS1_CIN),
        (h.tracker, FLAGS1_TRACKER),
        (h.firewalled, FLAGS1_PUSH),
    ] {
        if on {
            f |= bit;
        }
    }
    f
}

/// `ChanHit::writeAtoms`
pub fn write_atoms(out: &mut AtomBuf, h: &Hit, chan_id: &[u8; 16]) {
    let add_chan = is_set(chan_id);
    let up = h.uphost.ip_set();
    let ex = h.version_ex_number != 0;
    out.parent(PCP_HOST, 13 + add_chan as i32 + if up { 3 } else { 0 } + if ex { 2 } else { 0 });
    if add_chan {
        out.bytes(PCP_HOST_CHANID, chan_id);
    }
    out.bytes(PCP_HOST_ID, &h.session_id);
    out.address(PCP_HOST_IP, &h.rhost[0].ip);
    out.short(PCP_HOST_PORT, h.rhost[0].port as i16);
    out.address(PCP_HOST_IP, &h.rhost[1].ip);
    out.short(PCP_HOST_PORT, h.rhost[1].port as i16);
    out.int(PCP_HOST_NUML, h.num_listeners as i32);
    out.int(PCP_HOST_NUMR, h.num_relays as i32);
    out.int(PCP_HOST_UPTIME, h.up_time as i32);
    out.int(PCP_HOST_VERSION, h.version as i32);
    out.int(PCP_HOST_VERSION_VP, h.version_vp as i32);
    if ex {
        out.bytes(PCP_HOST_VERSION_EX_PREFIX, &h.version_ex_prefix);
        out.short(PCP_HOST_VERSION_EX_NUMBER, h.version_ex_number as i16);
    }
    out.char(PCP_HOST_FLAGS1, flags1(h));
    out.int(PCP_HOST_OLDPOS, h.oldest_pos as i32);
    out.int(PCP_HOST_NEWPOS, h.newest_pos as i32);
    if up {
        out.address(PCP_HOST_UPHOST_IP, &h.uphost.ip);
        out.int(PCP_HOST_UPHOST_PORT, h.uphost.port as i32);
        out.int(PCP_HOST_UPHOST_HOPS, h.uphost_hops as i32);
    }
}

/// `versionString`
pub fn version_string(h: &Hit) -> Vec<u8> {
    if h.version == 0 {
        Vec::new()
    } else if h.version_vp == 0 {
        h.version.to_string().into_bytes()
    } else if h.version_ex_number == 0 {
        format!("VP{}", h.version_vp).into_bytes()
    } else {
        let mut v = h.version_ex_prefix.to_vec();
        v.extend_from_slice(h.version_ex_number.to_string().as_bytes());
        v
    }
}

/// `ChanHit::Color`
pub const COLOR_RED: i32 = 0;
pub const COLOR_PURPLE: i32 = 1;
pub const COLOR_BLUE: i32 = 2;
pub const COLOR_GREEN: i32 = 3;

/// `getColor`
pub fn color(h: &Hit) -> i32 {
    if h.host.port == 0 {
        COLOR_RED
    } else if !h.relay {
        if h.num_relays == 0 {
            COLOR_PURPLE
        } else {
            COLOR_BLUE
        }
    } else {
        COLOR_GREEN
    }
}

/// `canGiv` (PeerCastStation 以外)
pub fn can_giv(h: &Hit) -> bool {
    &h.version_ex_prefix != b"ST"
}

// ---- 一覧 ----

/// 生きている (IP があって、死んでいない) ホスト
fn live(h: &Hit) -> bool {
    h.host.ip_set() && !h.dead
}

/// 数え上げの種類 (`pcrs_hits_count` の op)
#[derive(Clone, Copy, Debug)]
pub enum Count {
    NumHits,
    NumListeners,
    NumRelays,
    NumTrackers,
    NumFirewalled,
    ClosestHit,
    FurthestHit,
    NewestHit,
    TotalListeners,
    TotalRelays,
    TotalFirewalled,
}

/// `numHits` などの数え上げ。C++ 版の `int` と `unsigned int` の足し算と同じく一周する。
/// 返り値は C++ 版の返り値の型のビット列 (`newestHit` だけ `unsigned int`)。
pub fn count(hits: &[Hit], op: Count) -> u32 {
    let sum = |f: &dyn Fn(&Hit) -> Option<u32>| hits.iter().filter_map(f).fold(0u32, |a, b| a.wrapping_add(b));
    match op {
        Count::NumHits => sum(&|h| live(h).then_some(1)),
        Count::NumListeners => sum(&|h| live(h).then_some(h.num_listeners)),
        Count::NumRelays => sum(&|h| live(h).then_some(h.num_relays)),
        Count::NumTrackers => sum(&|h| (live(h) && h.tracker).then_some(1)),
        Count::NumFirewalled => sum(&|h| (live(h) && h.firewalled).then_some(1)),
        Count::ClosestHit => hits.iter().filter(|h| live(h)).map(|h| h.num_hops).fold(10000, u32::min),
        Count::FurthestHit => hits.iter().filter(|h| live(h)).map(|h| h.num_hops).fold(0, u32::max),
        Count::NewestHit => hits.iter().filter(|h| live(h)).map(|h| h.time).fold(0, u32::max),
        Count::TotalListeners => sum(&|h| h.host.ip_set().then_some(h.num_listeners)),
        Count::TotalRelays => sum(&|h| h.host.ip_set().then_some(h.num_relays)),
        Count::TotalFirewalled => sum(&|h| (h.host.ip_set() && h.firewalled).then_some(1)),
    }
}

/// `ChanHitSearch` の条件
#[derive(Clone, Copy, Debug, Default)]
pub struct Search {
    pub match_host: Host,
    pub wait_delay: u32,
    pub use_firewalled: bool,
    pub trackers_only: bool,
    pub use_busy_relays: bool,
    pub use_busy_controls: bool,
    pub exclude_id: [u8; 16],
    pub num_results: i32,
}

/// `ChanHitSearch::MAX_RESULTS`
pub const MAX_RESULTS: i32 = 8;

/// `pickHits` で選んだホスト
#[derive(Debug, PartialEq, Eq)]
pub struct Pick {
    pub index: usize,
    /// LAN 側のアドレス (`rhost[1]`) を使うか。false なら WAN 側 (`rhost[0]`)
    pub lan: bool,
}

/// `pickHits`: 条件に合うホストのうち、ホップ数が一番少ない (同じなら後ろの) ものを選ぶ。
/// 結果が `MAX_RESULTS` 個たまっていれば選ばない。
pub fn pick(hits: &[Hit], s: &Search, ctime: u32) -> Option<Pick> {
    let mut best_hops = 255u32;
    let mut best = None;
    for (i, c) in hits.iter().enumerate() {
        if !live(c) {
            continue;
        }
        if s.exclude_id == c.session_id {
            continue;
        }
        if !(s.wait_delay == 0 || ctime.wrapping_sub(c.last_contact) >= s.wait_delay) {
            continue;
        }
        if c.num_hops >= best_hops {
            continue;
        }
        if !(c.relay || s.use_busy_relays) || !(c.cin || s.use_busy_controls) {
            continue;
        }
        if s.trackers_only != c.tracker {
            continue;
        }
        let found = if s.match_host.ip_set() {
            (c.rhost[0].ip == s.match_host.ip && c.rhost[1].ip_set()).then_some(true)
        } else {
            (c.firewalled == s.use_firewalled).then_some(false)
        };
        if let Some(lan) = found {
            best = Some(Pick { index: i, lan });
            best_hops = c.num_hops;
        }
    }
    best.filter(|_| s.num_results < MAX_RESULTS)
}

/// `clearDeadHits`: 消すホストに印を付け、残るホストの数 (IP のあるもの) を返す
pub fn clear_dead(hits: &[Hit], timeout: u32, clear_trackers: bool, ctime: u32, del: &mut [bool]) -> i32 {
    let mut cnt = 0i32;
    for (i, h) in hits.iter().enumerate() {
        del[i] = false;
        if h.host.ip_set() {
            let old_enough = ctime.wrapping_sub(h.time) > timeout;
            if h.dead || (old_enough && (clear_trackers || !h.tracker)) {
                del[i] = true;
            } else {
                cnt = cnt.wrapping_add(1);
            }
        }
    }
    cnt
}

/// `deadHit` と `delHit` の対象: IP があり、`rhost` が 2 つとも同じホスト
pub fn same_hosts(hits: &[Hit], h: &Hit, out: &mut [bool]) {
    for (i, c) in hits.iter().enumerate() {
        out[i] = c.host.ip_set() && c.rhost == h.rhost;
    }
}

/// `addHit` の判断
#[derive(Debug, PartialEq, Eq)]
pub enum Add {
    /// 自分のホストなので加えない
    Own,
    /// この番号のホストを書き換える
    Replace(usize),
    /// `del` に印を付けたホスト (同じセッション ID のもの) を消し、先頭に加える
    New,
}

/// `addHit`: 同じアドレスの生きているホストがあれば書き換え、なければ同じセッション ID のホストを
/// 消して加える。
pub fn add(hits: &[Hit], h: &Hit, my_session_id: &[u8; 16], del: &mut [bool]) -> Add {
    if *my_session_id == h.session_id {
        return Add::Own;
    }
    for (i, c) in hits.iter().enumerate() {
        if c.rhost[0] == h.rhost[0] && (c.rhost[1] == h.rhost[1] || !c.rhost[1].ip_set()) && !c.dead {
            return Add::Replace(i);
        }
    }
    let by_session = is_set(&h.session_id);
    for (i, c) in hits.iter().enumerate() {
        del[i] = by_session && c.host.ip_set() && c.session_id == h.session_id;
    }
    Add::New
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8, port: u16) -> Host {
        let mut ip = [0u8; 16];
        ip[10] = 0xff;
        ip[11] = 0xff;
        ip[12..].copy_from_slice(&[a, b, c, d]);
        Host { ip, port }
    }

    fn hit(n: u8, hops: u32) -> Hit {
        let h = v4(10, 0, 0, n, 7144);
        // セッション ID が 0 だと、除外する ID (既定は 0) と同じになり選ばれない (C++ 版と同じ)
        Hit {
            host: h,
            rhost: [h, v4(192, 168, 0, n, 7144)],
            num_hops: hops,
            session_id: [n; 16],
            relay: true,
            cin: true,
            recv: true,
            ..Default::default()
        }
    }

    #[test]
    fn any_ip() {
        assert!(!Host::default().ip_set());
        assert!(!v4(0, 0, 0, 0, 1).ip_set());
        assert!(v4(0, 0, 0, 1, 1).ip_set());
    }

    #[test]
    fn versions_and_colors() {
        let mut h = hit(1, 0);
        assert_eq!(version_string(&h), b"");
        h.version = 1218;
        assert_eq!(version_string(&h), b"1218");
        h.version_vp = 27;
        assert_eq!(version_string(&h), b"VP27");
        h.version_ex_prefix = *b"YT";
        h.version_ex_number = 12;
        assert_eq!(version_string(&h), b"YT12");
        assert!(can_giv(&h));
        h.version_ex_prefix = *b"ST";
        assert!(!can_giv(&h));
        assert_eq!(color(&h), COLOR_GREEN);
        h.relay = false;
        assert_eq!(color(&h), COLOR_PURPLE);
        h.num_relays = 1;
        assert_eq!(color(&h), COLOR_BLUE);
        h.host.port = 0;
        assert_eq!(color(&h), COLOR_RED);
    }

    #[test]
    fn picks_fewest_hops() {
        let mut hits = vec![hit(1, 3), hit(2, 1), hit(3, 1), hit(4, 0)];
        hits[3].dead = true;
        let s = Search { use_busy_relays: true, use_busy_controls: true, ..Default::default() };
        assert_eq!(pick(&hits, &s, 0), Some(Pick { index: 1, lan: false }));
        let s2 = Search { match_host: v4(10, 0, 0, 3, 0), ..s };
        assert_eq!(pick(&hits, &s2, 0), Some(Pick { index: 2, lan: true }));
        assert_eq!(pick(&hits, &Search { trackers_only: true, ..s }, 0), None);
        assert_eq!(pick(&hits, &Search { num_results: 8, ..s }, 0), None);
        assert_eq!(count(&hits, Count::NumHits), 3);
        assert_eq!(count(&hits, Count::ClosestHit), 1);
        assert_eq!(count(&hits, Count::FurthestHit), 3);
    }

    #[test]
    fn add_replaces_or_inserts() {
        let hits = vec![hit(1, 0), hit(2, 0)];
        let mut del = vec![false; 2];
        assert_eq!(add(&hits, &hit(2, 5), &[1; 16], &mut del), Add::Replace(1));
        let mut h = hit(3, 0);
        h.session_id = [9; 16];
        assert_eq!(add(&hits, &h, &[9; 16], &mut del), Add::Own);
        let mut hits2 = hits.clone();
        hits2[0].session_id = [9; 16];
        assert_eq!(add(&hits2, &h, &[1; 16], &mut del), Add::New);
        assert_eq!(del, vec![true, false]);
    }

    #[test]
    fn host_atoms() {
        let mut h = hit(1, 0);
        h.uphost = v4(1, 2, 3, 4, 80);
        h.version_ex_number = 3;
        let mut b = AtomBuf::default();
        write_atoms(&mut b, &h, &[1; 16]);
        assert_eq!(&b.0[..8], &[b'h', b'o', b's', b't', 19, 0, 0, 0x80]);
    }
}
