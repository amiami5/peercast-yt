use rtmpserver::session::Session;
use rtmpserver::sink::{open_sink, Splitter};
use rtmpserver::{log, Error};
use std::io::{self, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::exit;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// 接続してから配信 (publish) が始まるまでの期限の既定値 (秒)
const DEFAULT_PUBLISH_TIMEOUT: u64 = 10;
/// ストリームキーを渡す環境変数。コマンドラインだと同じ PC のほかのユーザーからも見えるため
const STREAM_KEY_ENV: &str = "PEERCAST_RTMP_STREAM_KEY";

fn die(message: &str) -> ! {
    eprintln!("{}", message);
    exit(1);
}

#[derive(Debug, PartialEq)]
struct Options {
    port: u16,
    /// 待ち受けるアドレス。空なら全アドレス (C++ 版と同じ)
    binds: Vec<IpAddr>,
    /// publish までの期限 (秒)。0 なら期限なし
    publish_timeout: u64,
    urls: Vec<String>,
}

/// `-p PORT`、`-b ADDR` (何度でも)、`-t SECONDS` を取り出し、残りを URL とする。
/// `-b` と `-t` 以外は C++ 版 optparse() と同じ規則。
fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options { port: 1935, binds: Vec::new(), publish_timeout: DEFAULT_PUBLISH_TIMEOUT, urls: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let opt = args[i].as_str();
        if opt == "-p" || opt == "-b" || opt == "-t" {
            let v = args.get(i + 1).ok_or(format!("no value for option {}", opt))?;
            match opt {
                "-p" => o.port = v.parse().map_err(|_| format!("invalid port {:?}", v))?,
                "-b" => {
                    // [::1] のような書き方も受け付ける
                    let a = v.trim_start_matches('[').trim_end_matches(']');
                    o.binds.push(a.parse().map_err(|_| format!("invalid address {:?}", v))?);
                }
                _ => o.publish_timeout = v.parse().map_err(|_| format!("invalid timeout {:?}", v))?,
            }
            i += 2;
        } else {
            o.urls.push(args[i].clone());
            i += 1;
        }
    }
    Ok(o)
}

fn bind(port: u16, binds: &[IpAddr]) -> Vec<TcpListener> {
    if binds.is_empty() {
        // C++ 版と同じく、まず IPv6 の全アドレス (Linux の既定ではデュアルスタック) に bind する。
        return match TcpListener::bind(("::", port)).or_else(|_| TcpListener::bind(("0.0.0.0", port))) {
            Ok(l) => vec![l],
            Err(e) => die(&format!("Can't bind port {}: {}", port, e)),
        };
    }
    // 127.0.0.1 と ::1 のように複数指定されたときは、bind できたものだけで待ち受ける
    // (IPv6 が無効な環境で ::1 に bind できなくても動くように)。
    let mut ls = Vec::new();
    for a in binds {
        match TcpListener::bind((*a, port)) {
            Ok(l) => ls.push(l),
            Err(e) => log!("Can't bind {} port {}: {}", a, port, e),
        }
    }
    if ls.is_empty() {
        die(&format!("Can't bind port {}", port));
    }
    ls
}

/// 出力先。最初に書くときに開く (publish の前に切られた接続やストリームキーの違う接続では、
/// PeerCast に接続しないように)。
struct LazySinks {
    urls: Vec<String>,
    opened: Option<Splitter>,
}

impl LazySinks {
    fn get(&mut self) -> io::Result<&mut Splitter> {
        if self.opened.is_none() {
            let mut sinks: Vec<Box<dyn Write>> = Vec::new();
            for url in &self.urls {
                match open_sink(url) {
                    Ok(s) => sinks.push(s),
                    Err(e) => {
                        log!("Error: cannot open {}: {}", url, e);
                        return Err(io::Error::other(format!("cannot open {}", url)));
                    }
                }
            }
            self.opened = Some(Splitter(sinks));
        }
        Ok(self.opened.as_mut().expect("opened above"))
    }
}

impl Write for LazySinks {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.get()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.opened {
            Some(s) => s.flush(),
            None => Ok(()),
        }
    }
}

fn serve(client: TcpStream, o: &Options, stream_key: Option<&[u8]>) {
    let publish_timeout = Duration::from_secs(o.publish_timeout);
    let first_timeout = if o.publish_timeout > 0 { publish_timeout.min(READ_TIMEOUT) } else { READ_TIMEOUT };
    let _ = client.set_read_timeout(Some(first_timeout));
    let _ = client.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = client.set_nodelay(true);
    // publish が始まったら、読み取りのタイムアウトを通常の長さに戻すための複製
    let restore = client.try_clone().ok();

    let mut session = Session::new(client, LazySinks { urls: o.urls.clone(), opened: None });
    if let Some(k) = stream_key {
        session.set_stream_key(k);
    }
    if o.publish_timeout > 0 {
        session.set_publish_deadline(
            Instant::now() + publish_timeout,
            Box::new(move || {
                if let Some(c) = &restore {
                    let _ = c.set_read_timeout(Some(READ_TIMEOUT));
                }
            }),
        );
    }
    // 不正なクライアントのためにサーバー全体を落とさない。
    match catch_unwind(AssertUnwindSafe(|| session.run())) {
        Ok(Ok(())) => {}
        Ok(Err(Error::Eof)) => log!("EOFException: end of stream"),
        Ok(Err(e)) => log!("Error: {}", e),
        Err(_) => log!("Error: internal error (panic)"),
    }
    // session が drop されると、クライアントと全ての出力先が閉じられる。
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = parse_args(&args).unwrap_or_else(|e| die(&e));
    if o.urls.is_empty() {
        die("no URL supplied");
    }
    let stream_key = std::env::var_os(STREAM_KEY_ENV).map(|k| k.to_string_lossy().into_owned()).filter(|k| !k.is_empty());

    // 待ち受けごとにスレッドで受け付け、配信は今までどおり 1 本ずつ順に処理する。
    let (tx, rx) = mpsc::sync_channel::<TcpStream>(8);
    for listener in bind(o.port, &o.binds) {
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            match listener.accept() {
                Ok((client, _)) => {
                    if tx.send(client).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    // EMFILE などで空回りしないよう少し待つ。
                    log!("accept failed: {}", e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
    drop(tx);

    for client in rx {
        serve(client, &o, stream_key.as_deref().map(str::as_bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn args() {
        let o = parse_args(&v(&[])).unwrap();
        assert_eq!((o.port, o.binds.len(), o.publish_timeout, o.urls.len()), (1935, 0, DEFAULT_PUBLISH_TIMEOUT, 0));
        let o = parse_args(&v(&["url1", "-p", "9999", "url2"])).unwrap();
        assert_eq!((o.port, o.urls), (9999, v(&["url1", "url2"])));
        assert!(parse_args(&v(&["url1", "-p"])).is_err());
        assert!(parse_args(&v(&["-p", "abc"])).is_err());
        assert!(parse_args(&v(&["-p", "70000"])).is_err());
    }

    #[test]
    fn bind_and_timeout_args() {
        let o = parse_args(&v(&["-b", "127.0.0.1", "-b", "[::1]", "-t", "0", "url"])).unwrap();
        assert_eq!(o.binds, vec!["127.0.0.1".parse::<IpAddr>().unwrap(), "::1".parse().unwrap()]);
        assert_eq!(o.publish_timeout, 0);
        assert_eq!(o.urls, v(&["url"]));
        assert!(parse_args(&v(&["-b", "localhost", "url"])).is_err());
        assert!(parse_args(&v(&["-b"])).is_err());
        assert!(parse_args(&v(&["-t", "-1", "url"])).is_err());
    }
}
