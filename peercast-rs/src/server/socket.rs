//! ソケット (core/unix/usocket.cpp の `UClientSocket`、sslclientsocket.cpp の `SslClientSocket`)。
//!
//! C++ 版は非ブロッキングのソケットを `select` で待っていた。Rust 版はブロッキングのソケットに
//! タイムアウト (`SO_RCVTIMEO` / `SO_SNDTIMEO`) を付ける。待ち時間が過ぎたら `TimeoutException`、
//! 相手が閉じたら `EOFException` ("Closed on read") になるのは同じ。
//!
//! ほかのスレッドから接続を切る (`Servent::abort` など) ために、`Closer` で同じソケットを
//! `shutdown` できる。

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::error::{strerror, Error, Result};
use super::host::{Host, Ip};
use super::stats::{self, Stat as S};
use super::stream::{Stat, Stream};

enum Conn {
    Tcp(TcpStream),
    #[cfg(unix)]
    Tls(TcpStream, super::tls::Session),
}

impl Conn {
    fn tcp(&self) -> &TcpStream {
        match self {
            Conn::Tcp(s) => s,
            #[cfg(unix)]
            Conn::Tls(s, _) => s,
        }
    }
}

/// ほかのスレッドから接続を切るためのもの
#[derive(Clone, Default)]
pub struct Closer(Arc<Mutex<Option<TcpStream>>>);

impl Closer {
    /// 接続を切る (読み書きしているスレッドは誤りで戻る)
    pub fn close(&self) {
        if let Some(s) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }

    fn set(&self, s: Option<TcpStream>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
}

/// `ClientSocket`
pub struct ClientSocket {
    conn: Option<Conn>,
    pub host: Host,
    read_timeout: u32,
    write_timeout: u32,
    stat: Arc<Stat>,
    closer: Closer,
    /// TLS で接続するときの、検証するホスト名 (`SslClientSocket::setHostname`)
    tls_host: Option<Vec<u8>>,
}

impl Default for ClientSocket {
    fn default() -> Self {
        ClientSocket {
            conn: None,
            host: Host::none(),
            read_timeout: 30000,
            write_timeout: 5000,
            stat: Arc::new(Stat::default()),
            closer: Closer::default(),
            tls_host: None,
        }
    }
}

fn dur(ms: u32) -> Option<Duration> {
    if ms == 0 {
        None
    } else {
        Some(Duration::from_millis(ms as u64))
    }
}

impl ClientSocket {
    pub fn new() -> ClientSocket {
        ClientSocket::default()
    }

    /// TLS で接続するソケット (`SslClientSocket`)。`hostname` が空でなければ証明書を検証する。
    pub fn new_tls(hostname: &[u8]) -> ClientSocket {
        let mut c = ClientSocket::default();
        c.tls_host = Some(hostname.to_vec());
        c
    }

    fn from_tcp(s: TcpStream, host: Host) -> ClientSocket {
        let mut c = ClientSocket::default();
        c.closer.set(s.try_clone().ok());
        c.conn = Some(Conn::Tcp(s));
        c.host = host;
        c
    }

    /// `open` + `connect`
    pub fn connect(&mut self, host: Host) -> Result<()> {
        self.host = host;
        let addr = host.to_socket_addr();
        let s = match dur(self.write_timeout) {
            Some(d) => TcpStream::connect_timeout(&addr, d),
            None => TcpStream::connect(addr),
        };
        let s = match s {
            Ok(s) => s,
            Err(e) => {
                return Err(match e.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Error::timeout(),
                    _ => {
                        if self.tls_host.is_some() {
                            Error::sock(format!("Can't connect: {}", strerror(&e)))
                        } else {
                            Error::sock(strerror(&e))
                        }
                    }
                })
            }
        };
        let _ = s.set_read_timeout(dur(self.read_timeout));
        let _ = s.set_write_timeout(dur(self.write_timeout));
        self.closer.set(s.try_clone().ok());
        match &self.tls_host {
            #[cfg(unix)]
            Some(h) => {
                use std::os::unix::io::AsRawFd;
                let session = super::tls::Session::connect(s.as_raw_fd(), h)?;
                self.conn = Some(Conn::Tls(s, session));
            }
            #[cfg(not(unix))]
            Some(_) => return Err(Error::sock("SSL is not supported")),
            None => self.conn = Some(Conn::Tcp(s)),
        }
        Ok(())
    }

    /// `active`
    pub fn active(&self) -> bool {
        self.conn.is_some()
    }

    pub fn closer(&self) -> Closer {
        self.closer.clone()
    }

    /// ほかのスレッドから読み書きの量を見るためのもの
    pub fn shared_stat(&self) -> Arc<Stat> {
        self.stat.clone()
    }

    /// `readTimeout`
    pub fn read_timeout(&self) -> u32 {
        self.read_timeout
    }

    pub fn is_tls(&self) -> bool {
        #[cfg(unix)]
        {
            matches!(self.conn, Some(Conn::Tls(..)))
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// `getLocalHost` (ポートは 0)
    pub fn local_host(&self) -> Result<Host> {
        let c = self.conn.as_ref().ok_or_else(|| Error::sock("getsockname failed"))?;
        let a = c.tcp().local_addr().map_err(|_| Error::sock("getsockname failed"))?;
        Ok(Host::new(Ip::from_std(a.ip()), 0))
    }

    /// `setNagle`
    pub fn set_nagle(&mut self, on: bool) -> Result<()> {
        match &self.conn {
            Some(c) => c.tcp().set_nodelay(!on).map_err(|_| Error::sock("Unable to set NODELAY")),
            None => Ok(()),
        }
    }

    /// `peekChar`: 1 バイト先を見る (読まない)
    pub fn peek_char(&mut self) -> Result<u8> {
        let c = self.conn.as_ref().ok_or_else(|| Error::stream("recv MSG_PEEK failed. ret = -1"))?;
        let mut b = [0u8; 1];
        match c.tcp().peek(&mut b) {
            Ok(1) => Ok(b[0]),
            Ok(n) => Err(Error::stream(format!("recv MSG_PEEK failed. ret = {}", n))),
            Err(e) => Err(e.into()),
        }
    }

    /// `SslClientSocket::upgrade`: 受け付けた接続を、サーバーとして TLS にする
    #[cfg(unix)]
    pub fn upgrade_tls(mut self) -> Result<ClientSocket> {
        use std::os::unix::io::AsRawFd;
        let s = match self.conn.take() {
            Some(Conn::Tcp(s)) => s,
            _ => return Err(Error::stream("upgrade: not a TCP socket")),
        };
        let session = super::tls::Session::accept(s.as_raw_fd())?;
        let _ = s.set_read_timeout(dur(self.read_timeout));
        let _ = s.set_write_timeout(dur(self.write_timeout));
        self.conn = Some(Conn::Tls(s, session));
        Ok(self)
    }

    fn count_in(&self, n: usize) {
        stats::add(S::BytesIn, n as u32);
        if self.host.local_ip() {
            stats::add(S::LocalBytesIn, n as u32);
        }
        self.stat.update(n as u32, 0);
    }

    fn count_out(&self, n: usize) {
        stats::add(S::BytesOut, n as u32);
        if self.host.local_ip() {
            stats::add(S::LocalBytesOut, n as u32);
        }
        self.stat.update(0, n as u32);
    }

    /// 1 回読む。相手が閉じたら 0
    fn read_once(&mut self, buf: &mut [u8]) -> Result<usize> {
        let c = self.conn.as_mut().ok_or_else(|| Error::sock("Closed on read"))?;
        match c {
            Conn::Tcp(s) => loop {
                match s.read(buf) {
                    Ok(n) => return Ok(n),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
            },
            #[cfg(unix)]
            Conn::Tls(_, t) => t.read_once(buf),
        }
    }
}

impl Stream for ClientSocket {
    /// 要求した長さを全部読む。相手が閉じたら `EOFException`
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < buf.len() {
            let r = self.read_once(&mut buf[done..])?;
            if r == 0 {
                return Err(Error::eof("Closed on read"));
            }
            self.count_in(r);
            done += r;
        }
        Ok(done)
    }

    fn read_upto(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.is_tls() {
            // SslClientSocket::readUpto は 1 回だけ読む
            let r = self.read_once(buf)?;
            return Ok(r);
        }
        let mut done = 0;
        while done < buf.len() {
            let r = self.read_once(&mut buf[done..])?;
            if r == 0 {
                break;
            }
            self.count_in(r);
            done += r;
        }
        Ok(done)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        let c = self.conn.as_mut().ok_or_else(|| Error::sock("Closed on write"))?;
        match c {
            Conn::Tcp(s) => {
                let mut done = 0;
                while done < data.len() {
                    match s.write(&data[done..]) {
                        Ok(0) => return Err(Error::sock("Closed on write")),
                        Ok(n) => done += n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            #[cfg(unix)]
            Conn::Tls(_, t) => t.write(data)?,
        }
        self.count_out(data.len());
        Ok(())
    }

    /// `ClientSocket::eof`: 閉じていれば真
    fn eof(&mut self) -> Result<bool> {
        Ok(!self.active())
    }

    fn close(&mut self) {
        if let Some(c) = self.conn.take() {
            let _ = c.tcp().shutdown(Shutdown::Write);
        }
        self.closer.set(None);
    }

    fn set_read_timeout(&mut self, ms: u32) {
        self.read_timeout = ms;
        if let Some(c) = &self.conn {
            let _ = c.tcp().set_read_timeout(dur(ms));
        }
    }

    fn set_write_timeout(&mut self, ms: u32) {
        self.write_timeout = ms;
        if let Some(c) = &self.conn {
            let _ = c.tcp().set_write_timeout(dur(ms));
        }
    }

    /// `readReady`: `ms` ミリ秒以内に読めるようになるか (相手が閉じたときも真)
    fn read_ready(&mut self, ms: u32) -> bool {
        let c = match &self.conn {
            Some(c) => c,
            None => return false,
        };
        let s = c.tcp();
        let mut b = [0u8; 1];
        let r = if ms == 0 {
            let _ = s.set_nonblocking(true);
            let r = s.peek(&mut b);
            let _ = s.set_nonblocking(false);
            r
        } else {
            let _ = s.set_read_timeout(Some(Duration::from_millis(ms as u64)));
            let r = s.peek(&mut b);
            let _ = s.set_read_timeout(dur(self.read_timeout));
            r
        };
        r.is_ok()
    }

    /// `numPending`: 読まずに取れるバイト数
    fn num_pending(&mut self) -> Result<usize> {
        let c = self.conn.as_ref().ok_or_else(|| Error::stream("numPending"))?;
        let s = c.tcp();
        let mut b = vec![0u8; 65536];
        let _ = s.set_nonblocking(true);
        let r = s.peek(&mut b);
        let _ = s.set_nonblocking(false);
        match r {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
            Err(_) => Err(Error::stream("numPending")),
        }
    }

    fn stat(&self) -> Option<&Stat> {
        Some(&self.stat)
    }
}

impl Drop for ClientSocket {
    fn drop(&mut self) {
        self.close();
    }
}

/// 待ち受けのソケット (`UClientSocket::bind` と `accept`)
pub struct ServerSocket {
    listener: TcpListener,
    pub host: Host,
}

impl ServerSocket {
    /// `bind`: すべてのアドレスの `port` で待ち受ける (IPv6 と IPv4 の両方)
    pub fn bind(host: Host) -> Result<ServerSocket> {
        let addr: SocketAddr = format!("[::]:{}", host.port).parse().map_err(|_| Error::sock("Can`t bind socket"))?;
        let listener = TcpListener::bind(addr)
            .or_else(|_| TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], host.port))))
            .map_err(|_| Error::sock("Can`t bind socket"))?;
        listener.set_nonblocking(true).map_err(|_| Error::sock("Unable to set NONBLOCK"))?;
        Ok(ServerSocket { listener, host })
    }

    /// `accept`: 待っている接続があれば返す (なければ `None`。待たない)
    pub fn accept(&self) -> Option<ClientSocket> {
        match self.listener.accept() {
            Ok((s, addr)) => {
                let _ = s.set_nonblocking(false);
                let host = Host::from_socket_addr(addr);
                let mut cs = ClientSocket::from_tcp(s, host);
                let (r, w) = (cs.read_timeout, cs.write_timeout);
                cs.set_read_timeout(r);
                cs.set_write_timeout(w);
                Some(cs)
            }
            Err(_) => None,
        }
    }

    /// 接続が来るか `ms` ミリ秒たつまで待つ
    pub fn wait(&self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms.min(100) as u64));
    }
}

#[cfg(test)]
mod tests {
    use super::super::stream::StreamExt;
    use super::*;

    #[test]
    fn loopback() {
        let server = ServerSocket::bind(Host::v4(0x7f000001, 0)).unwrap();
        let port = server.listener.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let mut c = ClientSocket::new();
            c.connect(Host::v4(0x7f000001, port)).unwrap();
            c.write_line("hello").unwrap();
            let mut b = [0u8; 2];
            c.read(&mut b).unwrap();
            b
        });
        let mut s = loop {
            if let Some(s) = server.accept() {
                break s;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(s.host.ip.is_ipv4_loopback() || s.host.ip.is_ipv6_loopback());
        assert_eq!(s.read_line(100).unwrap(), b"hello");
        s.write(b"ok").unwrap();
        assert_eq!(&t.join().unwrap(), b"ok");
        // 相手が閉じたら EOF
        let mut b = [0u8; 1];
        let e = s.read(&mut b).unwrap_err();
        assert!(e.is_eof());
    }

    #[test]
    fn refused_and_timeout() {
        let server = ServerSocket::bind(Host::v4(0x7f000001, 0)).unwrap();
        let port = server.listener.local_addr().unwrap().port();
        let mut c = ClientSocket::new();
        c.connect(Host::v4(0x7f000001, port)).unwrap();
        c.set_read_timeout(100);
        let mut b = [0u8; 1];
        assert!(c.read(&mut b).unwrap_err().is_timeout());
        assert!(!c.read_ready(10));
        drop(server);
        let mut c = ClientSocket::new();
        assert!(c.connect(Host::v4(0x7f000001, 1)).unwrap_err().is_sock());
    }
}
