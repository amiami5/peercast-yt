//! PeerCast YT の C++ 実装を段階的に置き換えるための Rust ライブラリ。
//!
//! C++ 側からは `include/peercast_rs.h` の C ABI で呼ぶ。Rust 側の本体 (この下の各モジュール) は
//! `unsafe` を使わず、C とのやりとりは `ffi` モジュールだけに閉じ込めている。
//! 文字列は C++ の `std::string` に合わせて、UTF-8 とは限らないバイト列 (`&[u8]`) として扱う。
//!
//! 元のプログラムは GPL (v2 以降) なので、この移植も同じ条件で配布する。
#![deny(unsafe_code)]

pub mod amf0;
pub mod chandir;
pub mod chanhit;
pub mod chaninfo;
pub mod chanpacket;
pub mod cgi;
pub mod commands;
pub mod dechunk;
pub mod entities;
#[allow(unsafe_code)]
pub mod ffi;
pub mod gnuid;
pub mod http;
pub mod inspect;
pub mod jis;
mod jis_table;
pub mod md5;
pub mod media;
pub mod pcp;
pub mod pcstring;
pub mod public;
pub mod reader;
pub mod strtod;
pub mod strutil;
pub mod template;
pub mod url;
pub mod utf8;
pub mod xml;
