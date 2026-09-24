//! 実際に rtmp-server を起動して試す、運用上の頑健性のテスト (もとは Python の tests/robustness.py)。
//!   1. 何も送らずに居座るクライアント (slowloris) が読み取りのタイムアウト (30 秒) で切られ、次の配信を受けられる
//!   2. 出力先 (PeerCast) が途中で切断しても、サーバーが落ちずに次の配信を受けられる
//!   3. 出力先に接続できなくても、サーバーが落ちない

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---- 入力の組み立て (tests/session.rs と同じ)
fn amf_num(x: f64) -> Vec<u8> {
    let mut v = vec![0];
    v.extend_from_slice(&x.to_be_bytes());
    v
}
fn amf_str(s: &str) -> Vec<u8> {
    let mut v = vec![2];
    v.extend_from_slice(&(s.len() as u16).to_be_bytes());
    v.extend_from_slice(s.as_bytes());
    v
}
fn amf_null() -> Vec<u8> {
    vec![5]
}

fn chunked(cs: u8, ts: u32, ty: u8, sid: u32, payload: &[u8], chunk: usize) -> Vec<u8> {
    let mut out = vec![cs];
    out.extend_from_slice(&ts.to_be_bytes()[1..]);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
    out.push(ty);
    out.extend_from_slice(&sid.to_le_bytes());
    for (i, c) in payload.chunks(chunk).enumerate() {
        if i > 0 {
            out.push(0xc0 | cs);
        }
        out.extend_from_slice(c);
    }
    out
}

/// ハンドシェイク、connect から publish まで、onMetaData、SetChunkSize (4096)
fn publish_prefix() -> Vec<u8> {
    let mut v = vec![3];
    v.extend((0..1536).map(|i| (i % 251) as u8)); // C1
    v.extend(std::iter::repeat(0).take(1536)); // C2
    v.extend(chunked(3, 0, 0x14, 0, &[amf_str("connect"), amf_num(1.0), amf_null()].concat(), 128));
    v.extend(chunked(3, 0, 0x14, 0, &[amf_str("createStream"), amf_num(2.0), amf_null()].concat(), 128));
    v.extend(chunked(4, 0, 0x14, 1, &[amf_str("publish"), amf_num(3.0), amf_null(), amf_str("key")].concat(), 128));
    let mut body = amf_str("@setDataFrame");
    body.extend(amf_str("onMetaData"));
    body.push(8); // ECMA array
    body.extend_from_slice(&2u32.to_be_bytes());
    for k in ["videocodecid", "audiocodecid"] {
        body.extend_from_slice(&(k.len() as u16).to_be_bytes());
        body.extend_from_slice(k.as_bytes());
        body.extend(amf_num(7.0));
    }
    body.extend_from_slice(&[0, 0, 9]);
    v.extend(chunked(4, 0, 0x12, 1, &body, 128));
    v.extend(chunked(2, 0, 0x01, 0, &4096u32.to_be_bytes(), 128));
    v
}

/// 映像と音声を 1 つずつ送って終わる配信
fn normal() -> Vec<u8> {
    let mut v = publish_prefix();
    v.extend(chunked(6, 40, 0x09, 1, &[vec![0x17, 1, 0, 0, 0], vec![0xaa; 1000]].concat(), 4096));
    v.extend(chunked(5, 40, 0x08, 1, &[0xaf, 1, 9, 9], 4096));
    v.extend(chunked(3, 0, 0x14, 0, &[amf_str("deleteStream"), amf_num(5.0), amf_null(), amf_num(1.0)].concat(), 128));
    v
}

/// 大きな映像のメッセージが続く配信
fn big_video_messages() -> Vec<u8> {
    let mut v = publish_prefix();
    for i in 0..8u32 {
        v.extend(chunked(6, i * 33, 0x09, 1, &[vec![0x17, 1, 0, 0, 0], vec![i as u8; 200_000]].concat(), 4096));
    }
    v
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// 起動した rtmp-server。落とすときに止める
struct Server(Child);

impl Server {
    fn start(port: u16, url: &str) -> Server {
        let child = Command::new(env!("CARGO_BIN_EXE_rtmp-server"))
            .arg("-p")
            .arg(port.to_string())
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("rtmp-server を起動できない");
        let mut s = Server(child);
        // 待ち受けを始めるまで待つ (つないですぐ閉じる)
        let t0 = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(s.alive(), "rtmp-server がすぐに終わった");
            assert!(t0.elapsed() < Duration::from_secs(10), "rtmp-server が待ち受けを始めない");
            std::thread::sleep(Duration::from_millis(50));
        }
        s
    }

    fn alive(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 出力先 (PeerCast の代わり)。受けた接続ごとの中身を溜める
fn sink() -> (u16, Arc<Mutex<Vec<Vec<u8>>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let conns = Arc::new(Mutex::new(Vec::new()));
    let c2 = conns.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = match s {
                Ok(s) => s,
                Err(_) => return,
            };
            let c3 = c2.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = s.read_to_end(&mut buf);
                c3.lock().unwrap().push(buf);
            });
        }
    });
    (port, conns)
}

/// 送って、書き込み側を閉じ、相手が閉じるまで読む
fn send_all(port: u16, data: &[u8], timeout: Duration) {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(timeout)).unwrap();
    if c.write_all(data).is_ok() {
        let _ = c.shutdown(Shutdown::Write);
    }
    let mut buf = [0u8; 65536];
    while let Ok(n) = c.read(&mut buf) {
        if n == 0 {
            break;
        }
    }
}

#[test]
fn slowloris_is_timed_out() {
    let (sport, conns) = sink();
    let port = free_port();
    let mut srv = Server::start(port, &format!("http://127.0.0.1:{}/?name=t", sport));
    // C0 だけ送って黙る
    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    slow.write_all(b"\x03").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    // 待ち行列に並ぶ
    send_all(port, &normal(), Duration::from_secs(60));
    let waited = t0.elapsed();
    std::thread::sleep(Duration::from_millis(300));
    let flv = conns.lock().unwrap().iter().filter(|c| c.windows(3).any(|w| w == b"FLV")).count();
    assert!(waited >= Duration::from_secs(25) && waited <= Duration::from_secs(40), "waited {:?}", waited);
    assert_eq!(flv, 1);
    assert!(srv.alive());
    drop(slow);
}

#[test]
fn survives_sink_disconnect() {
    // 受けてすぐ閉じる出力先
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let sport = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            drop(s);
        }
    });
    let port = free_port();
    let mut srv = Server::start(port, &format!("http://127.0.0.1:{}/?name=t", sport));
    for _ in 0..3 {
        send_all(port, &big_video_messages(), Duration::from_secs(10));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(srv.alive());
    // 次の配信も受け付ける
    send_all(port, &normal(), Duration::from_secs(10));
    assert!(srv.alive());
}

#[test]
fn survives_unreachable_sink() {
    let port = free_port();
    let dead = free_port();
    let mut srv = Server::start(port, &format!("http://127.0.0.1:{}/?name=t", dead));
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.write_all(b"\x03").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    drop(c);
    std::thread::sleep(Duration::from_millis(300));
    assert!(srv.alive());
}
