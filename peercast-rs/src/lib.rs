//! PeerCast YT のサーバー (Rust 版)。元は C++ 版 (core/common など) を段階的に移したもの
//! (docs/rust-migration.md)。
//!
//! この下の、ネットワークからの入力を解釈するモジュール (`pcp`、`http`、`media`、`json` など) は、
//! 状態を持たず `unsafe` も使わない。サーバーの状態、スレッド、ソケットは `server` モジュール。
//! OS の機能と OpenSSL・librtmp を C ABI で呼ぶところ (`server::os`、`server::tls`、`server::rtmp`)
//! だけが `unsafe` を使う。文字列は C++ 版に合わせて、UTF-8 とは限らないバイト列 (`&[u8]`) として扱う。
//!
//! 元のプログラムは GPL (v2 以降) なので、この移植も同じ条件で配布する。
#![deny(unsafe_code)]

pub mod amf0;
pub mod bbs;
pub mod chandir;
pub mod channel;
pub mod chanhit;
pub mod chaninfo;
pub mod chanpacket;
pub mod cgi;
pub mod commands;
pub mod dechunk;
pub mod entities;
pub mod gnuid;
pub mod hostgraph;
pub mod http;
pub mod inspect;
pub mod jrpc;
pub mod json;
pub mod jis;
mod jis_table;
pub mod mapper;
pub mod md5;
pub mod media;
pub mod pcp;
pub mod pcstring;
pub mod public;
pub mod reader;
pub mod server;
pub mod servhs;
pub mod strtod;
pub mod strutil;
pub mod template;
pub mod uptest;
pub mod url;
pub mod utf8;
pub mod version;
pub mod xml;
