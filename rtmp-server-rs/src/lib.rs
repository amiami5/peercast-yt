//! PeerCast YT 付属の RTMP 受信サーバー (rtmp-server) の Rust 実装。
//!
//! 元の C++ 版 (../rtmp-server) と同じコマンドライン
//! `rtmp-server [-p PORT] URL...` で動き、受信した RTMP の音声・映像を
//! FLV にして URL (http:// の POST、file://、通常のファイルパス) へ書き出す。
//!
//! 元のプログラムは GPL (v2 以降) なので、この移植も同じ条件で配布する。
#![forbid(unsafe_code)]

pub mod amf0;
pub mod error;
pub mod flv;
pub mod session;
pub mod sink;

pub use error::{Error, Result};

/// ログ出力。標準出力が閉じられていても panic しないようにする
/// (println! は書き込みに失敗すると panic する)。
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = writeln!(::std::io::stdout(), $($arg)*);
    }};
}
