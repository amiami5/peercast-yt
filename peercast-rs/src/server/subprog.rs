//! 外部のプログラム (core/common/env.cpp の `Environment`、core/unix/usubprog.cpp の `Subprogram`)。

use std::process::{Child, Command, Stdio};

use super::error::{Error, Result};
use super::stream::Stream;

/// `Environment`: `名前=値` の並び
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    pub vars: Vec<Vec<u8>>,
}

impl Environment {
    /// `copyFromCurrentProcess`
    pub fn from_current_process() -> Environment {
        Environment { vars: env_vars() }
    }

    fn prefix(key: &[u8]) -> Vec<u8> {
        let mut p = key.to_vec();
        p.push(b'=');
        p
    }

    /// `set`: あれば置き換える。C++ 版は置き換えるときに `名前=` を付けずに値だけを入れていた
    /// (docs/cpp-known-issues.md)。Rust 版は `名前=値` にする。
    pub fn set(&mut self, key: &[u8], value: &[u8]) {
        let p = Self::prefix(key);
        let mut entry = p.clone();
        entry.extend_from_slice(value);
        for e in self.vars.iter_mut() {
            if e.starts_with(&p) {
                *e = entry;
                return;
            }
        }
        self.vars.push(entry);
    }

    pub fn unset(&mut self, key: &[u8]) {
        let p = Self::prefix(key);
        if let Some(i) = self.vars.iter().position(|e| e.starts_with(&p)) {
            self.vars.remove(i);
        }
    }

    pub fn get(&self, key: &[u8]) -> Vec<u8> {
        let p = Self::prefix(key);
        self.vars.iter().find(|e| e.starts_with(&p)).map_or(Vec::new(), |e| e[p.len()..].to_vec())
    }

    pub fn has_key(&self, key: &[u8]) -> bool {
        let p = Self::prefix(key);
        self.vars.iter().any(|e| e.starts_with(&p))
    }

    pub fn keys(&self) -> Vec<Vec<u8>> {
        self.vars.iter().map(|e| e[..e.iter().position(|&c| c == b'=').unwrap_or(e.len())].to_vec()).collect()
    }
}

#[cfg(unix)]
fn env_vars() -> Vec<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    std::env::vars_os()
        .map(|(k, v)| {
            let mut e = k.as_bytes().to_vec();
            e.push(b'=');
            e.extend_from_slice(v.as_bytes());
            e
        })
        .collect()
}

#[cfg(not(unix))]
fn env_vars() -> Vec<Vec<u8>> {
    std::env::vars().map(|(k, v)| format!("{}={}", k, v).into_bytes()).collect()
}

#[cfg(unix)]
fn os(b: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(b).to_os_string()
}

#[cfg(not(unix))]
fn os(b: &[u8]) -> std::ffi::OsString {
    String::from_utf8_lossy(b).into_owned().into()
}

/// `Subprogram`
pub struct Subprogram {
    name: Vec<u8>,
    receive: bool,
    feed: bool,
    child: Option<Child>,
}

impl Subprogram {
    pub fn new(name: &[u8], receive_data: bool, feed_data: bool) -> Subprogram {
        Subprogram { name: name.to_vec(), receive: receive_data, feed: feed_data, child: None }
    }

    /// `start`: 環境変数は `env` だけにして始める
    pub fn start(&mut self, args: &[Vec<u8>], env: &Environment) -> bool {
        let mut cmd = Command::new(os(&self.name));
        for a in args {
            cmd.arg(os(a));
        }
        cmd.env_clear();
        for e in &env.vars {
            let i = e.iter().position(|&c| c == b'=').unwrap_or(e.len());
            let v = if i < e.len() { &e[i + 1..] } else { &b""[..] };
            cmd.env(os(&e[..i]), os(v));
        }
        cmd.stdout(if self.receive { Stdio::piped() } else { Stdio::inherit() });
        cmd.stdin(if self.feed { Stdio::piped() } else { Stdio::inherit() });
        match cmd.spawn() {
            Ok(c) => {
                self.child = Some(c);
                true
            }
            Err(e) => {
                crate::log_error!("execle: {}: {}", String::from_utf8_lossy(&self.name), super::error::strerror(&e));
                false
            }
        }
    }

    /// プログラムの出力を読む (`inputStream`)
    pub fn input_stream(&mut self) -> Result<PipeReader> {
        let out = self.child.as_mut().and_then(|c| c.stdout.take()).ok_or_else(|| Error::general("no input stream"))?;
        Ok(PipeReader { r: Box::new(out), eof: false })
    }

    /// プログラムの入力に書く (`outputStream`)
    pub fn output_stream(&mut self) -> Result<PipeWriter> {
        let inp = self.child.as_mut().and_then(|c| c.stdin.take()).ok_or_else(|| Error::general("no output stream"))?;
        Ok(PipeWriter { w: Box::new(inp) })
    }

    pub fn pid(&self) -> i32 {
        self.child.as_ref().map_or(-1, |c| c.id() as i32)
    }

    /// `wait`: 終了コード (正常に終わらなければ `None`)
    pub fn wait(&mut self) -> Option<i32> {
        let mut c = self.child.take()?;
        c.wait().ok().and_then(|s| s.code())
    }

    /// `isAlive`
    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            None => false,
            Some(c) => match c.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Err(e) => {
                    crate::log_error!("Failed in checking the status of {}: {}", c.id(), e);
                    false
                }
            },
        }
    }

    /// `terminate`: SIGKILL を送って待つ
    pub fn terminate(&mut self) {
        if let Some(mut c) = self.child.take() {
            if let Err(e) = c.kill() {
                crate::log_error!("Failed in killing {}. {}", c.id(), e);
            }
            let _ = c.wait();
        }
    }
}

impl Drop for Subprogram {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.terminate();
        }
    }
}

/// 子プロセスの出力 (`FileStream` と同じく、読めた分だけ返し、終わりでは例外)
pub struct PipeReader {
    r: Box<dyn std::io::Read + Send>,
    eof: bool,
}

impl Stream for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.eof {
            return Err(Error::stream("End of file"));
        }
        let mut n = 0;
        while n < buf.len() {
            match self.r.read(&mut buf[n..]) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(r) => n += r,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.eof = true;
                    break;
                }
            }
        }
        if n == 0 {
            return Err(Error::stream("End of file"));
        }
        Ok(n)
    }

    fn write(&mut self, _data: &[u8]) -> Result<()> {
        Ok(())
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.eof)
    }
}

/// 子プロセスの入力
pub struct PipeWriter {
    w: Box<dyn std::io::Write + Send>,
}

impl Stream for PipeWriter {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        let _ = self.w.write_all(data);
        Ok(())
    }

    fn close(&mut self) {
        let _ = self.w.flush();
        self.w = Box::new(std::io::sink());
    }
}

#[cfg(test)]
mod tests {
    use super::super::stream::StreamExt;
    use super::*;

    #[test]
    fn env() {
        let mut e = Environment::default();
        e.set(b"A", b"1");
        e.set(b"B", b"2");
        e.set(b"A", b"3");
        assert_eq!(e.get(b"A"), b"3");
        assert_eq!(e.keys(), vec![b"A".to_vec(), b"B".to_vec()]);
        e.unset(b"A");
        assert!(!e.has_key(b"A"));
    }

    #[cfg(unix)]
    #[test]
    fn run() {
        let mut p = Subprogram::new(b"sh", true, false);
        let mut env = Environment::default();
        env.set(b"X", b"hello");
        assert!(p.start(&[b"-c".to_vec(), b"echo $X".to_vec()], &env));
        let mut r = p.input_stream().unwrap();
        assert_eq!(r.read_line(100).unwrap(), b"hello");
        assert_eq!(p.wait(), Some(0));
    }
}
