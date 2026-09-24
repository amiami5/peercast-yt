//! 通知 (core/common/notif.cpp の `Notification` と `NotificationBuffer`)。

use std::collections::VecDeque;

use super::state::{arr, obj, s, Value};

pub const NT_UPGRADE: u32 = 0x0001;
pub const NT_PEERCAST: u32 = 0x0002;
pub const NT_BROADCASTERS: u32 = 0x0004;
pub const NT_TRACKINFO: u32 = 0x0008;

/// `Notification`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub time: u32,
    pub ty: u32,
    pub message: Vec<u8>,
}

impl Default for Notification {
    fn default() -> Self {
        Notification { time: 0, ty: NT_PEERCAST, message: Vec::new() }
    }
}

/// `Notification::getTypeStr`
pub fn type_str(ty: u32) -> &'static str {
    match ty {
        NT_UPGRADE => "Upgrade Alert",
        NT_PEERCAST => "Peercast",
        NT_BROADCASTERS => "Broadcasters",
        NT_TRACKINFO => "Track Info",
        _ => "Unknown",
    }
}

/// `NotificationBuffer`: 新しい順に 20 個まで
#[derive(Debug, Default)]
pub struct NotificationBuffer {
    pub entries: VecDeque<(Notification, bool)>,
}

const MAX_NOTIFS: usize = 20;

impl NotificationBuffer {
    pub fn num_unread(&self) -> usize {
        self.entries.iter().filter(|e| !e.1).count()
    }

    /// `markAsRead`: `ctime` 以前のものを読んだことにする
    pub fn mark_as_read(&mut self, ctime: u32) {
        for e in self.entries.iter_mut() {
            if e.0.time <= ctime {
                e.1 = true;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `getNotification` (範囲外なら空の通知)
    pub fn get(&self, index: usize) -> (Notification, bool) {
        self.entries.get(index).cloned().unwrap_or_default()
    }

    /// `addNotification`
    pub fn add(&mut self, n: Notification) {
        while self.entries.len() >= MAX_NOTIFS {
            self.entries.pop_back();
        }
        self.entries.push_front((n, false));
    }

    /// `getState`
    pub fn state(&self) -> Value {
        let list = self
            .entries
            .iter()
            .map(|(n, read)| {
                obj(vec![
                    ("message", s(&n.message)),
                    ("isRead", s(if *read { "1" } else { "0" })),
                    ("type", s(type_str(n.ty))),
                    ("unixTime", s(n.time.to_string())),
                    ("time", s(crate::strutil::rstrip(&super::sys::time_string(n.time)))),
                ])
            })
            .collect();
        obj(vec![
            ("numNotifications", s(self.len().to_string())),
            ("numUnread", s(self.num_unread().to_string())),
            ("notifications", arr(list)),
        ])
    }
}
