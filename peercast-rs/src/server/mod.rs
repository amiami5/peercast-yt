//! Rust だけで動くサーバー (段階 9)。C++ 版の core/common と core/unix の、状態を持つ部分
//! (接続、チャンネル、設定、スレッド、ソケット) を移したもの。
//!
//! 段階 1〜8 で移した解析と判断のモジュール (`crate::pcp`、`crate::servhs` など) を使う。
//!
//! ロックの順: 一覧 (チャンネル、ヒットリスト、サーバント) のロックを先に取り、要素のロックは後。
//! 要素のロック (`st()` など) を持ったまま、ほかのものを呼ばない。

pub mod app;
pub mod bbs_http;
pub mod chanhit;
pub mod chaninfo;
pub mod chanmgr;
pub mod channel;
pub mod commands;
pub mod cookie;
pub mod directory;
pub mod error;
pub mod flag;
pub mod host;
pub mod html;
pub mod http;
pub mod ini;
pub mod jrpc_host;
pub mod log;
pub mod notif;
#[allow(unsafe_code)]
pub mod os;
pub mod packetbuf;
pub mod pcpconst;
pub mod pcpstream;
pub mod pcstr;
pub mod peercast;
pub mod playlist;
pub mod public;
pub mod regex;
#[cfg(feature = "rtmp")]
#[allow(unsafe_code)]
pub mod rtmp;
pub mod servent;
pub mod servent_http;
pub mod servfilter;
pub mod servmgr;
pub mod socket;
pub mod sources;
pub mod state;
pub mod stats;
pub mod stream;
pub mod subprog;
pub mod sys;
#[cfg(unix)]
#[allow(unsafe_code)]
pub mod tls;
pub mod xmlnode;
