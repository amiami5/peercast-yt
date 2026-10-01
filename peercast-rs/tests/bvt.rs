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
use peercast_rs::version::{PCP_CLIENT_VERSION, PCX_AGENT};

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
        "chanfilters.html",
        "console.html",
        "logout.html",
    ] {
        let r = get(p, &format!("/html/ja/{}", file));
        assert_eq!(r.code, 200, "{}", file);
        // 外部ライブラリの jQuery・jscolor は使わない
        let text = String::from_utf8_lossy(&r.body).to_lowercase();
        assert!(!text.contains("jquery") && !text.contains("jscolor") && !text.contains("$("), "{}", file);
    }
    // id= がないので Bad Request
    assert_eq!(get(p, "/html/ja/relayinfo.html").code, 400);
    assert_eq!(get(p, "/html/ja/play.html").code, 400);
}

/// broadcast.html: エンコーダーに設定する URL を、フォームの入力からサーバーが作る (もとは JavaScript)
#[test]
fn broadcast_urls() {
    let s = Server::start(17216);
    let p = s.port;
    let page = |q: &str| {
        let r = get(p, &format!("/html/ja/broadcast.html{}", q));
        assert_eq!(r.code, 200, "{}", q);
        String::from_utf8(r.body).unwrap()
    };
    let text = page("");
    assert!(!text.contains("<script src=\"js/"), "外の JavaScript を読んでいる");
    assert!(!text.contains("/?name="));
    let text = page("?push_name=a+%22%3Cb%3E%26&push_genre=g&push_type=FLV&push_ipv=6");
    let at = text.find("/?name=").expect("HTTP Push の URL");
    assert!(text[at..].starts_with("/?name=a+%22%3Cb%3E%26&amp;genre=g&amp;type=FLV&amp;ipv=6\"</textarea>"), "{}", &text[at..at + 100]);
    // 入力した値はエスケープしてフォームに戻す
    assert!(text.contains("name=\"push_name\" value=\"a &quot;&lt;b&gt;&amp;\""));
    assert!(text.contains("<option value=\"FLV\" selected>"));
    assert!(text.contains("<option value=\"6\" selected>"));
    assert!(!text.contains("<b>&"));
    let text = page("?wm_name=n&wm_genre=%3Cg%3E&wm_desc=&wm_url=");
    assert!(text.contains(">n;&lt;g&gt;</textarea>"));
    assert!(text.contains("/n;&lt;g&gt;</textarea>"));
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
    assert_eq!(str_at(&v, "agentName"), PCX_AGENT.as_bytes());
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
    assert_eq!(agent, PCX_AGENT);
    assert!(child(&children, b"sid").is_some(), "sid");
    let ver = child(&children, b"ver").expect("ver");
    assert_eq!(u32::from_le_bytes([ver[0], ver[1], ver[2], ver[3]]), PCP_CLIENT_VERSION);
    assert!(child(&children, b"rip").is_some(), "rip");
    assert!(child(&children, b"port").is_some(), "port");
}

/// 1〜65535 の外のポート番号は、16 ビットに切り詰めずに受け付けない
#[test]
fn port_out_of_range() {
    let s = Server::start(17211);
    let p = s.port;
    // RTMP サーバーのコマンド: エラーを返す
    for port in ["70000", "0", "-1"] {
        assert_eq!(get(p, &format!("/admin?cmd=control_rtmp&action=start&name=x&port={}", port)).code, 400, "{}", port);
    }
    // 設定画面: ポートは変えずに、同じポートの設定画面に戻る
    // (ほかの項目を送っていないので、HTML の許可などは外れる)
    let r = get(p, "/admin?cmd=apply&port=70000");
    assert_eq!(r.code, 302);
    assert_eq!(r.header("Location"), Some("/html/en/settings.html"));
    let ini = std::fs::read_to_string(s.dir.join("peercast.ini")).unwrap();
    assert!(ini.contains("serverPort = 17211"), "{}", ini);
    assert!(TcpStream::connect(("127.0.0.1", p)).is_ok());
}

/// `pass=` は、どのパスでも `?` の後ろの引数としてだけ効く
#[test]
fn pass_in_query() {
    let s = Server::start_with(17212, |ini| {
        ini.replacen("authType = cookie", "authType = http-basic", 1).replacen("password = \r\n", "password = pass\r\n", 1)
    });
    let p = s.port;
    // Host がループバックの名前でなければ、localhost からでも認証を省かない
    let code = |path: &str| request(p, &format!("GET {} HTTP/1.0\r\nHost: example.com\r\n\r\n", path), b"").code;
    assert_eq!(code("/html/en/index.html?pass=pass"), 200);
    assert_eq!(code("/admin?cmd=viewxml&pass=pass"), 200);
    assert_eq!(code("/cgi-bin/board.cgi?category=x&pass=pass"), 400); // 認証は通り、引数が足りない
    assert_eq!(code("/cmd?q=help&pass=pass"), 200);
    for path in ["/html/en/index.html", "/html/en/index.html&pass=pass", "/html/en/index.html?pass=x", "/cgi-bin/board.cgi&pass=pass", "/cmd?q=help"] {
        assert_eq!(code(path), 401, "{}", path);
    }
}

/// /cgi-bin: 掲示板ビューワーと flv.cgi (もとは CGI スクリプト)。外のホストにはつながない
#[test]
fn cgi_bin() {
    // flv.cgi はトランスコードが無効だと引数を見る前に 403 で断るので、有効にして引数の検証を確かめる
    let s = Server::start_with(17206, |ini| ini.replace("transcodingEnabled = No", "transcodingEnabled = Yes"));
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
    // 配信の中身は文書として開かせない (security-review #20)
    let head = String::from_utf8_lossy(head);
    for h in ["\r\nContent-Type: video/x-flv\r\n", "\r\nX-Content-Type-Options: nosniff\r\n", "\r\nContent-Security-Policy: sandbox\r\n"] {
        assert!(format!("{}\r\n", head).contains(h), "{}: {:?} がない: {}", label, h.trim(), head);
    }
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

/// チャンネル名の改行で `/stream/` の応答のヘッダーを書き足されない (security-review #21)。
/// 直接の視聴 (配信者から届いた名前) と、PCP の中継 (ほかのノードから届いた名前) の両方
#[test]
fn stream_header_injection() {
    let src = Server::start(17213);
    let relay = Server::start(17214);
    let _push = push_flv(src.port, "inj%0D%0AX-Injected:%201%0D%0A");
    let chans = wait_channels(src.port, 1);
    let cid = chans[0].1.clone();
    let head = |port: u16, q: &str| format!("GET /stream/{}.flv{} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", cid, q, port);
    let tip = format!("?tip=127.0.0.1:{}", src.port);
    for (label, data) in [
        ("direct", read_stream(src.port, &head(src.port, ""), 200000, Duration::from_secs(20))),
        ("relay", read_stream(relay.port, &head(relay.port, &tip), 200000, Duration::from_secs(30))),
    ] {
        check_flv(label, &data, 100000);
        let i = data.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&data[..i]);
        assert!(!head.contains("\r\nX-Injected:"), "{}: ヘッダーを書き足された: {}", label, head);
        assert!(head.contains("\r\nx-audiocast-name: injX-Injected: 1\r\n"), "{}: {}", label, head);
    }
}

/// リレー一覧のリンクの拡張子に、ほかから届いた `sext` をそのまま使わない (security-review #22)
#[test]
fn relays_stream_ext() {
    let s = Server::start_with(17217, |ini| {
        ini + "\n[RelayChannel]\nname = extest\nid = 0123456789ABCDEF0123456789ABCDEF\n\
               contentType = FLV\nstreamExt = /../../admin?cmd=stop&x=\nstayConnected = Yes\n[End]\n"
    });
    let text = String::from_utf8_lossy(&get(s.port, "/html/ja/relays.html").body).into_owned();
    assert!(text.contains("extest"), "チャンネルが一覧にない: {}", text);
    assert!(text.contains("<a href=\"/stream/0123456789ABCDEF0123456789ABCDEF.flv\">"), "{}", text);
    assert!(!text.contains("/../"), "{}", text);
}

/// viewxml の属性の値は、ほかのノードから届く `type` も含めてエスケープする (security-review #31)。
/// PCP で届くのと同じく、ini の `contentType` には表にない文字列もそのまま入る
#[test]
fn viewxml_escapes_values() {
    let s = Server::start_with(17227, |ini| {
        ini + "\n[RelayChannel]\nname = x\"<n>&'\nid = 0123456789ABCDEF0123456789ABCDEF\n\
               contentType = FLV\" evil=\"<script>\ngenre = g\"/><evil/>\nstayConnected = Yes\n[End]\n"
    });
    let r = get(s.port, "/admin?cmd=viewxml");
    assert_eq!(r.code, 200);
    let text = String::from_utf8(r.body).unwrap();
    assert!(text.contains("name=\"x&quot;&lt;n&gt;&amp;&#039;\""), "{}", text);
    assert!(text.contains("type=\"FLV&quot; evil=&quot;&lt;script&gt;\""), "{}", text);
    assert!(text.contains("genre=\"g&quot;/&gt;&lt;evil/&gt;\""), "{}", text);
    assert!(!text.contains("evil=\"") && !text.contains("<script") && !text.contains("<evil"), "{}", text);
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

/// ShoutCast の放送の 1 行目 (パスワード) を試しても、パスワードの締め出しを通ること。
/// localhost からは締め出さないので、このマシンのループバックでないアドレスからつなぐ
#[test]
fn shoutcast_password_lockout() {
    let lan = match lan_addr() {
        Some(ip) => ip,
        None => {
            eprintln!("ループバックでないアドレスがないので飛ばす");
            return;
        }
    };
    let s = Server::start_with(17218, |ini| ini.replacen("password = \r\n", "password = pass\r\n", 1));
    let first_line = |line: &str| -> String {
        let mut c = TcpStream::connect((lan, s.port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(format!("{}\r\n", line).as_bytes()).unwrap();
        let mut buf = [0u8; 256];
        let n = c.read(&mut buf).unwrap_or(0);
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        text.lines().next().unwrap_or("").to_string()
    };
    // 受け付けないメソッドの HTTP の要求は、パスワードを試したのとは数えない
    for _ in 0..8 {
        assert!(first_line("HEAD / HTTP/1.1").contains("400"));
    }
    assert_eq!(first_line("pass"), "OK2");
    // 外れた行は数え、5 回で締め出す。締め出している間は、当たっても 429 で区別できない
    for i in 0..5 {
        let r = first_line(&format!("guess{}", i));
        assert!(r.contains("400"), "{}: {}", i, r);
    }
    let r = first_line("pass");
    assert!(r.contains("429"), "{}", r);
    let r = first_line("guess");
    assert!(r.contains("429"), "{}", r);
}

/// PCP の PUSH で GIV のためにつなぎに行くのは、同時に全体で 8 つ、宛先ごとに 2 つまで (security-review #24)。
/// 相手の要求を読み終えるか接続が切れれば、また受け付ける
#[test]
fn push_giv_limit() {
    let src = Server::start(17219);
    let _push = push_flv(src.port, "givtest");
    wait_channels(src.port, 1);

    // GIV の宛先。つながれたら、宛先のアドレスと最初の行を覚え、要求は送らずに持っておく
    let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let lport = listener.local_addr().unwrap().port();
    let accepted: Arc<std::sync::Mutex<Vec<(std::net::IpAddr, String, TcpStream)>>> = Default::default();
    {
        let accepted = accepted.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { break };
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0u8; 64];
                let n = s.read(&mut buf).unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
                accepted.lock().unwrap().push((s.local_addr().unwrap().ip(), line, s));
            }
        });
    }

    // CIN としてつなぐ (配信中のノードは受け付ける)
    let mut s = TcpStream::connect(("127.0.0.1", src.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut out = AtomBuf::default();
    out.int(*b"pcp\n", 1);
    out.parent(id4(b"helo"), 3);
    out.string(id4(b"agnt"), b"PeerCast/0.1218 (YT50)");
    out.int(id4(b"ver"), 1218);
    out.bytes(id4(b"sid"), b"givtest-session!");
    s.write_all(&out.0).unwrap();
    let sid: [u8; 16] = match read_atom(&mut s) {
        Atom::Parent(id, c) if &id == b"oleh" => child(&c, b"sid").expect("sid").try_into().unwrap(),
        a => panic!("oleh expected: {:?}", match a { Atom::Parent(id, _) | Atom::Data(id, _) => id }),
    };
    // ほかに届くもの (ok、root) は読み捨てる
    let mut drain = s.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while matches!(drain.read(&mut buf), Ok(n) if n > 0) {}
    });

    let push = |s: &mut TcpStream, ip: [u8; 4]| {
        let mut mapped = [0u8; 16];
        mapped[10] = 0xff;
        mapped[11] = 0xff;
        mapped[12..].copy_from_slice(&ip);
        let mut out = AtomBuf::default();
        out.parent(id4(b"bcst"), 3);
        out.char(id4(b"ttl"), 1);
        out.bytes(id4(b"dest"), &sid);
        out.parent(id4(b"push"), 2);
        out.address(id4(b"ip"), &mapped);
        out.short(id4(b"port"), lport as i16);
        s.write_all(&out.0).unwrap();
    };
    let count = |ip: Option<[u8; 4]>| {
        let a = accepted.lock().unwrap();
        a.iter().filter(|(addr, _, _)| ip.map_or(true, |ip| *addr == std::net::IpAddr::from(ip))).count()
    };
    let wait_count = |n: usize| {
        let t0 = Instant::now();
        while count(None) < n && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // 多すぎないことも見るため、少し待つ
        std::thread::sleep(Duration::from_secs(2));
        count(None)
    };

    for _ in 0..3 {
        push(&mut s, [127, 0, 0, 1]);
    }
    for i in 2..=6 {
        push(&mut s, [127, 0, 0, i]);
        push(&mut s, [127, 0, 0, i]);
    }
    assert_eq!(wait_count(8), 8);
    assert_eq!(count(Some([127, 0, 0, 1])), 2);
    for (addr, line, _) in accepted.lock().unwrap().iter() {
        assert_eq!(line, "GIV", "{}", addr);
    }

    // 切れれば、また受け付ける
    accepted.lock().unwrap().clear();
    std::thread::sleep(Duration::from_millis(500));
    push(&mut s, [127, 0, 0, 7]);
    assert_eq!(wait_count(1), 1);
}

/// root atom (ホスト情報の更新間隔、ルートのメッセージ) は、rootHost (YP) への COUT で受け取った
/// ものだけ使う。更新間隔は 30〜3600 秒に収める
#[test]
fn root_atoms_from_yp_only() {
    // 偽の YP。helo を読んで oleh を返し、root atom を送る
    let yp = std::net::TcpListener::bind("127.0.0.1:17222").unwrap();
    let src = Server::start_with(17221, |ini| ini.replacen("rootHost = ", "rootHost = 127.0.0.1:17222", 1));
    let p = src.port;
    let _push = push_flv(p, "roottest");
    wait_channels(p, 1);

    let root = |updint: i32, msg: &[u8]| {
        let mut out = AtomBuf::default();
        out.parent(id4(b"root"), 2);
        out.int(id4(b"uint"), updint);
        out.string(id4(b"mesg"), msg);
        out
    };
    // 設定のページは root のときしか更新間隔を出さないので、読むためのテンプレートを置く
    std::fs::write(ui_dir().join("html/en/bvt-root.html"), "{$chanMgr.hostUpdateInterval}\n{$servMgr.rootMsg}\n").unwrap();
    let settings = || {
        let r = get(p, "/html/en/bvt-root.html");
        assert_eq!(r.code, 200);
        let body = String::from_utf8_lossy(&r.body).into_owned();
        let (huint, msg) = body.split_once('\n').unwrap();
        (huint.to_string(), msg.to_string())
    };
    let wait_msg = |m: &str| {
        let t0 = Instant::now();
        loop {
            let (huint, msg) = settings();
            if msg.contains(m) || t0.elapsed() > Duration::from_secs(20) {
                return (huint, msg);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };

    yp.set_nonblocking(true).unwrap();
    let t0 = Instant::now();
    let mut c = loop {
        match yp.accept() {
            Ok((c, _)) => break c,
            Err(_) => {
                assert!(t0.elapsed() < Duration::from_secs(20), "COUT がつなぎに来ない");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    c.set_nonblocking(false).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    match read_atom(&mut c) {
        Atom::Data(id, _) if &id == b"pcp\n" => {}
        _ => panic!("pcp\\n expected"),
    }
    assert!(matches!(read_atom(&mut c), Atom::Parent(id, _) if &id == b"helo"));
    let mut out = AtomBuf::default();
    out.parent(id4(b"oleh"), 2);
    out.string(id4(b"agnt"), b"PeerCast/0.1218");
    out.bytes(id4(b"sid"), b"fake-yp-session!");
    c.write_all(&out.0).unwrap();
    c.write_all(&root(5, b"from-yp").0).unwrap();
    let mut drain = c.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while matches!(drain.read(&mut buf), Ok(n) if n > 0) {}
    });
    let (huint, msg) = wait_msg("from-yp");
    assert!(msg.contains("from-yp"), "YP からのメッセージを使わない");
    assert_eq!(huint, "30", "更新間隔の下限");

    // CIN から送ったものは、そのままでも BCST の中でも使わない
    let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut out = AtomBuf::default();
    out.int(*b"pcp\n", 1);
    out.parent(id4(b"helo"), 3);
    out.string(id4(b"agnt"), b"PeerCast/0.1218 (YT50)");
    out.int(id4(b"ver"), 1218);
    out.bytes(id4(b"sid"), b"roottest-sessio!");
    s.write_all(&out.0).unwrap();
    assert!(matches!(read_atom(&mut s), Atom::Parent(id, _) if &id == b"oleh"));
    let mut drain = s.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while matches!(drain.read(&mut buf), Ok(n) if n > 0) {}
    });
    s.write_all(&root(900, b"from-cin").0).unwrap();
    let mut out = AtomBuf::default();
    out.parent(id4(b"bcst"), 3);
    out.char(id4(b"ttl"), 1);
    out.char(id4(b"grp"), 2);
    out.0.extend_from_slice(&root(901, b"from-bcst").0);
    s.write_all(&out.0).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let (huint, msg) = settings();
    assert!(!msg.contains("from-cin") && !msg.contains("from-bcst"), "CIN からのメッセージを使った");
    assert_eq!(huint, "30", "CIN からの更新間隔を使った");

    // YP からの大きすぎる値は上限に収める
    c.write_all(&root(100000, b"from-yp2").0).unwrap();
    let (huint, msg) = wait_msg("from-yp2");
    assert!(msg.contains("from-yp2"));
    assert_eq!(huint, "3600", "更新間隔の上限");
}

/// このマシンのループバックでないアドレス (経路を引くだけで、パケットは送らない)
fn lan_addr() -> Option<std::net::IpAddr> {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|u| u.connect("192.0.2.1:9").map(|_| u))
        .and_then(|u| u.local_addr())
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// 認証なしの要求の `?tip=` / `?ip=` ではヒットを足さず、名前も引かない。公開ディレクトリの
/// 再生ページからは中継を始めさせられない (security-review #25)
#[test]
fn connect_args_untrusted() {
    let s = Server::start_with(17220, |ini| ini.replacen("publicDirectory = No", "publicDirectory = Yes", 1));
    let p = s.port;
    // ヒットのあるチャンネルは、公開ディレクトリの index.txt に出る
    let found = || String::from_utf8_lossy(&get(p, "/public/index.txt").body).to_uppercase();
    let at = |host: std::net::IpAddr, path: &str| {
        let mut c = TcpStream::connect((host, p)).unwrap();
        // ヒットがあると /channel/ は応答までしばらく待つが、応答は見ないので待たない
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        c.write_all(format!("GET {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", path, p).as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = c.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).lines().next().unwrap_or("").to_string()
    };
    let lo: std::net::IpAddr = [127, 0, 0, 1].into();

    // 公開ディレクトリの再生ページ: 配信していないチャンネルは 404 で、中継も始めない
    let id1 = "11111111111111111111111111111111";
    assert_eq!(get(p, &format!("/public/play.html?id={}", id1)).code, 404);
    std::thread::sleep(Duration::from_millis(500));
    assert!(matches!(jrpc_call(p, "getChannels", "[]"), peercast_rs::json::Value::Array(a) if a.is_empty()));

    // 名前は引かない (localhost からでも。/channel/ は中継を始めない)
    let id2 = "22222222222222222222222222222222";
    at(lo, &format!("/channel/{}?tip=localhost:7144", id2));
    at(lo, &format!("/channel/{}?ip=localhost:7144", id2));
    assert!(!found().contains(id2), "{}", found());

    // ループバックでないアドレスからの要求では足さない
    let id3 = "33333333333333333333333333333333";
    if let Some(lan) = lan_addr() {
        for path in [format!("/stream/{}.flv?tip=127.0.0.1:7144", id3), format!("/channel/{}?tip=127.0.0.1:7144", id3), format!("/pls/{}?tip=127.0.0.1:7144", id3)] {
            let r = at(lan, &path);
            assert!(r.contains("404") || r.contains("503"), "{}: {}", path, r);
        }
        assert!(!found().contains(id3), "{}", found());
    } else {
        eprintln!("ループバックでないアドレスがないので、その確かめは飛ばす");
    }

    // localhost からの IP アドレスなら、これまでどおり足す
    at(lo, &format!("/channel/{}?tip=127.0.0.1:7144", id3));
    assert!(found().contains(id3), "{}", found());

    // ヒットのあるチャンネルでも、公開ディレクトリの再生ページからは中継を始めない (ヒットの先につなぎに来ない)
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let id4 = "44444444444444444444444444444444";
    at(lo, &format!("/channel/{}?tip={}", id4, listener.local_addr().unwrap()));
    assert!(found().contains(id4), "{}", found());
    assert_eq!(get(p, &format!("/public/play.html?id={}", id4)).code, 404);
    std::thread::sleep(Duration::from_secs(1));
    assert!(listener.accept().is_err(), "中継を始めた");
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
        assert!(head.contains("\r\nX-Content-Type-Options: nosniff\r\n"), "{}: {}", n, head);
    }
}

/// どの接続にも同じ応答を返す HTTP サーバー。ポート番号を返す
fn fixed_httpd(response: String) -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let response = response.clone();
            std::thread::spawn(move || {
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut req = Vec::new();
                let mut b = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match c.read(&mut b) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => req.extend_from_slice(&b[..n]),
                    }
                }
                let _ = c.write_all(response.as_bytes());
            });
        }
    });
    port
}

/// 外から取った URL のリダイレクト先やプレイリストの中身では、LAN やループバックの別のホストにつながない。
/// もとの URL と同じホストへのリダイレクトはこれまでどおり追う (security-review #33)
#[test]
fn no_fetch_into_internal() {
    let s = Server::start(17228);
    let p = s.port;
    // 内部のアドレスの代わり。もとの URL (127.0.0.1) と違うホストにする
    let internal = match std::net::TcpListener::bind("127.0.0.2:0") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("127.0.0.2 で待ち受けられないので飛ばす: {}", e);
            return;
        }
    };
    internal.set_nonblocking(true).unwrap();
    let internal_url = format!("http://127.0.0.2:{}/x", internal.local_addr().unwrap().port());
    let not_reached = |label: &str| {
        std::thread::sleep(Duration::from_secs(2));
        assert!(internal.accept().is_err(), "{}: 内部のアドレスにつないだ", label);
    };
    let redirect = fixed_httpd(format!("HTTP/1.0 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n", internal_url));
    let cmd_get = |url: &str| {
        let q = format!("get {}", url).replace(':', "%3A").replace('/', "%2F").replace(' ', "%20");
        // 内部のアドレスにつなぐと、そこが応答しないので返らない。待ちきれなくても続ける
        let mut c = TcpStream::connect(("127.0.0.1", p)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(format!("GET /cmd?q={} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", q, p).as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = c.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    };

    // コンソールの get (YP の index.txt の取得と同じ http::get)
    cmd_get(&format!("http://127.0.0.1:{}/", redirect));
    not_reached("get のリダイレクト");
    // 同じホストへのリダイレクトは追う
    let ok = fixed_httpd("HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_string());
    let same = fixed_httpd(format!("HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:{}/\r\nContent-Length: 0\r\n\r\n", ok));
    let out = cmd_get(&format!("http://127.0.0.1:{}/", same));
    assert!(out.contains("hello"), "{}", out);

    let fetch = |port: u16| {
        let params = format!(
            r#"{{"url": "http://127.0.0.1:{}/live", "name": "f{}", "desc": "", "genre": "", "contact": "", "bitrate": 0, "type": "FLV"}}"#,
            port, port
        );
        jrpc_call(p, "fetch", &params);
    };
    // 配信元の URL のリダイレクト先
    fetch(redirect);
    not_reached("配信元のリダイレクト");
    // HTTP で取ったプレイリストの中身
    let body = format!("{}\r\n", internal_url);
    let pls = fixed_httpd(format!("HTTP/1.0 200 OK\r\nContent-Type: audio/x-mpegurl\r\nContent-Length: {}\r\n\r\n{}", body.len(), body));
    fetch(pls);
    not_reached("プレイリストの中身");
}

/// このノード自身が送った要求 (User-Agent が PeerCast) は、localhost からでも認証を省かない (security-review #33)
#[test]
fn admin_distrusts_own_agent() {
    let s = Server::start_with(17229, |ini| {
        ini.replacen("authType = cookie", "authType = http-basic", 1).replacen("password = \r\n", "password = pass\r\n", 1)
    });
    let p = s.port;
    let code = |ua: &str| {
        let ua = if ua.is_empty() { String::new() } else { format!("User-Agent: {}\r\n", ua) };
        request(p, &format!("GET /admin?cmd=viewxml HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n{}\r\n", p, ua), b"").code
    };
    assert_eq!(code(""), 200);
    assert_eq!(code("Mozilla/5.0"), 200);
    assert_eq!(code(PCX_AGENT), 401);
    // パスワードを付ければ通る
    let r = request(p, &format!("GET /admin?cmd=viewxml&pass=pass HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nUser-Agent: {}\r\n\r\n", p, PCX_AGENT), b"");
    assert_eq!(r.code, 200);
}

/// ほかのサイトのページから送らされた要求 (CSRF) と、Host がほかのドメイン名の要求 (DNS リバインディング) では、
/// localhost からでも HTTP Push を受け付けず、中継も始めない。利用者がリンクを押して開いたものと、
/// ヘッダーのない要求 (プレイヤーなど) はこれまでどおり (security-review #34)
#[test]
fn cross_site_cannot_start_relay() {
    let s = Server::start(17230);
    let p = s.port;
    let mut n = 0;
    // 要求を送り、中継を始めたか (ヒットの先につなぎに来たか) を確かめる。ケースごとに別のチャンネル ID と待ち受けを使う
    let mut check = |label: &str, host: &str, extra: &str, path: &str, want: bool| {
        n += 1;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let path = path.replacen("{}", &format!("{:032x}", 0xa000 + n), 1).replacen("{}", &listener.local_addr().unwrap().to_string(), 1);
        let mut c = TcpStream::connect(("127.0.0.1", p)).unwrap();
        // 中継を始めると応答までしばらく待つが、応答は見ないので待たない
        c.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        c.write_all(format!("GET {} HTTP/1.1\r\nHost: {}\r\n{}\r\n", path, host, extra).as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = c.read_to_end(&mut buf);
        let t0 = Instant::now();
        let mut got = false;
        while t0.elapsed() < Duration::from_secs(if want { 10 } else { 2 }) {
            if listener.accept().is_ok() {
                got = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(got, want, "{}", label);
    };
    let lo = format!("127.0.0.1:{}", p);
    let cross = "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: video\r\n";
    let navigate = "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\nSec-Fetch-User: ?1\r\n";

    // 中継を始めないもの
    for (label, host, extra, path) in [
        ("stream: cross-site", lo.as_str(), cross, "/stream/{}.flv?tip={}"),
        ("stream: Origin", lo.as_str(), "Origin: http://evil.example.com\r\n", "/stream/{}.flv?tip={}"),
        ("stream: rebinding", "evil.example.com", "", "/stream/{}.flv?tip={}"),
        ("pls: cross-site", lo.as_str(), cross, "/pls/{}?tip={}"),
        ("pls: rebinding", "evil.example.com", "", "/pls/{}?tip={}"),
        ("play.html: cross-site", lo.as_str(), "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: iframe\r\n", "/html/en/play.html?id={}%3Ftip%3D{}"),
    ] {
        check(label, host, extra, path, false);
    }
    // これまでどおり中継を始めるもの
    for (label, host, extra, path) in [
        ("stream: no headers", lo.as_str(), "", "/stream/{}.flv?tip={}"),
        ("stream: same-origin", lo.as_str(), "Sec-Fetch-Site: same-origin\r\n", "/stream/{}.flv?tip={}"),
        ("stream: LAN name", "mypc.local", "", "/stream/{}.flv?tip={}"),
        ("pls: user navigation", lo.as_str(), navigate, "/pls/{}?tip={}"),
        ("play.html: user navigation", lo.as_str(), navigate, "/html/en/play.html?id={}%3Ftip%3D{}"),
    ] {
        check(label, host, extra, path, true);
    }

    // HTTP Push: ほかのサイトのページからの POST (利用者がフォームを送ったものも) と DNS リバインディングは断る
    let push = |host: &str, extra: &str| {
        let head = format!("POST /?name=csrf&type=FLV HTTP/1.1\r\nHost: {}\r\nContent-Type: video/x-flv\r\n{}\r\n", host, extra);
        request(p, &head, b"FLV\x01\x01\x00\x00\x00\x09\0\0\0\0").code
    };
    assert_eq!(push(&lo, "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: no-cors\r\n"), 403);
    assert_eq!(push(&lo, navigate), 403);
    assert_eq!(push(&lo, "Origin: null\r\n"), 403);
    assert_eq!(push("evil.example.com", ""), 403);
    // ヘッダーのない配信ソフトからは、これまでどおり受け付ける (中継のチャンネルも残っているかもしれないので名前で探す)
    let _push = push_flv(p, "pushok");
    let t0 = Instant::now();
    loop {
        let names: Vec<String> = wait_channels(p, 0).into_iter().map(|(name, _)| name).collect();
        assert!(!names.iter().any(|n| n == "csrf"), "{:?}", names);
        if names.iter().any(|n| n == "pushok") {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "pushok ができない: {:?}", names);
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------- TLS

/// テストの自己署名の証明書 server.crt と鍵 server.key を `dir` に作る
#[cfg(unix)]
fn make_certs(dir: &Path) {
    let args = [
        "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", "server.key", "-out",
        "server.crt", "-subj", "/CN=127.0.0.1", "-days", "2",
    ];
    let out = Command::new("openssl").args(args).current_dir(dir).stdin(Stdio::null()).output().expect("openssl を起動できない");
    assert!(out.status.success(), "openssl {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
}

/// 300 ミリ秒ごとに 1 バイト送り続け、`limit` までにサーバーが閉じることを確かめる
#[cfg(unix)]
fn assert_closed_while_trickling(mut c: TcpStream, byte: u8, limit: Duration) {
    let t0 = Instant::now();
    c.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let mut buf = [0u8; 1024];
    loop {
        assert!(t0.elapsed() < limit, "{:?} たっても切られない", limit);
        match c.read(&mut buf) {
            Ok(0) => return,
            // TLS の alert など
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if c.write_all(&[byte]).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// TLS でも、要求を読み終えるまでの期限 (handshakeTimeout) で切る。OpenSSL の中の recv のたびに
/// 読む待ち時間をまるごと使えたので、1 バイトずつ送る接続にいつまでも居座られていた
#[cfg(unix)]
#[test]
fn tls_slow_client() {
    use peercast_rs::server::tls::Session;
    use std::os::unix::io::AsRawFd;

    let s = Server::start_with(17223, |ini| {
        ini.replace("[Server]\r\n", "[Server]\r\nhandshakeTimeout = 2\r\n") + "\r\n[Flags]\r\nenableSSLServer = Yes\r\n[End]\r\n"
    });
    let p = s.port;
    make_certs(&s.dir);
    // ハンドシェイクできる (証明書が読めないとすぐに切られるので、下の確かめが意味をなさない)
    let c = TcpStream::connect(("127.0.0.1", p)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    Session::connect(c.as_raw_fd(), b"").expect("ハンドシェイク");
    drop(c);
    let limit = Duration::from_secs(5);

    // 平文は期限で切れる
    let mut c = TcpStream::connect(("127.0.0.1", p)).unwrap();
    c.write_all(b"GET /html/en/index.html HTTP/1.0\r\nX: ").unwrap();
    assert_closed_while_trickling(c, b'a', limit);

    // TLS のハンドシェイクを 1 バイトずつ (200 バイトのレコードと言っておく)
    let mut c = TcpStream::connect(("127.0.0.1", p)).unwrap();
    c.write_all(&[0x16, 0x03, 0x01, 0x00, 0xc8]).unwrap();
    assert_closed_while_trickling(c, 0x01, limit);

    // ハンドシェイクのあと、要求のレコードを 1 バイトずつ。暗号にしたレコードを少しずつ送るために、
    // 間に中継を置き、ハンドシェイクが済んだらクライアントからサーバーへを 1 バイトずつにする
    let proxy = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let slow = Arc::new(AtomicBool::new(false));
    let (closed_tx, closed_rx) = std::sync::mpsc::channel();
    let slow2 = slow.clone();
    std::thread::spawn(move || {
        let (mut a, _) = proxy.accept().unwrap();
        let mut b = TcpStream::connect(("127.0.0.1", p)).unwrap();
        let (mut a2, mut b2) = (a.try_clone().unwrap(), b.try_clone().unwrap());
        // サーバーからクライアントへ。サーバーが閉じたら知らせる
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut b2, &mut a2);
            let _ = closed_tx.send(Instant::now());
        });
        let mut buf = [0u8; 4096];
        while let Ok(n) = a.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &c in &buf[..n] {
                if slow2.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(300));
                }
                if b.write_all(&[c]).is_err() {
                    return;
                }
            }
        }
    });
    let c = TcpStream::connect(("127.0.0.1", proxy_port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut sess = Session::connect(c.as_raw_fd(), b"").expect("ハンドシェイク");
    // ハンドシェイクの最後のバイトが中継を通り終えるのを待つ
    std::thread::sleep(Duration::from_millis(200));
    slow.store(true, Ordering::SeqCst);
    let t0 = Instant::now();
    sess.write(format!("GET /html/en/index.html HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", p).as_bytes()).unwrap();
    let closed = closed_rx.recv_timeout(limit).expect("要求のレコードを 1 バイトずつ送ると切られない");
    assert!(closed - t0 < limit);
}

/// 速度測定の偽のサーバー: GET には yp4g.xml を、POST には 302 を返し、届いた要求を覚える
struct FakeUptest {
    port: u16,
    xml: Arc<std::sync::Mutex<String>>,
    requests: Arc<std::sync::Mutex<Vec<(String, usize)>>>,
}

impl FakeUptest {
    fn start(bind: &str) -> std::io::Result<FakeUptest> {
        let l = std::net::TcpListener::bind(bind)?;
        let port = l.local_addr().unwrap().port();
        let xml = Arc::new(std::sync::Mutex::new(String::new()));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (x, r) = (xml.clone(), requests.clone());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { continue };
                let (x, r) = (x.clone(), r.clone());
                std::thread::spawn(move || {
                    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut buf = Vec::new();
                    let mut b = [0u8; 4096];
                    let end = loop {
                        if let Some(e) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break e + 4;
                        }
                        match c.read(&mut b) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&b[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                    let len = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    while buf.len() < end + len {
                        match c.read(&mut b) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&b[..n]),
                        }
                    }
                    // 本体のあとに続くもの (書き足された要求) も覚える
                    let _ = c.set_read_timeout(Some(Duration::from_millis(300)));
                    while let Ok(n) = c.read(&mut b) {
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&b[..n]);
                    }
                    let rest = String::from_utf8_lossy(buf.get(end + len..).unwrap_or(&[])).into_owned();
                    r.lock().unwrap().push((head.clone() + &rest, buf.len().min(end + len) - end));
                    let res = if head.starts_with("GET ") {
                        let body = x.lock().unwrap().clone();
                        format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body)
                    } else {
                        "HTTP/1.0 302 Found\r\nLocation: /\r\nContent-Length: 0\r\n\r\n".to_string()
                    };
                    let _ = c.write_all(res.as_bytes());
                });
            }
        });
        Ok(FakeUptest { port, xml, requests })
    }

    fn set_srv(&self, addr: &str, port: &str, object: &str, post_size: &str) {
        *self.xml.lock().unwrap() = format!(
            "<yp4g><yp name=\"test\"/><host ip=\"127.0.0.1\" port_open=\"1\" speed=\"0\" over=\"0\"/>\
             <uptest checkable=\"1\" remain=\"0\"/>\
             <uptest_srv addr=\"{}\" port=\"{}\" object=\"{}\" post_size=\"{}\" limit=\"3000\" interval=\"15\" enabled=\"1\"/></yp4g>",
            addr, port, object, post_size
        );
    }

    /// 届いた POST (要求の頭と本体の長さ)
    fn posts(&self) -> Vec<(String, usize)> {
        self.requests.lock().unwrap().iter().filter(|(h, _)| !h.starts_with("GET ")).cloned().collect()
    }

    fn all(&self) -> String {
        self.requests.lock().unwrap().iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>().join("\n----\n")
    }
}

/// 速度測定は、yp4g.xml の `uptest_srv` の値で要求を書き足させず、LAN の中の別のアドレスへは送らず、
/// 大きすぎる `post_size` では送らずに誤りにする (security-review #29)
#[test]
fn uptest_srv_checked() {
    let mut s = Server::start(17224);
    let p = s.port;
    let fake = FakeUptest::start("127.0.0.1:0").unwrap();
    let fp = fake.port.to_string();
    fake.set_srv("127.0.0.1", &fp, "/uptest.cgi", "1");
    let r = get(p, &format!("/admin?cmd=add_speedtest&url=http%3A%2F%2F127.0.0.1%3A{}%2Fyp4g.xml", fp));
    assert_eq!(r.code, 302, "{}", String::from_utf8_lossy(&r.body));
    let take = || get(p, "/admin?cmd=take_speedtest&index=0");

    // yp4g.xml を取ったのと同じアドレスへは送る
    let r = take();
    assert_eq!(r.code, 302, "{}", String::from_utf8_lossy(&r.body));
    let posts = fake.posts();
    assert_eq!(posts.len(), 1, "{}", fake.all());
    assert!(posts[0].0.starts_with("POST /uptest.cgi HTTP/1.0\r\n"), "{}", posts[0].0);
    assert_eq!(posts[0].1, 1000);

    // object の改行で、ヘッダーや要求を書き足させない
    fake.set_srv("127.0.0.1", &fp, "/uptest.cgi\r\nX-Injected: 1\r\n\r\nGET /admin?cmd=injected HTTP/1.0\r\nX: ", "0");
    assert_eq!(take().code, 500);
    // addr の改行も
    fake.set_srv("127.0.0.1\r\nX-Injected: 1", &fp, "/uptest.cgi", "0");
    assert_eq!(take().code, 500);
    assert_eq!(fake.posts().len(), 1, "{}", fake.all());
    assert!(!fake.all().contains("Injected") && !fake.all().contains("injected"), "{}", fake.all());

    // LAN の中の、yp4g.xml を取ったのと違うアドレスへは送らない
    match std::net::TcpListener::bind("127.0.0.2:0") {
        Ok(other) => {
            other.set_nonblocking(true).unwrap();
            let op = other.local_addr().unwrap().port().to_string();
            fake.set_srv("127.0.0.2", &op, "/uptest.cgi", "1");
            assert_eq!(take().code, 500);
            std::thread::sleep(Duration::from_millis(300));
            assert!(other.accept().is_err(), "127.0.0.2 へつないだ");
        }
        Err(e) => eprintln!("127.0.0.2 で待ち受けられないので、その確かめは飛ばす: {}", e),
    }

    // 大きすぎる post_size では送らず、落ちない
    fake.set_srv("127.0.0.1", &fp, "/uptest.cgi", "2000000000");
    assert_eq!(take().code, 500);
    assert_eq!(fake.posts().len(), 1, "{}", fake.all());
    assert!(s.child.try_wait().unwrap().is_none(), "peercast が落ちた");

    // 直したあとも、正当な値なら送れる
    fake.set_srv("127.0.0.1", &fp, "/uptest.cgi?x=1", "2");
    assert_eq!(take().code, 302);
    let posts = fake.posts();
    assert_eq!(posts.len(), 2, "{}", fake.all());
    assert!(posts[1].0.starts_with("POST /uptest.cgi?x=1 HTTP/1.0\r\n"), "{}", posts[1].0);
    assert_eq!(posts[1].1, 2000);

    // IPv6 アドレスの addr ([] で囲んでも囲まなくてもよい)。yp4g.xml を [::1] から取れば ::1 へ送れる
    let fake6 = match FakeUptest::start("[::1]:0") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[::1] で待ち受けられないので、IPv6 の確かめは飛ばす: {}", e);
            return;
        }
    };
    let fp6 = fake6.port.to_string();
    // 127.0.0.1 から取った yp4g.xml で ::1 へは送らない
    fake.set_srv("::1", &fp6, "/uptest.cgi", "1");
    assert_eq!(take().code, 500);
    assert!(fake6.posts().is_empty(), "{}", fake6.all());
    let r = get(p, &format!("/admin?cmd=add_speedtest&url=http%3A%2F%2F%5B%3A%3A1%5D%3A{}%2Fyp4g.xml", fp6));
    assert_eq!(r.code, 302, "{}", String::from_utf8_lossy(&r.body));
    for (i, addr) in ["::1", "[::1]"].iter().enumerate() {
        fake6.set_srv(addr, &fp6, "/uptest.cgi", "1");
        let r = get(p, "/admin?cmd=take_speedtest&index=1");
        assert_eq!(r.code, 302, "{}: {}", addr, String::from_utf8_lossy(&r.body));
        let posts = fake6.posts();
        assert_eq!(posts.len(), i + 1, "{}", fake6.all());
        assert!(posts[i].0.contains("\r\nHost: [::1]\r\n"), "{}", posts[i].0);
        assert_eq!(posts[i].1, 1000);
    }
}

/// キャッシュの yp4g.xml は、管理画面のオリジンで XML (XHTML の script が動く) として開かせない (security-review #30)
#[test]
fn speedtest_cached_xml_as_text() {
    let s = Server::start(17225);
    let p = s.port;
    let fake = FakeUptest::start("127.0.0.1:0").unwrap();
    let fp = fake.port.to_string();
    let script = "<x:script xmlns:x=\"http://www.w3.org/1999/xhtml\">alert(1)</x:script>";
    fake.set_srv("127.0.0.1", &fp, "/uptest.cgi", "1");
    {
        let mut x = fake.xml.lock().unwrap();
        *x = x.replacen("</yp4g>", &format!("{}</yp4g>", script), 1);
    }
    let r = get(p, &format!("/admin?cmd=add_speedtest&url=http%3A%2F%2F127.0.0.1%3A{}%2Fyp4g.xml", fp));
    assert_eq!(r.code, 302, "{}", String::from_utf8_lossy(&r.body));
    // 測定のあとに yp4g.xml を取り直してキャッシュする
    let r = get(p, "/admin?cmd=take_speedtest&index=0");
    assert_eq!(r.code, 302, "{}", String::from_utf8_lossy(&r.body));
    let r = get(p, "/admin?cmd=speedtest_cached_xml&index=0");
    assert_eq!(r.code, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(String::from_utf8_lossy(&r.body).contains(script), "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.header("Content-Type"), Some("text/plain; charset=utf-8"));
    assert_eq!(r.header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(r.header("Content-Security-Policy"), Some("sandbox"));
}
