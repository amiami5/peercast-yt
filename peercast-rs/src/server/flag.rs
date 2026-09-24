//! 実験的な機能の旗 (core/common/flag.cpp の `Flag` と `FlagRegistory`)。

use std::sync::atomic::{AtomicBool, Ordering};

use super::state::{b, obj, s, Value};

/// `Flag`
#[derive(Debug)]
pub struct Flag {
    pub name: &'static str,
    pub desc: &'static str,
    pub default_value: bool,
    current: AtomicBool,
}

impl Flag {
    pub fn get(&self) -> bool {
        self.current.load(Ordering::Relaxed)
    }

    pub fn set(&self, v: bool) {
        self.current.store(v, Ordering::Relaxed);
    }
}

/// `FlagRegistory`
#[derive(Debug)]
pub struct FlagRegistry {
    flags: Vec<Flag>,
}

impl FlagRegistry {
    pub fn new(defs: &[(&'static str, &'static str, bool)]) -> FlagRegistry {
        FlagRegistry {
            flags: defs.iter().map(|&(name, desc, d)| Flag { name, desc, default_value: d, current: AtomicBool::new(d) }).collect(),
        }
    }

    /// `ServMgr` のコンストラクターの旗
    pub fn servmgr_default() -> FlagRegistry {
        FlagRegistry::new(&[
            ("randomizeBroadcastingChannelID", "配信するチャンネルのIDをランダムにする。", true),
            ("sendPortAtomWhenFirewallUnknown", "古いPeerCastStation相手に正常にポートチェックするにはオフにする。", false),
            ("forceFirewalled", "ファイアーウォール オンであるかの様に振る舞う。", false),
            ("startPlayingFromKeyFrame", "DIRECT接続でキーフレームまで継続パケットをスキップする。", true),
            ("banTrackersWhileBroadcasting", "配信中他の配信者による視聴をBANする。", false),
            ("persistTokenList", "アクセストークンリストを永続化する。", false),
            ("enableSSLServer", "SSL接続の受け付けを有効にする。", false),
            ("requireContinuationPacketSupportFromPeer", "継続パケットをサポートしないバージョンのクライアントとリレーしない。", false),
        ])
    }

    /// `get` (名前がなければ `None`。C++ 版は `std::out_of_range`)
    pub fn find(&self, name: &[u8]) -> Option<&Flag> {
        self.flags.iter().find(|f| f.name.as_bytes() == name)
    }

    /// 決まった名前の旗の値
    pub fn get(&self, name: &str) -> bool {
        self.find(name.as_bytes()).map_or(false, |f| f.get())
    }

    /// `forEachFlag`: 名前の順
    pub fn sorted(&self) -> Vec<&Flag> {
        let mut v: Vec<&Flag> = self.flags.iter().collect();
        v.sort_by(|a, b| a.name.cmp(b.name));
        v
    }

    /// `getState`
    pub fn state(&self) -> Value {
        obj(self
            .flags
            .iter()
            .map(|f| {
                (
                    f.name,
                    obj(vec![
                        ("name", s(f.name)),
                        ("desc", s(f.desc)),
                        ("defaultValue", b(f.default_value)),
                        ("currentValue", b(f.get())),
                    ]),
                )
            })
            .collect())
    }
}
