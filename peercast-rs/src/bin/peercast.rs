//! PeerCast YT のサーバー (C++ 版の ui/linux/main.cpp)。
//!
//! 使い方は `peercast --help`。設定ファイルなどの場所は XDG Base Directory に従う。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use peercast_rs::server::app::App;
use peercast_rs::server::http::PCX_VERSTRING;
use peercast_rs::server::log::{self, Level};
use peercast_rs::server::os;
use peercast_rs::server::peercast::Peercast;
use peercast_rs::server::sys;

static QUIT: AtomicBool = AtomicBool::new(false);
static GOT_INT: AtomicBool = AtomicBool::new(false);
static GOT_TERM: AtomicBool = AtomicBool::new(false);
static GOT_HUP: AtomicBool = AtomicBool::new(false);

/// シグナルハンドラーではフラグを立てるだけにする (C++ 版はハンドラーの中でログを書いていた)
extern "C" fn sig_proc(sig: std::os::raw::c_int) {
    match sig {
        os::SIGINT => {
            GOT_INT.store(true, Ordering::SeqCst);
            QUIT.store(true, Ordering::SeqCst);
            os::default_signal(os::SIGINT);
        }
        os::SIGTERM => {
            GOT_TERM.store(true, Ordering::SeqCst);
            QUIT.store(true, Ordering::SeqCst);
            os::default_signal(os::SIGTERM);
        }
        os::SIGHUP => GOT_HUP.store(true, Ordering::SeqCst),
        _ => {}
    }
}

fn bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    sys::path_to_bytes(std::path::Path::new(s))
}

fn env(name: &str) -> Option<Vec<u8>> {
    std::env::var_os(name).map(|v| bytes(&v))
}

/// `mkdir_p`
fn mkdir_p(dir: &[u8]) -> bool {
    let p = match sys::bytes_to_path(dir) {
        Some(p) => p,
        None => return false,
    };
    if p.is_dir() {
        return true;
    }
    if p.exists() {
        eprintln!("mkdir_p: Not a directory `{}`", p.display());
        return false;
    }
    match std::fs::create_dir_all(&p) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("mkdir: {}", e);
            false
        }
    }
}

/// `getConfDir` など: `$XDG_xxx_HOME/peercast` か `$HOME/<fallback>/peercast`
fn xdg_dir(var: &str, fallback: &str) -> Vec<u8> {
    let mut dir = match env(var) {
        Some(d) => d,
        // HOME がなければ、C++ 版と同じくパスワードのデータベースから引く
        None => [&env("HOME").or_else(os::home_dir_of_user).unwrap_or_default()[..], fallback.as_bytes()].concat(),
    };
    dir.extend_from_slice(b"/peercast");
    if dir.first() == Some(&b'/') {
        mkdir_p(&dir);
    }
    dir
}

type LogFile = Arc<Mutex<Option<File>>>;

fn open_log(path: &[u8]) -> Option<File> {
    let p = sys::bytes_to_path(path)?;
    OpenOptions::new().append(true).create(true).open(p).ok()
}

/// `printLog`: `2024/01/02 03:04:05.678 [INFO] ...`
fn print_log(file: &LogFile, t: Level, msg: &[u8]) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let tm = os::localtime(now.as_secs() as i64).unwrap_or_default();
    let mut line = format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}.{:03} ",
        tm.year + 1900,
        tm.mon + 1,
        tm.mday,
        tm.hour,
        tm.min,
        tm.sec,
        now.subsec_millis()
    )
    .into_bytes();
    if t != Level::None {
        line.extend_from_slice(format!("[{}] ", t.type_str()).as_bytes());
    }
    line.extend_from_slice(msg);
    line.push(b'\n');
    let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
    match f.as_mut() {
        Some(f) => {
            let _ = f.write_all(&line);
        }
        None => {
            let out = std::io::stdout();
            let mut out = out.lock();
            let _ = out.write_all(&line);
            let _ = out.flush();
        }
    }
}

fn usage() {
    println!("peercast - P2P Streaming Server, version {}", PCX_VERSTRING);
    println!("\nCopyright (c) 2002-2006 PeerCast.org <code@peercast.org>");
    println!("This is free software; see the source for copying conditions.");
    println!("Usage: peercast [options]");
    println!("-i, --inifile <inifile>      specify ini file");
    println!("-l, --logfile <logfile>      specify log file");
    println!("-P, --path <path>            set path to html files");
    println!("-d, --daemon                 fork in background");
    println!("-p, --pidfile <pidfile>      specify pid file");
    println!("--enable-notify-send         enable notification through notify-send command");
    println!("-h, --help                   show this help");
}

fn main() {
    // 設定ファイルやアクセストークンのファイルを自分しか読めないようにする
    os::set_umask(0o077);

    let bindir = sys::dirname(&sys::executable_path());
    let mut html_path = [&bindir[..], b"/../share/peercast/"].concat();
    let confdir = xdg_dir("XDG_CONFIG_HOME", "/.config");
    let mut ini_filename = [&confdir[..], b"/peercast.ini"].concat();
    let mut pid_filename = [&confdir[..], b"/peercast.pid"].concat();
    let mut log_filename = [&confdir[..], b"/peercast.log"].concat();
    let state_dir = xdg_dir("XDG_STATE_HOME", "/.local/state");
    let cache_dir = xdg_dir("XDG_CACHE_HOME", "/.cache");

    let mut log_to_file = false;
    let mut fork_daemon = false;
    let mut set_pid_file = false;
    let mut enable_notify_send = false;

    let args: Vec<Vec<u8>> = std::env::args_os().skip(1).map(|a| bytes(&a)).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_slice();
        let next = |i: &mut usize| -> Option<Vec<u8>> {
            *i += 1;
            args.get(*i).cloned()
        };
        match a {
            b"--inifile" | b"-i" => {
                if let Some(v) = next(&mut i) {
                    ini_filename = v;
                }
            }
            b"--logfile" | b"-l" => {
                if let Some(v) = next(&mut i) {
                    log_to_file = true;
                    log_filename = v;
                }
            }
            b"--path" | b"-P" => {
                if let Some(v) = next(&mut i) {
                    match sys::real_path(&v) {
                        // 最後に "/" を足す
                        Some(p) => html_path = [&p[..], b"/"].concat(),
                        None => eprintln!("{}: No such file or directory", String::from_utf8_lossy(&v)),
                    }
                }
            }
            b"--help" | b"-h" => {
                usage();
                return;
            }
            b"--daemon" | b"-d" => fork_daemon = true,
            b"--pidfile" | b"-p" => {
                if let Some(v) = next(&mut i) {
                    set_pid_file = true;
                    pid_filename = v;
                }
            }
            b"--enable-notify-send" => enable_notify_send = true,
            _ => {
                println!("Invalid argument {}", String::from_utf8_lossy(a));
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let logfile: LogFile = Arc::new(Mutex::new(if log_to_file { open_log(&log_filename) } else { None }));
    {
        let f = logfile.clone();
        log::set_printer(Box::new(move |t, msg| print_log(&f, t, msg)));
    }

    if fork_daemon {
        peercast_rs::log_debug!("Forking to the background");
        os::daemonize();
    }

    // デーモンになったあとのプロセス ID を書く
    if set_pid_file {
        peercast_rs::log_debug!("Peercast PID is: {}", std::process::id());
        if let Some(p) = sys::bytes_to_path(&pid_filename) {
            if let Ok(mut f) = File::create(p) {
                let _ = writeln!(f, "{}", std::process::id());
            }
        }
    }

    let app = App {
        html_path,
        settings_dir: sys::dirname(&ini_filename),
        ini_filename: ini_filename.clone(),
        token_list_filename: [&state_dir[..], b"/tokens.json"].concat(),
        cache_dir,
        state_dir,
        enable_notify_send,
    };
    let pc = Peercast::new(app);
    pc.init();

    peercast_rs::log_info!("Config file: {}", String::from_utf8_lossy(&ini_filename));
    if log_to_file {
        peercast_rs::log_info!("Log file: {}", String::from_utf8_lossy(&log_filename));
    }
    if set_pid_file {
        peercast_rs::log_info!("PID file: {}", String::from_utf8_lossy(&pid_filename));
    }

    os::set_signal(os::SIGINT, sig_proc);
    os::set_signal(os::SIGTERM, sig_proc);
    os::set_signal(os::SIGHUP, sig_proc);

    while !QUIT.load(Ordering::SeqCst) {
        sys::sleep(1000);
        if GOT_HUP.swap(false, Ordering::SeqCst) {
            peercast_rs::log_debug!("Received HUP signal, reloading a new logfile");
            // logrotate などでログのファイルを移したあと、新しいファイルを開き直す
            if log_to_file {
                let mut f = logfile.lock().unwrap_or_else(|e| e.into_inner());
                *f = None;
                if let Some(p) = sys::bytes_to_path(&log_filename) {
                    let _ = std::fs::remove_file(p);
                }
                *f = open_log(&log_filename);
            }
        }
        if let Some(f) = logfile.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
        // サーバーが止まるよう求めた (シャットダウンのタイマーなど)
        if pc.is_quitting() {
            break;
        }
    }
    if GOT_INT.load(Ordering::SeqCst) {
        peercast_rs::log_debug!("Received INT signal");
    }
    if GOT_TERM.load(Ordering::SeqCst) {
        peercast_rs::log_debug!("Received TERM signal");
    }

    pc.save_settings();
    pc.quit();

    {
        let mut f = logfile.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = f.as_mut() {
            let _ = file.flush();
        }
        // このあとのログは標準出力に書く
        *f = None;
    }
    if set_pid_file {
        if let Some(p) = sys::bytes_to_path(&pid_filename) {
            let _ = std::fs::remove_file(p);
        }
    }
}
