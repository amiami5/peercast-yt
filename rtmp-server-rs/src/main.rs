use rtmpserver::session::Session;
use rtmpserver::sink::{open_sink, Splitter};
use rtmpserver::{log, Error};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::exit;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

fn die(message: &str) -> ! {
    eprintln!("{}", message);
    exit(1);
}

/// `-p PORT` を取り出し、残りを URL として返す。C++ 版 optparse() と同じ規則。
fn parse_args(args: &[String]) -> Result<(u16, Vec<String>), String> {
    let mut port: u16 = 1935;
    let mut urls = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-p" {
            let v = args.get(i + 1).ok_or("no value for option -p")?;
            port = v.parse().map_err(|_| format!("invalid port {:?}", v))?;
            i += 2;
        } else {
            urls.push(args[i].clone());
            i += 1;
        }
    }
    Ok((port, urls))
}

fn bind(port: u16) -> std::io::Result<TcpListener> {
    // C++ 版と同じく、まず IPv6 の全アドレス (Linux の既定ではデュアルスタック) に bind する。
    TcpListener::bind(("::", port)).or_else(|_| TcpListener::bind(("0.0.0.0", port)))
}

fn serve(client: TcpStream, urls: &[String]) {
    let _ = client.set_read_timeout(Some(READ_TIMEOUT));
    let _ = client.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = client.set_nodelay(true);

    // 出力先はクライアントごとに開く (C++ 版と同じ)。開けなければこのクライアントだけ諦める。
    let mut sinks: Vec<Box<dyn Write>> = Vec::new();
    for url in urls {
        match open_sink(url) {
            Ok(s) => sinks.push(s),
            Err(e) => {
                log!("Error: cannot open {}: {}", url, e);
                return;
            }
        }
    }

    let mut session = Session::new(client, Splitter(sinks));
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
    let (port, urls) = parse_args(&args).unwrap_or_else(|e| die(&e));
    if urls.is_empty() {
        die("no URL supplied");
    }

    let listener = bind(port).unwrap_or_else(|e| die(&format!("Can't bind port {}: {}", port, e)));

    loop {
        match listener.accept() {
            Ok((client, _)) => serve(client, &urls),
            Err(e) => {
                // EMFILE などで空回りしないよう少し待つ。
                log!("accept failed: {}", e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
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
        assert_eq!(parse_args(&v(&[])).unwrap(), (1935, vec![]));
        assert_eq!(
            parse_args(&v(&["url1", "-p", "9999", "url2"])).unwrap(),
            (9999, v(&["url1", "url2"]))
        );
        assert!(parse_args(&v(&["url1", "-p"])).is_err());
        assert!(parse_args(&v(&["-p", "abc"])).is_err());
        assert!(parse_args(&v(&["-p", "70000"])).is_err());
    }
}
