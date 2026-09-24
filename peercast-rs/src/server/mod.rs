//! Rust だけで動くサーバー (段階 9)。C++ 版の core/common と core/unix の、状態を持つ部分
//! (接続、チャンネル、設定、スレッド、ソケット) を移したもの。
//!
//! 段階 1〜8 で移した解析と判断のモジュール (`crate::pcp`、`crate::servhs` など) を使う。

pub mod cookie;
pub mod error;
pub mod flag;
pub mod host;
pub mod http;
pub mod ini;
pub mod log;
pub mod notif;
#[allow(unsafe_code)]
pub mod os;
pub mod regex;
pub mod servfilter;
pub mod socket;
pub mod state;
pub mod stats;
pub mod stream;
pub mod subprog;
pub mod sys;
#[cfg(unix)]
#[allow(unsafe_code)]
pub mod tls;
