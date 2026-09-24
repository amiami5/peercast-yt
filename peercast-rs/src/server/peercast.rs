//! サーバー全体 (core/common/peercast.cpp の `PeercastInstance` と、グローバル変数の `servMgr`、`chanMgr`、
//! `g_ypList`、`g_notificationBuffer`)。
//!
//! C++ 版のグローバル変数の代わりに、`Arc<Peercast>` を引数で渡す。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::app::App;
use super::chanmgr::ChanMgr;
use super::directory::YpList;
use super::notif::{Notification, NotificationBuffer};
use super::servmgr::ServMgr;
use super::sys;

pub struct Peercast {
    pub app: App,
    pub servmgr: ServMgr,
    pub chanmgr: ChanMgr,
    pub yplist: YpList,
    notifications: Mutex<NotificationBuffer>,
    quitting: AtomicBool,
}

impl Peercast {
    /// サーバーを作る (まだ始めない)
    pub fn new(app: App) -> Arc<Peercast> {
        let rtmp_path = sys::join_path(&[&sys::dirname(&sys::executable_path()), b"rtmp-server"]);
        Arc::new(Peercast {
            servmgr: ServMgr::new(&rtmp_path),
            chanmgr: ChanMgr::new(),
            yplist: YpList::default(),
            notifications: Mutex::new(NotificationBuffer::default()),
            quitting: AtomicBool::new(false),
            app,
        })
    }

    /// `PeercastInstance::init`: 設定を読んでサーバーを始める
    pub fn init(self: &Arc<Self>) {
        if !self.app.ini_filename.is_empty() {
            self.servmgr.load_settings(self, &self.app.ini_filename);
        }
        #[cfg(unix)]
        super::tls::configure_server(
            &[&self.app.settings_dir[..], b"/server.crt"].concat(),
            &[&self.app.settings_dir[..], b"/server.key"].concat(),
        );
        self.servmgr.load_token_list(self);
        self.servmgr.start(self);
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    /// `PeercastInstance::quit`
    pub fn quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        self.chanmgr.quit();
        self.servmgr.quit();
        // スレッドが後始末をする時間
        sys::sleep(1000);
    }

    /// `PeercastInstance::saveSettings`
    pub fn save_settings(&self) {
        self.servmgr.save_settings(self, &self.app.ini_filename);
    }

    /// `peercast::notifyMessage`
    pub fn notify_message(&self, ty: u32, message: &[u8]) {
        self.notifications().add(Notification { time: sys::get_time(), ty, message: message.to_vec() });
        self.app.notify_message(ty, message);
    }

    pub fn notifications(&self) -> MutexGuard<'_, NotificationBuffer> {
        self.notifications.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `PeercastInstance::setServerPort`
    pub fn set_server_port(&self, port: u16) {
        self.servmgr.settings().server_host.port = port;
        self.servmgr.restart_server.store(true, Ordering::SeqCst);
    }
}
