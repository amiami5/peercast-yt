//! `/cmd?q=` のコマンド (core/common/commands.cpp の `Commands`)。オプションの解釈は `crate::commands`。

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::chaninfo::ChanInfo;
use super::channel::{self, Channel};
use super::error::{Error, Result};
use super::host::{Host, Ip};
use super::log::Level;
use super::pcpconst::*;
use super::pcstr::{PcString, StrType};
use super::peercast::Peercast;
use super::socket::ClientSocket;
use super::stream::Stream;
use super::sys;
use crate::template::Value;

/// コマンドの出力。ログ (`AUX_LOG_FUNC_VECTOR` で受けたもの) と順番が前後しないように、いったん
/// ためてから書く
pub struct Out<'a> {
    s: &'a mut dyn Stream,
    buf: Rc<RefCell<Vec<u8>>>,
}

impl<'a> Out<'a> {
    pub fn new(s: &'a mut dyn Stream) -> Out<'a> {
        Out { s, buf: Rc::new(RefCell::new(Vec::new())) }
    }

    fn flush(&mut self) -> Result<()> {
        let data = std::mem::take(&mut *self.buf.borrow_mut());
        if data.is_empty() {
            Ok(())
        } else {
            self.s.write(&data)
        }
    }

    pub fn write(&mut self, d: impl AsRef<[u8]>) -> Result<()> {
        self.buf.borrow_mut().extend_from_slice(d.as_ref());
        self.flush()
    }

    pub fn line(&mut self, d: impl AsRef<[u8]>) -> Result<()> {
        self.write([d.as_ref(), b"\r\n"].concat())
    }

    /// `body` の間のログを "Error: " などを付けて出力に入れる (C++ 版の `runProcess` と `helo`)
    pub fn with_log<R>(&mut self, body: impl FnOnce(&mut Out) -> R) -> R {
        let buf = self.buf.clone();
        let r = super::log::with_aux(
            Box::new(move |ty, msg| {
                let mut b = buf.borrow_mut();
                if ty == Level::Error {
                    b.extend_from_slice(b"Error: ");
                } else if ty == Level::Warn {
                    b.extend_from_slice(b"Warning: ");
                }
                b.extend_from_slice(msg);
                b.extend_from_slice(b"\r\n");
            }),
            || body(self),
        );
        let _ = self.flush();
        r
    }
}

type Cancel<'a> = &'a dyn Fn() -> bool;
type Command = fn(&Arc<Peercast>, &mut Out, &[Vec<u8>], Cancel) -> Result<()>;

const COMMANDS: &[(&str, Command)] = &[
    ("chan", chan),
    ("date", date),
    ("echo", echo),
    ("expr", expr),
    ("flag", flag),
    ("get", get),
    ("helo", helo),
    ("help", help),
    ("log", log),
    ("notify", notify),
    ("nslookup", nslookup),
    ("pid", pid),
    ("pwd", pwd),
    ("shutdown", shutdown),
    ("sleep", sleep),
    ("ssl", ssl),
];

fn find(name: &[u8]) -> Option<Command> {
    COMMANDS.iter().find(|(n, _)| n.as_bytes() == name).map(|(_, f)| *f)
}

fn parse(argv: &[Vec<u8>], names: &[&str]) -> Result<(BTreeMap<Vec<u8>, Vec<u8>>, Vec<Vec<u8>>)> {
    let names: Vec<Vec<u8>> = names.iter().map(|n| n.as_bytes().to_vec()).collect();
    crate::commands::parse_options(argv, &names).map_err(|e| Error::format(String::from_utf8_lossy(&e).into_owned()))
}

/// `Commands::system`
pub fn system(pc: &Arc<Peercast>, out: &mut Out, cmdline: &[u8], cancel: Cancel) -> Result<()> {
    let r = (|| -> Result<()> {
        let words = crate::strutil::shellwords(cmdline).map_err(Error::format)?;
        if words.is_empty() {
            return out.line("Error: Empty command line");
        }
        match find(&words[0]) {
            Some(f) => f(pc, out, &words[1..], cancel),
            None => out.line([&b"Error: No such command '"[..], &words[0], b"'"].concat()),
        }
    })();
    match r {
        // ソケットに書けなくなったものは上に知らせる
        Err(e) if e.is_sock() => Err(e),
        Err(e) => out.line(format!("Error: {}", e.msg)),
        Ok(()) => Ok(()),
    }
}

fn pwd(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if !p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: pwd")?;
        return out.line("Print the name of the current working directory.");
    }
    let cwd = std::env::current_dir().map(|p| sys::path_to_bytes(&p)).map_err(Error::from)?;
    out.line(cwd)
}

fn date(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if !p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: date")?;
        return out.line("Display the current time.");
    }
    out.write(sys::time_string(sys::get_time()))
}

fn sleep(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.len() != 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: sleep NUMBER")?;
        return out.line("Pause for NUMBER seconds.");
    }
    let secs = crate::strtod::atof(&p[0]);
    let ms = secs * 1000.0;
    // C++ 版は double を unsigned int の引数に渡す (範囲の外は x86-64 と同じく切り捨て)
    let ms = if ms.is_nan() || ms <= -1.0 { 0 } else if ms >= u32::MAX as f64 { u32::MAX } else { ms as u32 };
    sys::sleep(ms);
    Ok(())
}

fn shutdown(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if !p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: shutdown")?;
        return out.line("Shutdown server.");
    }
    out.line("Server is shutting down NOW!")?;
    pc.servmgr.shutdown_timer.store(1, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// `amf0::format`: 長ければ折り返す
pub fn format_value(v: &Value, allowance: i32, indent: i32) -> Result<Vec<u8>> {
    let insp = v.inspect().map_err(|e| Error::general(format!("{:?}", e)))?;
    if allowance <= 0 || insp.len() <= allowance as usize {
        return Ok(insp);
    }
    let pad = |n: i32| vec![b' '; n.max(0) as usize];
    match v {
        Value::StrictArray(a) => {
            let mut out = b"[\n".to_vec();
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b",\n");
                }
                out.extend(pad(indent + 2));
                out.extend(format_value(e, allowance - indent, indent + 2)?);
            }
            out.push(b'\n');
            out.extend(pad(indent));
            out.push(b']');
            Ok(out)
        }
        Value::Object(m) | Value::Array(m) => {
            let mut out = b"{\n".to_vec();
            for (i, (k, e)) in m.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b",\n");
                }
                let key = Value::String(k.clone()).inspect().map_err(|e| Error::general(format!("{:?}", e)))?;
                out.extend(pad(indent + 2));
                out.extend_from_slice(&key);
                out.extend_from_slice(b": ");
                // C++ 版は size_t で計算して int に戻す (x86-64 と同じく下位 32 ビット)
                let a = (allowance as i64 - indent as i64 - key.len() as i64 - 3) as i32;
                out.extend(format_value(e, a, indent + 2)?);
            }
            out.push(b'\n');
            out.extend(pad(indent));
            out.push(b'}');
            Ok(out)
        }
        _ => Ok(insp),
    }
}

fn expr(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: expr EXPRESSION...")?;
        return out.line("Evaluate expression.");
    }
    let expression = p.join(&b" "[..]);
    let mut t = super::html::Template::new(pc, Vec::new(), b"");
    t.prepend_scope(super::html::root_scope(pc));
    let v = t.eval_str(&expression)?;
    out.line(format_value(&v, 80, 0)?)
}

fn pid(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if !p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: pid")?;
        return out.line("Show the process ID of the server process.");
    }
    out.line(std::process::id().to_string())
}

fn help(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], cancel: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.len() > 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: help [COMMAND]")?;
        return out.line("List available commands or print the usage of the specified command.");
    }
    if p.is_empty() {
        for (n, _) in COMMANDS {
            out.line(n)?;
        }
        Ok(())
    } else {
        match find(&p[0]) {
            Some(f) => f(pc, out, &[b"--help".to_vec()], cancel),
            None => out.line([&b"Error: No such command '"[..], &p[0], b"'"].concat()),
        }
    }
}

fn notify(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.len() != 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: notify MESSAGE")?;
        return out.line("Send a notification message.");
    }
    pc.notify_message(super::notif::NT_PEERCAST, &p[0]);
    Ok(())
}

fn echo(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["-v", "--help"])?;
    if o.contains_key(&b"--help"[..]) {
        return out.line("Usage: echo [-v] WORDS...");
    }
    if o.contains_key(&b"-v"[..]) {
        for (i, w) in p.iter().enumerate() {
            out.line([format!("[{}] ", i + 1).as_bytes(), w].concat())?;
        }
        Ok(())
    } else {
        out.line(p.join(&b" "[..]))
    }
}

/// `pickChannels`: 名前が同じもの、なければ ID の先頭が一致するもの
fn pick_channels(pc: &Arc<Peercast>, desig: &[u8]) -> Result<Vec<Arc<Channel>>> {
    let chs = pc.chanmgr.channels();
    let desig_s = super::pcstr::cut(desig);
    let res: Vec<Arc<Channel>> = chs.iter().filter(|c| c.st().info.name.data == desig_s).cloned().collect();
    if !res.is_empty() {
        return Ok(res);
    }
    let prefix = crate::strutil::upcase(desig);
    if prefix.is_empty() {
        return Err(Error::argument("Empty channel ID prefix"));
    }
    Ok(chs.into_iter().filter(|c| crate::strutil::upcase(super::chaninfo::id_str(&c.id()).as_bytes()).starts_with(&prefix)).collect())
}

fn id_name(c: &Channel) -> Vec<u8> {
    [super::chaninfo::id_str(&c.id()).as_bytes(), b" ", &c.st().info.name.data].concat()
}

fn ambiguous(out: &mut Out, desig: &[u8], chs: &[Arc<Channel>]) -> Result<()> {
    out.line([&b"'"[..], desig, b"' is ambiguous between the following:"].concat())?;
    for c in chs {
        out.line(id_name(c))?;
    }
    Ok(())
}

fn chan(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help", "--name", "--genre", "--url", "--bitrate", "--type", "--ipv"])?;
    let usage = |out: &mut Out| -> Result<()> {
        out.line("Usage: chan ls")?;
        out.line("       chan show CHANNEL")?;
        out.line("       chan set-url CHANNEL URL")?;
        out.line("       chan fetch SOURCE_URL [--name=NAME] [--genre==GENRE] [--bitrate=KBPS] [--type=TYPE] [--ipv=<4|6>]")
    };
    if p.is_empty() || o.contains_key(&b"--help"[..]) {
        return usage(out);
    }
    let sub = p[0].as_slice();
    if p.len() >= 3 && sub == b"set-url" {
        let chs = pick_channels(pc, &p[1])?;
        if chs.is_empty() {
            return out.line([&b"Error: Channel not found: "[..], &p[1]].concat());
        } else if chs.len() > 1 {
            return ambiguous(out, &p[1], &chs);
        }
        let it = &chs[0];
        let id = super::chaninfo::id_str(&it.id());
        let (ty, src, info) = {
            let st = it.st();
            (st.ty, st.src_type, st.info.clone())
        };
        if ty != channel::T_BROADCAST {
            return out.line(format!("Error: {} is not a broadcasting channel.", id));
        }
        if src != channel::SRC_URL {
            return out.line(format!("Error: The source type of {} is not URL.", id));
        }
        out.line(format!("Stopping channel {} ...", id))?;
        it.thread.shutdown();
        it.wait_thread();
        out.line("Channel stopped.")?;
        sys::sleep(5000);
        out.line("Restarting channel ...")?;
        let ch = pc.chanmgr.create_channel(pc, &info, None);
        ch.start_url(pc, &p[2]);
        out.line("Started.")
    } else if sub == b"ls" {
        for c in pc.chanmgr.channels() {
            out.line([&id_name(&c)[..], b" ", c.status_str().as_bytes()].concat())?;
        }
        Ok(())
    } else if p.len() == 2 && sub == b"show" {
        let chs = pick_channels(pc, &p[1])?;
        if chs.is_empty() {
            out.line([&b"Error: Channel not found: "[..], &p[1]].concat())
        } else if chs.len() > 1 {
            ambiguous(out, &p[1], &chs)
        } else {
            out.line(format_value(&chs[0].state(pc), 80, 0)?)
        }
    } else if sub == b"fetch" {
        if p.len() != 2 {
            return out.line("Error: chan-fetch only needs one argument.");
        }
        let get = |k: &str| o.get(k.as_bytes()).map(|v| super::pcstr::cut(v)).unwrap_or_default();
        let field = |k: &str| {
            let v = crate::utf8::valid(&get(k));
            PcString::with_type(&crate::utf8::truncate(&v, 255).unwrap_or(v), StrType::Ascii)
        };
        let mut info = ChanInfo::new();
        info.name = field("--name");
        info.desc = field("--desc");
        info.genre = field("--genre");
        info.url = field("--contact");
        info.bitrate = crate::http::atoi(&get("--bitrate"));
        info.set_content_type(&get("--type"));
        super::servent_http::set_broadcast_id_channel_id(pc, &mut info, &pc.chanmgr.broadcast_id());
        let c = pc.chanmgr.create_channel(pc, &info, None);
        if get("--ipv") == b"6" {
            c.st().ip_version = channel::IP_V6;
            crate::log_info!("Channel IP version set to 6");
            pc.servmgr.check_firewall_ipv6();
        }
        c.start_url(pc, &p[1]);
        out.line(super::chaninfo::id_str(&info.id))
    } else if sub == b"stop" {
        if p.len() != 2 {
            return out.line("chan-stop only needs one argument.");
        }
        let chs = pick_channels(pc, &p[1])?;
        if chs.is_empty() {
            out.line([&b"Error: Channel not found: "[..], &p[1]].concat())
        } else if chs.len() > 1 {
            ambiguous(out, &p[1], &chs)
        } else {
            let it = &chs[0];
            out.write(format!("Stopping channel {} ... ", super::chaninfo::id_str(&it.id())))?;
            it.thread.shutdown();
            it.wait_thread();
            out.line("done.")
        }
    } else {
        usage(out)
    }
}

fn get(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.len() != 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: get URL")?;
        return out.line("Get a web resource and print it.");
    }
    // C++ 版は位置引数でなく argv[0] を使う (docs/cpp-known-issues.md)
    match super::http::get(&argv[0]) {
        Ok(body) => out.write(body),
        Err(e) => out.write(format!("Error: {}", e.msg)),
    }
}

fn log(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], cancel: Cancel) -> Result<()> {
    let (o, _) = parse(argv, &["--help"])?;
    if o.contains_key(&b"--help"[..]) {
        out.line("Usage: log")?;
        return out.line("Start printing the log.");
    }
    let queue: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
    let q = queue.clone();
    let id = super::log::with_buffer(|b| {
        b.add_listener(Box::new(move |_t, ty, msg| {
            let chunk = [format!("[{}] ", ty.type_str()).as_bytes(), msg, b"\n"].concat();
            q.lock().unwrap_or_else(|e| e.into_inner()).push_back(chunk);
        }))
    });
    struct Remove(u32);
    impl Drop for Remove {
        fn drop(&mut self) {
            let id = self.0;
            super::log::with_buffer(|b| b.remove_listener(id));
        }
    }
    let _remove = Remove(id);
    while !cancel() {
        let chunks: Vec<Vec<u8>> = queue.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        for c in chunks {
            out.write(c)?;
        }
        sys::sleep(100);
    }
    Ok(())
}

fn nslookup(_: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if p.len() != 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: nslookup NAME")?;
        return out.line("Look up a host name.");
    }
    let s = &p[0];
    match Ip::parse(s) {
        Some(ip) => match sys::hostname_by_address(&ip) {
            Some(name) => out.line(name),
            None => out.line([&b"Error: '"[..], s, b"' not found"].concat()),
        },
        None => {
            let ips = super::host::resolve_all(s);
            if ips.is_empty() {
                out.line([&b"Error: '"[..], s, b"' not found: getaddrinfo failed"].concat())
            } else {
                for ip in ips {
                    out.line(ip.str())?;
                }
                Ok(())
            }
        }
    }
}

/// 読み書きしたものを覚えておくストリーム (`CopyingStream`)
struct Copying<'a> {
    inner: &'a mut ClientSocket,
    read: Vec<u8>,
    written: Vec<u8>,
}

impl Stream for Copying<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.extend_from_slice(&buf[..n]);
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.inner.write(data)?;
        self.written.extend_from_slice(data);
        Ok(())
    }

    fn eof(&mut self) -> Result<bool> {
        self.inner.eof()
    }
}

fn helo(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["-v", "--help"])?;
    if p.len() != 1 || o.contains_key(&b"--help"[..]) {
        out.line("Usage: helo [-v] HOST")?;
        return out.line("Perform a PCP handshake with HOST.");
    }
    let verbose = o.contains_key(&b"-v"[..]);
    let target = p[0].clone();
    out.with_log(|out| {
        let r = (|| -> Result<()> {
            let host = Host::from_string(&target, super::servmgr::DEFAULT_PORT);
            out.line(format!("HELO {}", host.str()))?;
            let mut sock = ClientSocket::new();
            sock.set_read_timeout(30000);
            sock.connect(host)?;
            let rhost = sock.host;
            let mut cs = Copying { inner: &mut sock, read: Vec::new(), written: Vec::new() };
            let r = (|| -> Result<()> {
                let mut a = crate::pcp::write::AtomBuf::default();
                if host.ip.is_ipv4_mapped() {
                    a.int(PCP_CONNECT, 1);
                    crate::log_debug!("PCP_CONNECT 1");
                } else {
                    a.int(PCP_CONNECT, 100);
                    crate::log_debug!("PCP_CONNECT 100");
                }
                cs.write(&a.0)?;
                let (rid, agent) = super::servent::handshake_outgoing_pcp(pc, &mut cs, rhost, false)?;
                out.line(format!("Remote ID: {}", super::chaninfo::id_str(&rid)))?;
                out.line([&b"Remote agent: "[..], &agent.data].concat())?;
                let mut q = crate::pcp::write::AtomBuf::default();
                q.int(PCP_QUIT, PCP_ERROR_QUIT);
                cs.write(&q.0)
            })();
            if verbose {
                out.line(format!("--- {} bytes written", cs.written.len()))?;
                if !cs.written.is_empty() {
                    out.line(crate::strutil::ascii_dump(&cs.written, b"."))?;
                    out.line(crate::strutil::hexdump(&cs.written))?;
                }
                out.line(format!("--- {} bytes read", cs.read.len()))?;
                if !cs.read.is_empty() {
                    out.line(crate::strutil::ascii_dump(&cs.read, b"."))?;
                    out.line(crate::strutil::hexdump(&cs.read))?;
                }
            }
            r?;
            sock.close();
            Ok(())
        })();
        match r {
            Err(e) => out.line(format!("Error: {}", e.msg)),
            Ok(()) => Ok(()),
        }
    })
}

fn ssl(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    if !p.is_empty() || o.contains_key(&b"--help"[..]) {
        out.line("Usage: ssl")?;
        return out.line("Display the SSL server configuration.");
    }
    #[cfg(unix)]
    let (crt, key) = super::tls::server_configuration();
    #[cfg(not(unix))]
    let (crt, key) = (Vec::new(), Vec::new());
    out.line(format!("SSL server: {}", if pc.servmgr.flags.get("enableSSLServer") { "Enabled" } else { "Disabled" }))?;
    let show = |out: &mut Out, path: &[u8]| -> Result<()> {
        // C++ 版の sys->realPath は解決できなければ例外を投げる
        let full = sys::real_path(path).ok_or_else(|| Error::general(format!("realPath: {}", String::from_utf8_lossy(path))))?;
        out.line([&b"         Path: "[..], path].concat())?;
        out.line([&b"    Full path: "[..], &full].concat())?;
        let readable = super::stream::FileStream::open_read(&full).is_ok();
        out.line(format!("       Status: {}", if readable { "OK, Readable" } else { "Cannot open!" }))
    };
    out.line("\nCertificate File")?;
    show(out, &crt)?;
    out.line("\nPrivate Key File")?;
    show(out, &key)
}

fn flag_line(f: &super::flag::Flag) -> String {
    format!("{:<31} {}{}", f.name, if f.get() { "true" } else { "false" }, if f.get() == f.default_value { " (default)" } else { "" })
}

fn flag(pc: &Arc<Peercast>, out: &mut Out, argv: &[Vec<u8>], _: Cancel) -> Result<()> {
    let (o, p) = parse(argv, &["--help"])?;
    let usage = |out: &mut Out| -> Result<()> {
        out.line("Usage: flag ls")?;
        out.line("       flag set FLAG <true|false|default>")?;
        out.line("Manipulate flags.")
    };
    if o.contains_key(&b"--help"[..]) {
        return usage(out);
    }
    if p.len() == 1 && p[0] == b"ls" {
        for f in pc.servmgr.flags.sorted() {
            out.line(flag_line(f))?;
        }
        Ok(())
    } else if p.len() == 3 && p[0] == b"set" {
        let f = match pc.servmgr.flags.find(&p[1]) {
            Some(f) => f,
            None => return out.line([&b"Flag not found: "[..], &p[1]].concat()),
        };
        match p[2].as_slice() {
            b"true" => f.set(true),
            b"false" => f.set(false),
            b"default" => f.set(f.default_value),
            v => return out.line([&b"Invalid value: "[..], v].concat()),
        }
        out.line(flag_line(f))?;
        pc.save_settings();
        pc.notify_message(super::notif::NT_PEERCAST, "フラグ設定を保存しました。".as_bytes());
        Ok(())
    } else {
        usage(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::{arr, obj, s};

    #[test]
    fn format_short_and_long() {
        let v = obj(vec![("a", s("x")), ("b", arr(vec![s("y"), s("z")]))]);
        assert_eq!(format_value(&v, 80, 0).unwrap(), br#"{"a":"x","b":["y","z"]}"#.to_vec());
        assert_eq!(format_value(&v, 10, 0).unwrap(), b"{\n  \"a\": \"x\",\n  \"b\": [\n    \"y\",\n    \"z\"\n  ]\n}".to_vec());
    }

    #[test]
    fn out_orders_log_lines() {
        let mut ss = crate::server::stream::StringStream::new();
        {
            let mut o = Out::new(&mut ss);
            o.line("a").unwrap();
            o.with_log(|o| {
                crate::server::log::add_log(Level::Error, b"bad");
                o.line("b").unwrap();
            });
        }
        assert_eq!(ss.str(), b"a\r\nError: bad\r\nb\r\n");
    }
}
