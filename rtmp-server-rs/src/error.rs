use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// 入力の終端に達した (C++ 版の EOFException 相当)。
    Eof,
    /// プロトコル違反や未対応の入力。
    Protocol(String),
    Io(io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn protocol(msg: impl Into<String>) -> Error {
        Error::Protocol(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eof => write!(f, "end of stream"),
            Error::Protocol(m) => write!(f, "{}", m),
            Error::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Error::Eof
        } else {
            Error::Io(e)
        }
    }
}
