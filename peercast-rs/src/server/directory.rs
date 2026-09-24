//! イエローページとその周り: YP の一覧 (core/common/yplist.cpp の `YPList`)、チャンネルの一覧の取得
//! (chandir.cpp の `ChannelDirectory`)、帯域測定 (uptest.cpp の `UptestServiceRegistry`)、RTMP サーバーの
//! 監視 (rtmpmonit.cpp の `RTMPServerMonitor`)、IPv6 のポートの確認 (portcheck.cpp の `IPv6PortChecker`)。
//! 解析と判断は段階 7 の `crate::chandir` と `crate::uptest`。

use std::sync::Mutex;

use super::error::{Error, Result};
use super::host::{Host, Ip};
use super::http::{Headers, Http, Request, PCX_AGENT};
use super::socket::ClientSocket;
use super::state::{arr, n, obj, s, Value};
use super::subprog::{Environment, Subprogram};
use super::sys;
use crate::chandir::Entry;

// ---------------------------------------------------------------- YP の一覧

/// `YP`
#[derive(Clone, Debug)]
pub struct Yp {
    pub name: &'static str,
    pub feed_url: &'static str,
    pub root_host: &'static str,
}

/// `YPList`
pub struct YpList {
    pub list: Vec<Yp>,
}

impl Default for YpList {
    fn default() -> Self {
        YpList {
            list: vec![
                Yp { name: "SP", feed_url: "http://bayonet.ddo.jp/sp/index.txt", root_host: "bayonet.ddo.jp:7146" },
                Yp { name: "Heisei", feed_url: "http://yp.pcgw.pgw.jp/index.txt", root_host: "yp.pcgw.pgw.jp:7146" },
                Yp { name: "P@", feed_url: "https://p-at.net/index.txt", root_host: "root.p-at.net" },
                Yp { name: "YPv6", feed_url: "http://ypv6.pecastation.org/index.txt", root_host: "ypv6.pecastation.org" },
                Yp { name: "Event YP", feed_url: "http://eventyp.xrea.jp/index.txt", root_host: "" },
            ],
        }
    }
}

impl YpList {
    /// `feedUrlToRootHost`
    pub fn root_host_of(&self, feed_url: &[u8]) -> Vec<u8> {
        match self.list.iter().find(|y| y.feed_url.as_bytes() == feed_url) {
            Some(y) => y.root_host.as_bytes().to_vec(),
            None => {
                crate::log_warn!("feedUrlToRootHost: Entry not found for '{}'", String::from_utf8_lossy(feed_url));
                Vec::new()
            }
        }
    }

    /// `getState`
    pub fn state(&self) -> Value {
        obj(vec![(
            "list",
            arr(self
                .list
                .iter()
                .map(|y| obj(vec![("name", s(y.name)), ("feedUrl", s(y.feed_url)), ("rootHost", s(y.root_host))]))
                .collect()),
        )])
    }
}

// ---------------------------------------------------------------- チャンネルの一覧

/// `ChannelEntry` (`feedUrl` 付き)
#[derive(Clone, Debug, Default)]
pub struct ChannelEntry {
    pub e: Entry,
    pub feed_url: Vec<u8>,
}

impl std::ops::Deref for ChannelEntry {
    type Target = Entry;
    fn deref(&self) -> &Entry {
        &self.e
    }
}

impl ChannelEntry {
    pub fn chat_url(&self) -> Vec<u8> {
        crate::chandir::chat_url(&self.feed_url, &self.e.encoded_name)
    }

    pub fn stats_url(&self) -> Vec<u8> {
        crate::chandir::stats_url(&self.feed_url, &self.e.encoded_name)
    }

    /// `ChannelEntry::textToChannelEntries`
    pub fn text_to_entries(text: &[u8], feed_url: &[u8], errors: &mut Vec<Vec<u8>>) -> Vec<ChannelEntry> {
        let mut res = Vec::new();
        crate::chandir::parse_index(text, |line| match line {
            crate::chandir::Line::Entry(e) => res.push(ChannelEntry { e, feed_url: feed_url.to_vec() }),
            crate::chandir::Line::Error(lineno) => errors.push(crate::chandir::parse_error_message(lineno)),
        });
        res
    }
}

/// `ChannelFeed::Status`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedStatus {
    Unknown,
    Ok,
    Error,
}

/// `ChannelFeed`
#[derive(Clone, Debug)]
pub struct ChannelFeed {
    pub url: Vec<u8>,
    pub status: FeedStatus,
    pub log: Vec<u8>,
}

impl FeedStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FeedStatus::Unknown => "UNKNOWN",
            FeedStatus::Ok => "OK",
            FeedStatus::Error => "ERROR",
        }
    }
}

#[derive(Default)]
struct DirInner {
    channels: Vec<ChannelEntry>,
    feeds: Vec<ChannelFeed>,
    last_update: u32,
}

/// `ChannelDirectory`
#[derive(Default)]
pub struct ChannelDirectory {
    inner: Mutex<DirInner>,
    /// `update` を同時に 2 つ走らせない (C++ 版は一覧のロックを持ったまま取りに行っていた)
    updating: Mutex<()>,
}

/// `ChannelDirectory::UpdateMode`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateMode {
    Auto,
    Manual,
}

impl ChannelDirectory {
    fn lock(&self) -> std::sync::MutexGuard<'_, DirInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn num_channels(&self) -> i32 {
        self.lock().channels.len() as i32
    }

    pub fn num_feeds(&self) -> i32 {
        self.lock().feeds.len() as i32
    }

    pub fn feeds(&self) -> Vec<ChannelFeed> {
        self.lock().feeds.clone()
    }

    pub fn channels(&self) -> Vec<ChannelEntry> {
        self.lock().channels.clone()
    }

    /// `addFeed`: http(s) の URL で、まだないものだけ
    pub fn add_feed(&self, url: &[u8]) -> bool {
        let mut g = self.lock();
        if g.feeds.iter().any(|f| f.url == url) {
            crate::log_error!("Already have feed {}", String::from_utf8_lossy(url));
            return false;
        }
        let ok = crate::url::parse_url(url).map_or(false, |u| u.scheme == b"http" || u.scheme == b"https");
        if !ok {
            crate::log_error!("Invalid feed URL {}", String::from_utf8_lossy(url));
            return false;
        }
        g.feeds.push(ChannelFeed { url: url.to_vec(), status: FeedStatus::Unknown, log: Vec::new() });
        true
    }

    /// `clearFeeds`
    pub fn clear_feeds(&self) {
        let mut g = self.lock();
        g.feeds.clear();
        g.channels.clear();
        g.last_update = 0;
    }

    pub fn total_listeners(&self) -> i32 {
        self.lock().channels.iter().fold(0i32, |a, e| a.wrapping_add(e.num_directs.max(0)))
    }

    pub fn total_relays(&self) -> i32 {
        self.lock().channels.iter().fold(0i32, |a, e| a.wrapping_add(e.num_relays.max(0)))
    }

    /// `findEntry`
    pub fn find_entry(&self, id: &[u8; 16]) -> Option<ChannelEntry> {
        self.lock().channels.iter().find(|e| e.id == *id).cloned()
    }

    /// `findTracker`
    pub fn find_tracker(&self, id: &[u8; 16]) -> Vec<u8> {
        self.find_entry(id).map(|e| e.tip.clone()).unwrap_or_default()
    }

    /// `update`: 一覧を取り直す (自動なら 5 分、手動なら 30 秒おき)。取りに行ったら true
    pub fn update(&self, server_port: u16, mode: UpdateMode) -> bool {
        let _u = match self.updating.try_lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let cool_down = if mode == UpdateMode::Manual { 30 } else { 5 * 60 };
        let feeds = {
            let g = self.lock();
            if sys::get_time().wrapping_sub(g.last_update) < cool_down {
                return false;
            }
            g.feeds.clone()
        };
        let t0 = sys::get_dtime();
        let handles: Vec<_> = feeds
            .into_iter()
            .map(|mut feed| {
                std::thread::spawn(move || {
                    let mut channels = Vec::new();
                    let start = sys::get_time();
                    let (ok, lines) = super::log::capture(|| {
                        let t1 = sys::get_dtime();
                        let ok = get_feed(&feed.url, server_port, &mut channels);
                        let t2 = sys::get_dtime();
                        crate::log_trace!("Got {} channels from {} ({:.6} seconds)", channels.len(), String::from_utf8_lossy(&feed.url), t2 - t1);
                        ok
                    });
                    feed.status = if ok { FeedStatus::Ok } else { FeedStatus::Error };
                    // runProcess の出力: 始めた時刻と、ログ (エラーと警告には印)
                    let mut log = Vec::new();
                    log.extend_from_slice(b"Start time: ");
                    log.extend(sys::time_string(start));
                    log.push(b'\n');
                    for (ty, msg) in lines {
                        match ty {
                            super::log::Level::Error => log.extend_from_slice(b"Error: "),
                            super::log::Level::Warn => log.extend_from_slice(b"Warning: "),
                            _ => {}
                        }
                        log.extend_from_slice(&msg);
                        log.extend_from_slice(b"\r\n");
                    }
                    feed.log = log;
                    (feed, channels)
                })
            })
            .collect();
        let mut new_feeds = Vec::new();
        let mut all = Vec::new();
        for h in handles {
            if let Ok((f, chs)) = h.join() {
                new_feeds.push(f);
                all.extend(chs);
            }
        }
        // 聴取者の多い順 (同じなら元の順)
        all.sort_by(|a, b| b.num_directs.cmp(&a.num_directs));
        let n = all.len();
        {
            let mut g = self.lock();
            g.channels = all;
            for f in new_feeds {
                if let Some(x) = g.feeds.iter_mut().find(|x| x.url == f.url) {
                    x.status = f.status;
                    x.log = f.log;
                }
            }
            g.last_update = sys::get_time();
        }
        crate::log_info!("Channel feed update: total of {} channels in {:.6} sec", n, sys::get_dtime() - t0);
        true
    }

    /// `getState`
    pub fn state(&self) -> Value {
        let (channels, feeds, last) = {
            let g = self.lock();
            (g.channels.clone(), g.feeds.clone(), g.last_update)
        };
        let chs: Vec<Value> = channels
            .iter()
            .map(|c| {
                obj(vec![
                    ("name", s(&c.name)),
                    ("id", s(super::chaninfo::id_str(&c.id))),
                    ("tip", s(&c.tip)),
                    ("url", s(&c.url)),
                    ("genre", s(&c.genre)),
                    ("desc", s(&c.desc)),
                    ("numDirects", n(c.num_directs)),
                    ("numRelays", n(c.num_relays)),
                    ("bitrate", n(c.bitrate)),
                    ("contentTypeStr", s(&c.content_type)),
                    ("trackArtist", s(&c.track_artist)),
                    ("trackAlbum", s(&c.track_album)),
                    ("trackName", s(&c.track_name)),
                    ("trackContact", s(&c.track_contact)),
                    ("encodedName", s(&c.encoded_name)),
                    ("uptime", s(&c.uptime)),
                    ("status", s(&c.status)),
                    ("comment", s(&c.comment)),
                    ("direct", n(c.direct)),
                    ("feedUrl", s(&c.feed_url)),
                    ("chatUrl", s(c.chat_url())),
                    ("statsUrl", s(c.stats_url())),
                ])
            })
            .collect();
        let fs: Vec<Value> = feeds
            .iter()
            .map(|f| {
                let count = channels.iter().filter(|c| c.feed_url == f.url).count();
                obj(vec![
                    ("url", s(&f.url)),
                    ("directoryUrl", s(crate::chandir::directory_url(&f.url))),
                    ("status", s(f.status.as_str())),
                    ("numChannels", n(count as f64)),
                ])
            })
            .collect();
        obj(vec![
            ("totalListeners", n(self.total_listeners())),
            ("totalRelays", n(self.total_relays())),
            ("lastUpdate", s(crate::chandir::format_time(sys::get_time().wrapping_sub(last)))),
            ("channels", arr(chs)),
            ("feeds", arr(fs)),
        ])
    }
}

/// `getFeed`: index.txt を取って読む。エラーなく読めたら true
fn get_feed(url: &[u8], server_port: u16, out: &mut Vec<ChannelEntry>) -> bool {
    out.clear();
    // cgi::Query に host=localhost:port を入れて str() したもの
    let q = [&b"host="[..], &crate::cgi::escape(format!("localhost:{}", server_port).as_bytes())].concat();
    let full = [url, b"?", &q].concat();
    match super::http::get(&full) {
        Ok(body) => {
            let mut errors = Vec::new();
            *out = ChannelEntry::text_to_entries(&body, url, &mut errors);
            for m in &errors {
                crate::log_error!("{}", String::from_utf8_lossy(m));
            }
            errors.is_empty()
        }
        Err(e) => {
            crate::log_error!("{}", e);
            false
        }
    }
}

// ---------------------------------------------------------------- 帯域測定

/// `UptestEndpoint`
#[derive(Clone, Debug)]
pub struct UptestEndpoint {
    pub url: Vec<u8>,
    pub status: i32,
    pub info: crate::uptest::Info,
    pub last_tried_at: u32,
    pub xml: Vec<u8>,
}

impl UptestEndpoint {
    fn new(url: &[u8]) -> UptestEndpoint {
        UptestEndpoint { url: url.to_vec(), status: crate::uptest::UNTRIED, info: Default::default(), last_tried_at: 0, xml: Vec::new() }
    }

    fn field(&self, name: &str) -> &[u8] {
        let i = crate::uptest::FIELDS.iter().position(|f| *f == name).unwrap_or(0);
        &self.info[i]
    }

    /// `update`: yp4g.xml を取り直す
    fn update(&mut self) {
        self.last_tried_at = sys::get_time();
        crate::log_debug!("Speedtest: {}", String::from_utf8_lossy(&self.url));
        match download(&self.url).and_then(|x| read_info(&x).map(|i| (x, i))) {
            Ok((x, i)) => {
                self.xml = x;
                self.info = i;
                self.status = crate::uptest::SUCCESS;
                crate::log_info!("Speedtest result: {} kbps", String::from_utf8_lossy(self.field("speed")));
            }
            Err(e) => {
                crate::log_error!("UptestEndpoint {}: {}", String::from_utf8_lossy(&self.url), e);
                self.status = crate::uptest::ERROR;
            }
        }
    }

    /// `takeSpeedtest`: 乱数のデータを送って測ってもらう
    fn take_speedtest(&mut self) -> std::result::Result<(), Vec<u8>> {
        match download(&self.url).and_then(|x| read_info(&x).map(|i| (x, i))) {
            Ok((x, i)) => {
                self.xml = x;
                self.info = i;
                crate::log_debug!(
                    "Speedtest {}: checkable = {}",
                    String::from_utf8_lossy(&self.url),
                    String::from_utf8_lossy(self.field("checkable"))
                );
            }
            Err(e) => {
                crate::log_error!("Speedtest {}: {}", String::from_utf8_lossy(&self.url), e);
                return Err(e.msg.into_bytes());
            }
        }
        if self.field("checkable") != b"1" {
            crate::log_error!("speedtest server unavailable: checkable != 1");
            return Err(b"speedtest server unavailable".to_vec());
        }
        let post = crate::uptest::post_url(self.field("addr"), self.field("port"), self.field("object"));
        let size = (crate::http::atoi(self.field("post_size")) as i64 * 1000).max(0) as usize;
        crate::log_debug!("Posting {} bytes of random data to {} ...", size, String::from_utf8_lossy(&post));
        match post_random_data(&post, size) {
            Ok(code) => {
                crate::log_trace!("... done");
                if code == 302 {
                    Ok(())
                } else {
                    Err(format!("unexpected status code {}", code).into_bytes())
                }
            }
            Err(e) => {
                crate::log_error!("exception occurred while posting: {}", e);
                Err(b"exception occurred while posting".to_vec())
            }
        }
    }
}

fn read_info(x: &[u8]) -> Result<crate::uptest::Info> {
    crate::uptest::read_info(x).map_err(|e| match e {
        crate::uptest::ReadError::Xml(crate::xml::Error::TagTooLong) => Error::stream("Tag too long"),
        crate::uptest::ReadError::Xml(crate::xml::Error::ContentTooBig) => Error::stream("Content too big"),
        crate::uptest::ReadError::Xml(crate::xml::Error::NotXml) => Error::stream("Not XML document"),
        crate::uptest::ReadError::Xml(crate::xml::Error::UnexpectedEndTag) => Error::stream("Unexpected end tag"),
        crate::uptest::ReadError::Attr(crate::xml::AttrError::TooMany) => Error::stream("Too many attributes"),
        crate::uptest::ReadError::Attr(crate::xml::AttrError::BadValue) => Error::stream("Bad tag value"),
        _ => Error::general("non-null assertion failed"),
    })
}

/// `UptestEndpoint::download`
fn download(url: &[u8]) -> Result<Vec<u8>> {
    let u = crate::url::parse_url(url).map_err(|_| Error::general("invalid URI"))?;
    if u.scheme != b"http" {
        return Err(Error::general("unsupported protocol"));
    }
    let host = Host::from_str_name(&u.host, super::http::uri_port(&u));
    if !host.ip.is_set() {
        return Err(Error::general("could not resolve host name"));
    }
    let mut sock = ClientSocket::new();
    sock.connect(host)?;
    let path = [&b"/"[..], &u.path].concat();
    let req = Request::new(b"GET", &path, b"HTTP/1.0", Headers::from(&[("Host", &u.host), ("Connection", b"close"), ("User-Agent", PCX_AGENT.as_bytes())]));
    let res = Http::new(&mut sock).send_request(&req)?;
    if res.status_code != 200 {
        return Err(Error::general(format!("unexpected status code {}", res.status_code)));
    }
    Ok(res.body)
}

/// `UptestEndpoint::postRandomData`: 状態の番号を返す
fn post_random_data(url: &[u8], size: usize) -> Result<i32> {
    let u = crate::url::parse_url(url).map_err(|_| Error::general("invalid URI"))?;
    let host = Host::from_str_name(&u.host, super::http::uri_port(&u));
    if !host.ip.is_set() {
        return Err(Error::general("Could not resolve host name"));
    }
    let mut sock = ClientSocket::new();
    sock.connect(host)?;
    let path = [&b"/"[..], &u.path].concat();
    let mut req = Request::new(
        b"POST",
        &path,
        b"HTTP/1.0",
        Headers::from(&[
            ("Host", &u.host),
            ("Connection", b"close"),
            ("User-Agent", PCX_AGENT.as_bytes()),
            ("Content-Length", size.to_string().as_bytes()),
            ("Content-Type", b"application/octet-stream"),
        ]),
    );
    let mut r = sys::Random::default();
    req.body = (0..size).map(|_| r.next() as u8).collect();
    Ok(Http::new(&mut sock).send_request(&req)?.status_code)
}

/// `UptestServiceRegistry`
#[derive(Default)]
pub struct UptestServiceRegistry {
    providers: Mutex<Vec<UptestEndpoint>>,
}

impl UptestServiceRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<UptestEndpoint>> {
        self.providers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `addURL`
    pub fn add_url(&self, url: &[u8]) -> std::result::Result<(), Vec<u8>> {
        let mut p = self.lock();
        let parsed = crate::url::parse_url(url);
        let scheme = parsed.as_ref().map(|u| u.scheme.clone()).unwrap_or_default();
        let existing: Vec<&[u8]> = p.iter().map(|e| e.url.as_slice()).collect();
        crate::uptest::check_add_url(parsed.is_ok(), &scheme, url, &existing).map_err(|e| e.to_vec())?;
        p.push(UptestEndpoint::new(url));
        Ok(())
    }

    pub fn urls(&self) -> Vec<Vec<u8>> {
        self.lock().iter().map(|e| e.url.clone()).collect()
    }

    /// `deleteByIndex`
    pub fn delete_by_index(&self, index: i32) -> std::result::Result<(), Vec<u8>> {
        let mut p = self.lock();
        if index < 0 || index as usize >= p.len() {
            return Err(b"index out of range".to_vec());
        }
        p.remove(index as usize);
        Ok(())
    }

    /// `takeSpeedtest`
    pub fn take_speedtest(&self, index: i32) -> std::result::Result<(), Vec<u8>> {
        let mut p = self.lock();
        if index < 0 || index as usize >= p.len() {
            return Err(b"index out of range".to_vec());
        }
        p[index as usize].take_speedtest()
    }

    /// `getXML`
    pub fn xml(&self, index: i32) -> std::result::Result<Vec<u8>, Vec<u8>> {
        let p = self.lock();
        if index < 0 || index as usize >= p.len() {
            return Err(b"index out of range".to_vec());
        }
        let e = &p[index as usize];
        if e.status != crate::uptest::SUCCESS {
            return Err(b"no XML".to_vec());
        }
        Ok(e.xml.clone())
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// `update`: 成功していないものを、間を空けて取り直す
    pub fn update(&self) {
        let mut p = self.lock();
        for e in p.iter_mut() {
            if e.status != crate::uptest::SUCCESS {
                if crate::uptest::is_ready(e.status, e.last_tried_at, sys::get_time()) {
                    e.update();
                } else {
                    crate::log_trace!("{} not ready to download", String::from_utf8_lossy(&e.url));
                }
            }
        }
    }

    /// `forceUpdate`
    pub fn force_update(&self) {
        for e in self.lock().iter_mut() {
            e.update();
        }
    }

    /// `getState`
    pub fn state(&self) -> Value {
        let p = self.lock();
        obj(vec![(
            "providers",
            arr(p
                .iter()
                .map(|e| {
                    obj(vec![
                        ("url", s(&e.url)),
                        ("status", s(crate::uptest::text_status(e.status).unwrap_or(b""))),
                        ("speed", s(e.field("speed"))),
                        ("over", s(e.field("over"))),
                        ("checkable", s(e.field("checkable"))),
                    ])
                })
                .collect()),
        )])
    }
}

// ---------------------------------------------------------------- RTMP サーバーの監視

struct RtmpInner {
    server: Subprogram,
    ip_version: i32,
    enabled: bool,
}

/// `RTMPServerMonitor`
pub struct RtmpServerMonitor {
    inner: Mutex<RtmpInner>,
}

impl RtmpServerMonitor {
    pub fn new(path: &[u8]) -> RtmpServerMonitor {
        RtmpServerMonitor { inner: Mutex::new(RtmpInner { server: Subprogram::new(path, false, false), ip_version: 4, enabled: false }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RtmpInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().enabled
    }

    pub fn status(&self) -> &'static str {
        if self.is_enabled() {
            "UP"
        } else {
            "DOWN"
        }
    }

    pub fn set_ip_version(&self, v: i32) {
        self.lock().ip_version = v;
    }

    /// `update`: 止まっていたら起動し直す。`args` は起動の引数 (`-p port url`)
    pub fn update(&self, args: impl FnOnce(i32) -> Vec<Vec<u8>>) {
        let mut g = self.lock();
        if !g.enabled {
            return;
        }
        if !g.server.is_alive() {
            crate::log_error!("RTMP server is down! Restarting... ");
            let a = args(g.ip_version);
            let env = Environment::from_current_process();
            g.server.start(&a, &env);
        }
    }

    pub fn enable(&self) {
        self.lock().enabled = true;
    }

    pub fn disable(&self) {
        let mut g = self.lock();
        g.enabled = false;
        if g.server.is_alive() {
            g.server.terminate();
        }
    }

    /// `getState`
    pub fn state(&self) -> Value {
        let mut g = self.lock();
        let status = if g.enabled { "UP" } else { "DOWN" };
        let pid = g.server.pid();
        let _ = &mut g;
        obj(vec![("status", s(status)), ("processID", s(pid.to_string())), ("ipVersion", s(g.ip_version.to_string()))])
    }
}

// ---------------------------------------------------------------- IPv6 のポートの確認

/// `IPv6PortChecker::run` の結果
pub struct PortCheckResult {
    pub ip: Ip,
    pub ports: Vec<i32>,
}

/// `IPv6PortChecker::run`: v6.api.pecastation.org に `port` を確かめてもらう
pub fn ipv6_port_check(session_id: &[u8; 16], port: u16) -> Result<PortCheckResult> {
    let host_name = b"v6.api.pecastation.org";
    let ip = super::host::resolve_all(host_name).into_iter().find(|ip| !ip.is_ipv4_mapped());
    let ip = match ip {
        Some(ip) if ip.is_set() => ip,
        _ => return Err(Error::general("No IPv6 address associated with v6.api.pecastation.org")),
    };
    crate::log_debug!("Contacting v6.api.pecastation.org ({}) ...", ip.str());
    let mut sock = ClientSocket::new();
    sock.connect(Host::new(ip, 80))?;
    let body = format!("{{\"instanceId\":\"{}\",\"ports\":[{}]}}", super::chaninfo::id_str(session_id), port as i32);
    let mut req = Request::new(
        b"POST",
        b"/portcheck",
        b"HTTP/1.1",
        Headers::from(&[
            ("Host", b"v6.api.pecastation.org"),
            ("Content-Type", b"application/json"),
            ("User-Agent", PCX_AGENT.as_bytes()),
            ("Content-Length", body.len().to_string().as_bytes()),
        ]),
    );
    req.body = body.clone().into_bytes();
    crate::log_debug!("Request: {}", body);
    let res = Http::new(&mut sock).send_request(&req)?;
    if res.status_code != 200 {
        return Err(Error::general(format!("HTTP request failed: {}", res.status_code)).with_err(res.status_code));
    }
    crate::log_debug!("Response: {}", String::from_utf8_lossy(&res.body));
    let data = crate::json::parse(&res.body).map_err(|e| Error::general(String::from_utf8_lossy(&e.what()).into_owned()))?;
    let ipstr = match data.get(b"ip") {
        Some(crate::json::Value::Str(s)) => s.clone(),
        _ => return Err(Error::general("[json.exception.type_error.302] type must be string, but is null")),
    };
    let ip = Ip::parse(&ipstr).ok_or_else(|| Error::format(format!("Invalid IP address: {}", String::from_utf8_lossy(&ipstr))))?;
    let mut ports = Vec::new();
    if let Some(crate::json::Value::Array(a)) = data.get(b"ports") {
        for v in a {
            if let Ok(p) = v.as_int() {
                ports.push(p);
            }
        }
    }
    Ok(PortCheckResult { ip, ports })
}
