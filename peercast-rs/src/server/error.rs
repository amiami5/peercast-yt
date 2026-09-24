//! C++ 版の例外 (common.h の `GeneralException` とその派生クラス) に当たるもの。

use std::fmt;

/// 例外の種類。`Stream` から下は `StreamException` の派生クラス。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    General,
    NotImplemented,
    Logic,
    Argument,
    Format,
    Stream,
    Sock,
    Eof,
    Timeout,
}

/// `GeneralException`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: Kind,
    pub msg: String,
    pub err: i32,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(kind: Kind, msg: impl Into<String>) -> Error {
        Error { kind, msg: msg.into(), err: 0 }
    }

    pub fn general(msg: impl Into<String>) -> Error {
        Error::new(Kind::General, msg)
    }

    pub fn stream(msg: impl Into<String>) -> Error {
        Error::new(Kind::Stream, msg)
    }

    pub fn sock(msg: impl Into<String>) -> Error {
        Error::new(Kind::Sock, msg)
    }

    pub fn eof(msg: impl Into<String>) -> Error {
        Error::new(Kind::Eof, msg)
    }

    /// `TimeoutException()` (文言は "Timeout")
    pub fn timeout() -> Error {
        Error::new(Kind::Timeout, "Timeout")
    }

    pub fn format(msg: impl Into<String>) -> Error {
        Error::new(Kind::Format, msg)
    }

    pub fn argument(msg: impl Into<String>) -> Error {
        Error::new(Kind::Argument, msg)
    }

    pub fn with_err(mut self, err: i32) -> Error {
        self.err = err;
        self
    }

    /// `catch (StreamException&)` に当たるか
    pub fn is_stream(&self) -> bool {
        matches!(self.kind, Kind::Stream | Kind::Sock | Kind::Eof | Kind::Timeout)
    }

    /// `catch (SockException&)`
    pub fn is_sock(&self) -> bool {
        self.kind == Kind::Sock
    }

    pub fn is_timeout(&self) -> bool {
        self.kind == Kind::Timeout
    }

    pub fn is_eof(&self) -> bool {
        self.kind == Kind::Eof
    }

    /// `what()`
    pub fn what(&self) -> &str {
        &self.msg
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

/// 標準ライブラリの入出力の誤りを、C++ 版のソケットの例外にする
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        use std::io::ErrorKind;
        match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => Error::timeout(),
            ErrorKind::UnexpectedEof => Error::eof("Closed on read"),
            _ => Error::sock(strerror(&e)).with_err(e.raw_os_error().unwrap_or(0)),
        }
    }
}

/// `str::strerror` と同じく、OS の誤りの文言 ("Connection refused" など)
pub fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    // 標準ライブラリは "Connection refused (os error 111)" の形にする
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}
