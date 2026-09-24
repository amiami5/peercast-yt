//! 統計 (core/common/stats.cpp の `Stats`)。C++ 版と同じくプロセスに 1 つ。

use std::sync::{Mutex, OnceLock};

use super::sys;

/// `Stats::STAT`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Stat {
    None = 0,
    PacketsStart,
    NumQueryIn,
    NumQueryOut,
    NumPingIn,
    NumPingOut,
    NumPongIn,
    NumPongOut,
    NumPushIn,
    NumPushOut,
    NumHitIn,
    NumHitOut,
    NumOtherIn,
    NumOtherOut,
    NumDropped,
    NumDup,
    NumAccepted,
    NumOld,
    NumBad,
    NumHops1,
    NumHops2,
    NumHops3,
    NumHops4,
    NumHops5,
    NumHops6,
    NumHops7,
    NumHops8,
    NumHops9,
    NumHops10,
    NumPacketsIn,
    NumPacketsOut,
    NumRouted,
    NumBroadcasted,
    NumDiscarded,
    NumDead,
    PacketDataIn,
    PacketDataOut,
    PacketSend,
    BytesIn,
    BytesOut,
    LocalBytesIn,
    LocalBytesOut,
    Max,
}

const MAX: usize = Stat::Max as usize;

struct Inner {
    current: [u32; MAX],
    last: [u32; MAX],
    per_sec: [u32; MAX],
    last_update: u32,
}

fn stats() -> &'static Mutex<Inner> {
    static S: OnceLock<Mutex<Inner>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Inner { current: [0; MAX], last: [0; MAX], per_sec: [0; MAX], last_update: 0 }))
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    stats().lock().unwrap_or_else(|e| e.into_inner())
}

/// `Stats::add`
pub fn add(s: Stat, n: u32) {
    let mut g = lock();
    g.current[s as usize] = g.current[s as usize].wrapping_add(n);
}

/// `Stats::update`: 5 秒ごとに 1 秒あたりの量を計算する
pub fn update() {
    let ctime = sys::get_time();
    let mut g = lock();
    let diff = ctime.wrapping_sub(g.last_update);
    if diff >= 5 {
        for i in 0..MAX {
            g.per_sec[i] = g.current[i].wrapping_sub(g.last[i]) / diff;
            g.last[i] = g.current[i];
        }
        g.last_update = ctime;
    }
}

/// `Stats::clearRange`
pub fn clear_range(s: Stat, e: Stat) {
    let mut g = lock();
    for i in s as usize..=e as usize {
        g.current[i] = 0;
    }
}

pub fn per_second(s: Stat) -> u32 {
    lock().per_sec[s as usize]
}

pub fn current(s: Stat) -> u32 {
    lock().current[s as usize]
}

/// `BYTES_TO_KBPS`: `float` で計算する
pub fn bytes_to_kbps(n: u32) -> f32 {
    ((n as f32) * 8.0f32) / 1024.0f32
}

/// `Stats::getState` の値 (名前と文字列)
pub fn state() -> Vec<(&'static str, String)> {
    let ps = per_second;
    let kb = |n: u32| format!("{:.1}", bytes_to_kbps(n) as f64);
    vec![
        ("totalInPerSec", kb(ps(Stat::BytesIn))),
        ("totalOutPerSec", kb(ps(Stat::BytesOut))),
        ("totalPerSec", kb(ps(Stat::BytesIn).wrapping_add(ps(Stat::BytesOut)))),
        ("wanInPerSec", kb(ps(Stat::BytesIn).wrapping_sub(ps(Stat::LocalBytesIn)))),
        ("wanOutPerSec", kb(ps(Stat::BytesOut).wrapping_sub(ps(Stat::LocalBytesOut)))),
        (
            "wanTotalPerSec",
            kb(ps(Stat::BytesIn)
                .wrapping_sub(ps(Stat::LocalBytesIn))
                .wrapping_add(ps(Stat::BytesOut).wrapping_sub(ps(Stat::LocalBytesOut)))),
        ),
        ("netInPerSec", kb(ps(Stat::PacketDataIn))),
        ("netOutPerSec", kb(ps(Stat::PacketDataOut))),
        ("netTotalPerSec", kb(ps(Stat::PacketDataOut).wrapping_add(ps(Stat::PacketDataIn)))),
        ("packInPerSec", format!("{}", ps(Stat::NumPacketsIn))),
        ("packOutPerSec", format!("{}", ps(Stat::NumPacketsOut))),
        ("packTotalPerSec", format!("{}", ps(Stat::NumPacketsOut).wrapping_add(ps(Stat::NumPacketsIn)))),
    ]
}
