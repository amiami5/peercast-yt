//! サーバーを実際に起動して試すテスト (もとは Ruby の bvt/)。
//!
//! テストごとに一時ディレクトリに UI (html/) と設定ファイルを置き、別々のポートで `peercast` を起こす。
//! 設定は `tests/peercast.ini` (YP は空にしてあり、外のホストにはつながない)。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
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
        let dir = std::env::temp_dir().join(format!("peercast-bvt-{}-{}", std::process::id(), port));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ini = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/peercast.ini")).unwrap();
        let ini = ini.replace("serverPort = 7144", &format!("serverPort = {}", port));
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
