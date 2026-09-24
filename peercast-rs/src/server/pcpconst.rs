//! PCP の atom の ID と定数のうち、段階 6 の `crate::pcp` にないもの (core/common/pcp.h、gnutella.h)。

pub use crate::pcp::atom::{id4, Id4};
pub use crate::pcp::*;

pub const PCP_CONNECT: Id4 = *b"pcp\n";
pub const PCP_HELO_AGENT: Id4 = id4(b"agnt");
pub const PCP_HELO_OSTYPE: Id4 = id4(b"ostp");
pub const PCP_HELO_PORT: Id4 = id4(b"port");
pub const PCP_HELO_PING: Id4 = id4(b"ping");
pub const PCP_HELO_REMOTEIP: Id4 = id4(b"rip");
pub const PCP_HELO_VERSION: Id4 = id4(b"ver");
pub const PCP_HELO_BCID: Id4 = id4(b"bcid");
pub const PCP_HELO_DISABLE: Id4 = id4(b"dis");

pub const PCP_BCST_GROUP_ALL: i32 = -1; // (char)0xff
pub const PCP_ERROR_READ: i32 = 3000;
pub const PCP_ERROR_WRITE: i32 = 4000;
pub const PCP_ERROR_GENERAL: i32 = 5000;
pub const PCP_ERROR_SKIP: i32 = 1;
pub const PCP_ERROR_ALREADYCONNECTED: i32 = 2;
pub const PCP_ERROR_UNAVAILABLE: i32 = 3;
pub const PCP_ERROR_NOTIDENTIFIED: i32 = 5;
pub const PCP_ERROR_BADRESPONSE: i32 = 6;
pub const PCP_ERROR_BADAGENT: i32 = 7;
pub const PCP_ERROR_OFFAIR: i32 = 8;
pub const PCP_ERROR_SHUTDOWN: i32 = 9;
pub const PCP_ERROR_NOROOT: i32 = 10;
pub const PCP_ERROR_BANNED: i32 = 11;

pub const PCP_ROOT_VERSION: i32 = 1218;
pub const PCP_CLIENT_MINVERSION: i32 = 1200;

pub const PCX_PCP_CONNECT: &str = "pcp";
pub const PCX_HS_CHANNELID: &str = "x-peercast-channelid:";
pub const PCX_HS_PCP: &str = "x-peercast-pcp:";
pub const PCX_HS_POS: &str = "x-peercast-pos:";
pub const PCX_OS_LINUX: &str = "Linux";
pub const ICY_OK: &str = "ICY 200 OK";
