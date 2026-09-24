//! サーバーを実際に起動して試すテスト (もとは Ruby の bvt/ と、Python の tests/server/ の
//! relay_test.py、source_test.py)。
//!
//! テストごとに一時ディレクトリに UI (html/) と設定ファイルを置き、別々のポートで `peercast` を起こす。
//! 設定は `tests/peercast.ini` (YP は空にしてあり、外のホストにはつながない)。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use peercast_rs::pcp::atom::id4;
use peercast_rs::pcp::write::AtomBuf;

/// UI (html/) は全部のテストで同じものを使う
fn ui_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("peercast-bvt-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
        peercast_ui_gen::run(&ui, &dir).expect("UI の生成");
        dir
    })
}

/// 起動した peercast。落とすときにディレクトリも消す
struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Server {
    fn start(port: u16) -> Server {
        Server::start_with(port, |ini| ini)
    }

    /// 設定を `edit` で変えて起こす
    fn start_with(port: u16, edit: fn(String) -> String) -> Server {
        let dir = std::env::temp_dir().join(format!("peercast-bvt-{}-{}", std::process::id(), port));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ini = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/peercast.ini")).unwrap();
        let ini = edit(ini.replace("serverPort = 7144", &format!("serverPort = {}", port)));
        std::fs::write(dir.join("peercast.ini"), ini).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_peercast"))
            .arg("-i")
            .arg(dir.join("peercast.ini"))
            .arg("-P")
            .arg(ui_dir())
            .current_dir(&dir)
            .env("XDG_CONFIG_HOME", &dir)
            .env("XDG_STATE_HOME", &dir)
            .env("XDG_CACHE_HOME", &dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("peercast を起動できない");
        let mut s = Server { child, port, dir };
        // 待ち受けを始めるまで待つ
        let t0 = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(s.child.try_wait().unwrap().is_none(), "peercast died immediately after spawn");
            assert!(t0.elapsed() < Duration::from_secs(10), "peercast が待ち受けを始めない");
            std::thread::sleep(Duration::from_millis(50));
        }
        s
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Response {
    code: u32,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn request(port: u16, head: &str, body: &[u8]) -> Response {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").expect("応答のヘッダーの終わりがない");
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap();
    let code = status.split(' ').nth(1).and_then(|c| c.parse().ok()).expect("状態の行");
    let headers = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    Response { code, headers, body: buf[end + 4..].to_vec() }
}

fn get(port: u16, path: &str) -> Response {
    request(port, &format!("GET {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", path, port), b"")
}

fn post(port: u16, path: &str, body: &str) -> Response {
    request(
        port,
        &format!("POST {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\n\r\n", path, port, body.len()),
        body.as_bytes(),
    )
}

fn json(r: &Response) -> peercast_rs::json::Value {
    peercast_rs::json::parse(&r.body).unwrap_or_else(|_| panic!("JSON でない: {}", String::from_utf8_lossy(&r.body)))
}

fn str_at<'a>(v: &'a peercast_rs::json::Value, key: &str) -> &'a [u8] {
    v.get(key.as_bytes()).and_then(|x| x.as_string().ok()).unwrap_or(b"")
}

/// 00-smoke: 起動して、すぐには落ちない
#[test]
fn smoke() {
    let mut s = Server::start(17201);
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.child.try_wait().unwrap().is_none(), "process is dead");
}

/// 01-int-kills: SIGINT で 10 秒以内に正常に終わる
#[cfg(unix)]
#[test]
fn int_kills() {
    let mut s = Server::start(17202);
    let st = Command::new("kill").arg("-INT").arg(s.child.id().to_string()).status().unwrap();
    assert!(st.success());
    let t0 = Instant::now();
    loop {
        if let Some(st) = s.child.try_wait().unwrap() {
            assert!(st.code().is_some(), "process died abnormally: {:?}", st);
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "SIGINT で終わらない");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 02-html: 管理画面のページ
#[test]
fn html() {
    let s = Server::start(17203);
    let p = s.port;
    for path in ["/", "/html"] {
        let r = get(p, path);
        assert_eq!(r.code, 302, "{}", path);
        assert_eq!(r.header("Location"), Some("/html/en/index.html"), "{}", path);
    }
    assert_eq!(get(p, "/html/").code, 404);
    assert_eq!(get(p, "/html/en").code, 404);
    for path in ["/html/en/index.html", "/html/ja/index.html"] {
        let r = get(p, path);
        assert_eq!(r.code, 200, "{}", path);
        assert!(!r.body.is_empty(), "{}", path);
    }
    let r = get(p, "/html/index.html");
    assert_eq!(r.code, 302);
    assert_eq!(r.header("Location"), Some("/"));
    for file in [
        "notifications.html",
        "settings.html",
        "broadcast.html",
        "connections.html",
        "login.html",
        "viewlog.html",
        "channels.html",
        "logout.html",
    ] {
        assert_eq!(get(p, &format!("/html/ja/{}", file)).code, 200, "{}", file);
    }
    // id= がないので Bad Request
    assert_eq!(get(p, "/html/ja/relayinfo.html").code, 400);
    assert_eq!(get(p, "/html/ja/play.html").code, 400);
}

/// 設定ファイルのフィルターが、最後のものまで全部読まれる (設定の画面に出る)
#[test]
fn filters() {
    let s = Server::start_with(17210, |ini| {
        let filter = |ip: &str, private: &str, network: &str, direct: &str| {
            format!("[Filter]\r\nip = {}\r\nprivate = {}\r\nban = No\r\nnetwork = {}\r\ndirect = {}\r\n[End]\r\n", ip, private, network, direct)
        };
        let old = filter("255.255.255.255", "No", "Yes", "Yes");
        assert!(ini.contains(&old));
        let new = [
            filter("255.255.255.255", "No", "Yes", "Yes"),
            filter("::/0", "No", "No", "Yes"),
            filter("192.0.2.0/24", "Yes", "Yes", "Yes"),
        ]
        .concat();
        ini.replace(&old, &new)
    });
    let body = String::from_utf8(get(s.port, "/html/en/settings.html").body).unwrap();
    let ips: Vec<&str> = body
        .split("value=\"")
        .skip(1)
        .filter(|t| t.contains("name=\"filt_ip\""))
        .map(|t| &t[..t.find('"').unwrap()])
        .collect();
    // 最後は新しく足すための空の行
    assert_eq!(ips, ["255.255.255.255", "::/0", "192.0.2.0/24", "0.0.0.0"]);
    let checked = |name: &str| body.contains(&format!("checked value=\"1\" name=\"{}\"", name));
    assert!(checked("filt_nw0") && checked("filt_di0") && !checked("filt_pr0"));
    assert!(!checked("filt_nw1") && checked("filt_di1") && !checked("filt_pr1"));
    assert!(checked("filt_nw2") && checked("filt_di2") && checked("filt_pr2"));
}

/// 03-jrpc: JSON-RPC
#[test]
fn jrpc() {
    let s = Server::start(17204);
    let p = s.port;
    let r = get(p, "/api/1");
    assert_eq!(r.code, 200);
    let v = json(&r);
    assert!(str_at(&v, "agentName").starts_with(b"PeerCast/0.1218"));
    assert_eq!(str_at(&v, "apiVersion"), b"1.0.0");
    assert_eq!(str_at(&v, "jsonrpc"), b"2.0");

    let r = post(p, "/api/1", r#"{"jsonrpc": "2.0","method": "getStatus","id": 9999}"#);
    let v = json(&r);
    assert_eq!(v.get(b"id").and_then(|x| x.as_int().ok()), Some(9999));
    assert_eq!(str_at(&v, "jsonrpc"), b"2.0");
    let result = v.get(b"result").expect("result");
    for key in ["globalDirectEndPoint", "globalRelayEndPoint", "isFirewalled", "localDirectEndPoint", "localRelayEndPoint", "uptime"] {
        assert!(result.get(key.as_bytes()).is_some(), "{}", key);
    }
}

/// atom を 1 つ読む (親なら子も)。名前と、子か中身
enum Atom {
    Parent([u8; 4], Vec<Atom>),
    Data([u8; 4], Vec<u8>),
}

fn read_atom(s: &mut TcpStream) -> Atom {
    let mut h = [0u8; 8];
    s.read_exact(&mut h).unwrap();
    let id = [h[0], h[1], h[2], h[3]];
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    if len & 0x8000_0000 != 0 {
        let n = len & 0x7fff_ffff;
        assert!(n < 1000, "子が多すぎる");
        Atom::Parent(id, (0..n).map(|_| read_atom(s)).collect())
    } else {
        assert!(len < 1_000_000, "atom が大きすぎる");
        let mut d = vec![0u8; len as usize];
        s.read_exact(&mut d).unwrap();
        Atom::Data(id, d)
    }
}

fn child<'a>(children: &'a [Atom], name: &[u8]) -> Option<&'a [u8]> {
    children.iter().find_map(|a| match a {
        Atom::Data(id, d) if &id[..name.len()] == name && id[name.len()..].iter().all(|&c| c == 0) => Some(d.as_slice()),
        _ => None,
    })
}

/// 04-helo: PCP のハンドシェイク (helo を送って oleh を受け取る)
#[test]
fn helo() {
    let srv = Server::start(17205);
    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut out = AtomBuf::default();
    out.int(*b"pcp\n", 1);
    out.parent(id4(b"helo"), 4);
    out.string(id4(b"agnt"), b"PeerCast/0.1218 (YT50)");
    out.int(id4(b"ver"), 1218);
    out.bytes(id4(b"sid"), b"1234567890123456");
    out.short(id4(b"port"), 8144);
    s.write_all(&out.0).unwrap();

    let (id, children) = match read_atom(&mut s) {
        Atom::Parent(id, c) => (id, c),
        Atom::Data(id, _) => panic!("oleh expected, got {:?}", id),
    };
    assert_eq!(&id, b"oleh");
    let agent = child(&children, b"agnt").expect("agnt");
    let agent = std::str::from_utf8(agent).unwrap().trim_end_matches('\0');
    let rest = agent.strip_prefix("PeerCast/0.1218 (YT").expect("agnt");
    let (num, tail) = rest.split_at(2);
    assert!(num.bytes().all(|c| c.is_ascii_digit()) && tail.ends_with(')'), "agnt: {}", agent);
    assert!(child(&children, b"sid").is_some(), "sid");
    let ver = child(&children, b"ver").expect("ver");
    assert_eq!(u32::from_le_bytes([ver[0], ver[1], ver[2], ver[3]]), 1218);
    assert!(child(&children, b"rip").is_some(), "rip");
    assert!(child(&children, b"port").is_some(), "port");
}

/// /cgi-bin: 掲示板ビューワーと flv.cgi (もとは CGI スクリプト)。外のホストにはつながない
#[test]
fn cgi_bin() {
    let s = Server::start(17206);
    let p = s.port;
    let r = get(p, "/cgi-bin/board.cgi?category=x");
    assert_eq!(r.code, 400);
    assert_eq!(r.header("Content-Type"), Some("text/plain"));
    assert_eq!(r.body, b"bad parameter\n");
    assert_eq!(get(p, "/cgi-bin/thread.cgi?fqdn=a.example&category=..&id=1").body, b"bad category\n");
    assert_eq!(get(p, "/cgi-bin/post.cgi?fqdn=a.example&category=x&id=1").body, b"body\n");
    // 内部のアドレスには接続しない
    for fqdn in ["127.0.0.1:1", "localhost:1", "10.0.0.1"] {
        assert_eq!(get(p, &format!("/cgi-bin/board.cgi?fqdn={}&category=test", fqdn)).code, 500, "{}", fqdn);
    }
    // ほかのスクリプトはない
    for path in ["/cgi-bin/", "/cgi-bin/x.cgi", "/cgi-bin/bbs_reader.py", "/cgi-bin/cgiform.py", "/cgi-bin/../html/en/index.html", "/cgi-bin/flv.cgix"] {
        assert_eq!(get(p, path).code, 404, "{}", path);
    }
    assert_eq!(get(p, "/cgi-bin/flv.cgi?id=xyz&preset=p&audio_codec=a&type=t").code, 400);
    assert_eq!(get(p, "/cgi-bin/flv.cgi?id=00000000000000000000000000000000&preset=-i&audio_codec=a&type=t").code, 400);
}

fn jrpc_call(port: u16, method: &str, params: &str) -> peercast_rs::json::Value {
    let r = post(port, "/api/1", &format!(r#"{{"jsonrpc": "2.0", "id": 1, "method": "{}", "params": {}}}"#, method, params));
    let v = json(&r);
    v.get(b"result").unwrap_or_else(|| panic!("{}: {}", method, String::from_utf8_lossy(&r.body))).clone()
}

/// getChannels の (名前, ID) の並び。`n` 個になるまで待つ
fn wait_channels(port: u16, n: usize) -> Vec<(String, String)> {
    let t0 = Instant::now();
    loop {
        let chans = match jrpc_call(port, "getChannels", "[]") {
            peercast_rs::json::Value::Array(a) => a,
            v => panic!("getChannels: {}", v.type_name()),
        };
        let list: Vec<(String, String)> = chans
            .iter()
            .map(|c| {
                let name = c.get(b"info").map(|i| str_at(i, "name")).unwrap_or(b"");
                (String::from_utf8_lossy(name).into_owned(), String::from_utf8_lossy(str_at(c, "channelId")).into_owned())
            })
            .collect();
        if list.len() >= n {
            return list;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "チャンネルができない: {:?}", list);
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn flv_tag(ty: u8, ts: u32, data: &[u8]) -> Vec<u8> {
    let mut v = vec![ty];
    v.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(&ts.to_be_bytes()[1..]);
    v.push((ts >> 24) as u8);
    v.extend_from_slice(&[0, 0, 0]);
    v.extend_from_slice(data);
    v.extend_from_slice(&(11 + data.len() as u32).to_be_bytes());
    v
}

/// FLV のヘッダーと、1 秒ごとにキーフレームがある 30fps の映像のタグ (`seconds` 秒分)
fn flv_stream(seconds: usize) -> Vec<Vec<u8>> {
    let meta = flv_tag(18, 0, b"\x02\x00\x0aonMetaData\x08\x00\x00\x00\x00\x00\x00\x09");
    let mut v = vec![b"FLV\x01\x01\x00\x00\x00\x09\0\0\0\0".to_vec(), meta];
    for i in 0..seconds * 30 {
        let mut body = vec![if i % 30 == 0 { 0x12 } else { 0x22 }];
        body.extend(std::iter::repeat(i as u8).take(2000));
        v.push(flv_tag(9, i as u32 * 33, &body));
    }
    v
}

/// 30fps の速さで送る (`chunked` なら chunked 転送の形で)。`stop` が立つか、送れなくなったら終わる
fn paced_send(s: &mut dyn Write, chunks: Vec<Vec<u8>>, chunked: bool, stop: &AtomicBool) {
    let start = Instant::now();
    for (n, c) in chunks.into_iter().enumerate() {
        let data = if chunked { [format!("{:x}\r\n", c.len()).into_bytes(), c, b"\r\n".to_vec()].concat() } else { c };
        if stop.load(Ordering::SeqCst) || s.write_all(&data).is_err() {
            return;
        }
        let target = start + Duration::from_millis((n as u64 + 1) * 1000 / 30);
        if let Some(d) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
}

/// 止めるまで送り続けるスレッド。落とすときに止める
struct Sender {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sender {
    fn spawn(f: impl FnOnce(&AtomicBool) + Send + 'static) -> Sender {
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        Sender { stop, thread: Some(std::thread::spawn(move || f(&st))) }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// HTTP Push (chunked の POST) で FLV を配信する
fn push_flv(port: u16, name: &str) -> Sender {
    let name = name.to_string();
    Sender::spawn(move |stop| {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let head = format!(
            "POST /?name={}&type=FLV&bitrate=500 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: video/x-flv\r\nTransfer-Encoding: chunked\r\n\r\n",
            name, port
        );
        s.write_all(head.as_bytes()).unwrap();
        paced_send(&mut s, flv_stream(40), true, stop);
    })
}

/// 要求を送り、応答を `nbytes` バイトまで (または `timeout` まで) 読む
fn read_stream(port: u16, head: &str, nbytes: usize, timeout: Duration) -> Vec<u8> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    s.write_all(head.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let end = Instant::now() + timeout;
    let mut b = vec![0u8; 65536];
    while buf.len() < nbytes && Instant::now() < end {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&b[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
    buf
}

/// 200 OK で、本体が FLV のヘッダーから始まり、`min` バイト以上ある
fn check_flv(label: &str, data: &[u8], min: usize) {
    let i = data.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or_else(|| panic!("{}: ヘッダーの終わりがない", label));
    let (head, body) = (&data[..i], &data[i + 4..]);
    let status = String::from_utf8_lossy(head.split(|&c| c == b'\r').next().unwrap()).into_owned();
    assert!(status.ends_with("200 OK"), "{}: {}", label, status);
    assert!(body.starts_with(b"FLV\x01"), "{}: FLV でない", label);
    assert!(body.len() >= min, "{}: {} バイトしかない", label, body.len());
}

/// 配信 (HTTP Push) → 直接の視聴 → 別のサーバーでの PCP の中継 (もとは relay_test.py)
#[test]
fn relay() {
    let src = Server::start(17207);
    let relay = Server::start(17208);
    let _push = push_flv(src.port, "relaytest");
    let chans = wait_channels(src.port, 1);
    let cid = chans[0].1.clone();
    let head = |port: u16, q: &str| format!("GET /stream/{}.flv{} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", cid, q, port);
    check_flv("direct", &read_stream(src.port, &head(src.port, ""), 200000, Duration::from_secs(20)), 100000);
    let tip = format!("?tip=127.0.0.1:{}", src.port);
    check_flv("relay", &read_stream(relay.port, &head(relay.port, &tip), 200000, Duration::from_secs(30)), 100000);
    assert_eq!(wait_channels(relay.port, 1)[0].1, cid);
}

/// MP3 のフレーム (MPEG1 Layer III 128kbps 44.1kHz、パディングなし: 417 バイト) を `n` 個
fn mp3_frames(n: usize, tag: u8) -> Vec<u8> {
    let mut frame = b"\xff\xfb\x90\x64".to_vec();
    frame.extend(std::iter::repeat(tag).take(413));
    frame.repeat(n)
}

/// ShoutCast (最初の行がパスワード) や Icecast (SOURCE) の形で MP3 を配信する
fn icy_push(port: u16, first_line: &'static [u8], headers: &'static [&'static [u8]]) -> Sender {
    Sender::spawn(move |stop| {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(&[first_line, b"\r\n"].concat()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut h: Vec<u8> = headers.iter().flat_map(|h| [*h, b"\r\n"].concat()).collect();
        h.extend_from_slice(b"\r\n");
        s.write_all(&h).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut resp = [0u8; 100];
        let _ = s.read(&mut resp);
        let mut i = 0u8;
        let t0 = Instant::now();
        while !stop.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(30) {
            if s.write_all(&mp3_frames(38, i)).is_err() {
                return;
            }
            i = i.wrapping_add(1);
            std::thread::sleep(Duration::from_millis(1000));
        }
    })
}

/// 配信元の種類ごと: HTTP の取得 (fetch)、ShoutCast と Icecast の放送、ICY のメタデータ付きの視聴
/// (もとは source_test.py)
#[test]
fn sources() {
    let s = Server::start_with(17209, |ini| ini.replacen("password = \r\n", "password = pass\r\n", 1));
    let p = s.port;

    // FLV を返す HTTP サーバー
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let httpd_port = l.local_addr().unwrap().port();
    let _httpd = Sender::spawn(move |stop| {
        let (mut c, _) = l.accept().unwrap();
        let mut req = Vec::new();
        let mut b = [0u8; 1024];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            match c.read(&mut b) {
                Ok(0) | Err(_) => return,
                Ok(n) => req.extend_from_slice(&b[..n]),
            }
        }
        assert!(req.starts_with(b"GET /live.flv "), "{}", String::from_utf8_lossy(&req));
        let _ = c.write_all(b"HTTP/1.0 200 OK\r\nContent-Type: video/x-flv\r\n\r\n");
        paced_send(&mut c, flv_stream(30), false, stop);
    });
    let params = format!(
        r#"{{"url": "http://127.0.0.1:{}/live.flv", "name": "fetched", "desc": "", "genre": "", "contact": "", "bitrate": 0, "type": "FLV"}}"#,
        httpd_port
    );
    let cid = jrpc_call(p, "fetch", &params);
    let cid = String::from_utf8_lossy(cid.as_string().expect("fetch の結果")).into_owned();
    let head = format!("GET /stream/{}.flv HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", cid, p);
    std::thread::sleep(Duration::from_secs(2));
    check_flv("fetch", &read_stream(p, &head, 100000, Duration::from_secs(20)), 50000);

    let _shout = icy_push(p, b"pass", &[b"icy-name:shout", b"icy-br:128", b"content-type:audio/mpeg"]);
    let _ice = icy_push(p, b"SOURCE pass /mnt", &[b"ice-name:ice", b"ice-bitrate:128", b"content-type:audio/mpeg"]);
    let t0 = Instant::now();
    let chans = loop {
        let chans = wait_channels(p, 1);
        if ["shout", "ice"].iter().all(|n| chans.iter().any(|c| c.0 == *n)) {
            break chans;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "チャンネルができない: {:?}", chans);
        std::thread::sleep(Duration::from_millis(200));
    };
    for n in ["shout", "ice"] {
        let id = &chans.iter().find(|c| c.0 == n).unwrap().1;
        let buf = read_stream(p, &format!("GET /stream/{}.mp3 HTTP/1.0\r\nIcy-MetaData:1\r\n\r\n", id), 20000, Duration::from_secs(5));
        let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(buf.len());
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        assert!(head.starts_with("ICY 200 OK") && head.contains("icy-metaint:"), "{}: {}", n, head);
    }
}
