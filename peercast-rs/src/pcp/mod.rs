//! PCP の受け取ったパケットの処理 (core/common/pcp.cpp の `PCPStream::procAtom` 以下と、
//! chaninfo.cpp の `ChanInfo::readInfoAtoms` / `readTrackAtoms`)。
//!
//! C++ 版と同じく、パケットは 16KB のバッファ (`ChanPacket::data`) の上で処理する。`helo` への返事
//! (`oleh`) をそのバッファに書き込むこと、中継するパケットを別のバッファに組み立てることも同じ。
//! チャンネル、ホスト、サーバーの状態 (chanMgr、servMgr、Channel) は C++ 側にあり、`Host` の
//! コールバックで読み書きする。コールバックは C++ 版と同じ順序で呼ぶ。

pub mod atom;
pub mod handshake;
pub mod write;
#[cfg(test)]
mod tests;

pub use atom::{AtomIo, AtomStream, Id4, Ip, MemStream};

/// パケットのバッファの上の AtomStream
type MemAtom<'a> = AtomStream<MemStream<'a>>;
use atom::{id4, id_str};

/// 処理の中断
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// コールバックの中で C++ の例外が起きた (C++ 側で投げ直す)
    Abort,
    /// `StreamException`
    Stream(&'static str),
}

impl From<crate::reader::Abort> for Error {
    fn from(_: crate::reader::Abort) -> Self {
        Error::Abort
    }
}

pub type Result<T> = std::result::Result<T, Error>;
use crate::reader::Abort;

/// `ChanPacket::MAX_DATALEN` (パケットのバッファの大きさ)
pub const MAX_DATALEN: usize = 16384;

pub const PCP_OK: Id4 = id4(b"ok");
pub const PCP_HELO: Id4 = id4(b"helo");
pub const PCP_HELO_SESSIONID: Id4 = id4(b"sid");
pub const PCP_OLEH: Id4 = id4(b"oleh");
pub const PCP_ROOT: Id4 = id4(b"root");
pub const PCP_ROOT_UPDINT: Id4 = id4(b"uint");
pub const PCP_ROOT_CHECKVER: Id4 = id4(b"chkv");
pub const PCP_ROOT_URL: Id4 = id4(b"url");
pub const PCP_ROOT_UPDATE: Id4 = id4(b"upd");
pub const PCP_ROOT_NEXT: Id4 = id4(b"next");
pub const PCP_HOST: Id4 = id4(b"host");
pub const PCP_HOST_ID: Id4 = id4(b"id");
pub const PCP_HOST_IP: Id4 = id4(b"ip");
pub const PCP_HOST_PORT: Id4 = id4(b"port");
pub const PCP_HOST_NUML: Id4 = id4(b"numl");
pub const PCP_HOST_NUMR: Id4 = id4(b"numr");
pub const PCP_HOST_UPTIME: Id4 = id4(b"uptm");
pub const PCP_HOST_CHANID: Id4 = id4(b"cid");
pub const PCP_HOST_VERSION: Id4 = id4(b"ver");
pub const PCP_HOST_VERSION_VP: Id4 = id4(b"vevp");
pub const PCP_HOST_VERSION_EX_PREFIX: Id4 = id4(b"vexp");
pub const PCP_HOST_VERSION_EX_NUMBER: Id4 = id4(b"vexn");
pub const PCP_HOST_FLAGS1: Id4 = id4(b"flg1");
pub const PCP_HOST_OLDPOS: Id4 = id4(b"oldp");
pub const PCP_HOST_NEWPOS: Id4 = id4(b"newp");
pub const PCP_HOST_UPHOST_IP: Id4 = id4(b"upip");
pub const PCP_HOST_UPHOST_PORT: Id4 = id4(b"uppt");
pub const PCP_HOST_UPHOST_HOPS: Id4 = id4(b"uphp");
pub const PCP_QUIT: Id4 = id4(b"quit");
pub const PCP_CHAN: Id4 = id4(b"chan");
pub const PCP_CHAN_ID: Id4 = id4(b"id");
pub const PCP_CHAN_BCID: Id4 = id4(b"bcid");
pub const PCP_CHAN_KEY: Id4 = id4(b"key");
pub const PCP_CHAN_PKT: Id4 = id4(b"pkt");
pub const PCP_CHAN_PKT_TYPE: Id4 = id4(b"type");
pub const PCP_CHAN_PKT_POS: Id4 = id4(b"pos");
pub const PCP_CHAN_PKT_HEAD: Id4 = id4(b"head");
pub const PCP_CHAN_PKT_DATA: Id4 = id4(b"data");
pub const PCP_CHAN_PKT_CONTINUATION: Id4 = id4(b"cont");
pub const PCP_CHAN_INFO: Id4 = id4(b"info");
pub const PCP_CHAN_INFO_TYPE: Id4 = id4(b"type");
pub const PCP_CHAN_INFO_STREAMTYPE: Id4 = id4(b"styp");
pub const PCP_CHAN_INFO_STREAMEXT: Id4 = id4(b"sext");
pub const PCP_CHAN_INFO_BITRATE: Id4 = id4(b"bitr");
pub const PCP_CHAN_INFO_GENRE: Id4 = id4(b"gnre");
pub const PCP_CHAN_INFO_NAME: Id4 = id4(b"name");
pub const PCP_CHAN_INFO_URL: Id4 = id4(b"url");
pub const PCP_CHAN_INFO_DESC: Id4 = id4(b"desc");
pub const PCP_CHAN_INFO_COMMENT: Id4 = id4(b"cmnt");
pub const PCP_CHAN_TRACK: Id4 = id4(b"trck");
pub const PCP_CHAN_TRACK_TITLE: Id4 = id4(b"titl");
pub const PCP_CHAN_TRACK_CREATOR: Id4 = id4(b"crea");
pub const PCP_CHAN_TRACK_URL: Id4 = id4(b"url");
pub const PCP_CHAN_TRACK_ALBUM: Id4 = id4(b"albm");
pub const PCP_MESG: Id4 = id4(b"mesg");
pub const PCP_MESG_ASCII: Id4 = id4(b"asci");
pub const PCP_BCST: Id4 = id4(b"bcst");
pub const PCP_BCST_TTL: Id4 = id4(b"ttl");
pub const PCP_BCST_HOPS: Id4 = id4(b"hops");
pub const PCP_BCST_FROM: Id4 = id4(b"from");
pub const PCP_BCST_DEST: Id4 = id4(b"dest");
pub const PCP_BCST_GROUP: Id4 = id4(b"grp");
pub const PCP_BCST_CHANID: Id4 = id4(b"cid");
pub const PCP_BCST_VERSION: Id4 = id4(b"vers");
pub const PCP_BCST_VERSION_VP: Id4 = id4(b"vrvp");
pub const PCP_BCST_VERSION_EX_PREFIX: Id4 = id4(b"vexp");
pub const PCP_BCST_VERSION_EX_NUMBER: Id4 = id4(b"vexn");
pub const PCP_PUSH: Id4 = id4(b"push");
pub const PCP_PUSH_IP: Id4 = id4(b"ip");
pub const PCP_PUSH_PORT: Id4 = id4(b"port");
pub const PCP_PUSH_CHANID: Id4 = id4(b"cid");
pub const PCP_ATOM: Id4 = id4(b"atom");

pub const PCP_BCST_GROUP_ROOT: i32 = 1;
pub const PCP_BCST_GROUP_TRACKERS: i32 = 2;
pub const PCP_BCST_GROUP_RELAYS: i32 = 4;

pub const PCP_ERROR_QUIT: i32 = 1000;
pub const PCP_ERROR_BCST: i32 = 2000;
pub const PCP_ERROR_LOOPBACK: i32 = 4;

pub const PCP_HOST_FLAGS1_RECV: i32 = 0x10;

pub use crate::version::{PCP_CLIENT_VERSION, PCP_CLIENT_VERSION_EX_NUMBER, PCP_CLIENT_VERSION_EX_PREFIX, PCP_CLIENT_VERSION_VP};

/// `String::MAX_LEN` (C++ の `String` のバッファの大きさ)
pub const STRING_MAX: i32 = 256;

/// `procAtom` の入れ子 (`atom` の子、`bcst` の中の atom) の上限。C++ 版には上限がなく、`bcst` の
/// 入れ子ごとに 16KB のバッファをスタックに取るので、数百段の入れ子でスタックを使い果たして
/// 落ちていた (ネットワークから届くパケットで起きる)。
pub const MAX_PROC_DEPTH: i32 = 64;

/// `BroadcastState`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BroadcastState {
    pub chan_id: [u8; 16],
    pub bc_id: [u8; 16],
    pub num_hops: i32,
    pub for_me: bool,
    pub stream_pos: u32,
    pub group: i32,
}

/// ログの重さ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Error = 2,
}

/// `ChanInfo` の文字列のメンバー (`readInfoAtoms` / `readTrackAtoms` が書くもの)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoField {
    Name = 0,
    Genre = 1,
    Url = 2,
    Desc = 3,
    Comment = 4,
    ContentType = 5,
    MimeType = 6,
    StreamExt = 7,
    TrackTitle = 8,
    TrackCreator = 9,
    TrackUrl = 10,
    TrackAlbum = 11,
}

/// `readHostAtoms` が読んだ `ChanHit` の値。`None` は、`ChanHit::init()` の値のまま。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    pub rhost_ip: [Option<Ip>; 2],
    /// `readShort` の値 (short を int に広げたもの)
    pub rhost_port: [Option<i32>; 2],
    pub num_listeners: Option<i32>,
    pub num_relays: Option<i32>,
    pub up_time: Option<i32>,
    pub oldest_pos: Option<i32>,
    pub newest_pos: Option<i32>,
    pub version: Option<i32>,
    pub version_vp: Option<i32>,
    pub version_ex_prefix: Option<[u8; 2]>,
    pub version_ex_number: Option<i32>,
    /// `flg1` の値 (C++ の `readChar`)。recv、relay、direct、cin、tracker、firewalled を決める
    pub flags1: Option<i32>,
    pub session_id: Option<[u8; 16]>,
    pub uphost_ip: Option<Ip>,
    pub uphost_port: Option<i32>,
    pub uphost_hops: Option<i32>,
    pub chan_id: [u8; 16],
    pub num_hops: i32,
}

/// `readPktAtoms` が読んだパケット
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    /// `ChanPacket::TYPE` (T_UNKNOWN 0、T_HEAD 1、T_DATA 2)
    pub kind: i32,
    pub pos: u32,
    pub cont: bool,
    pub data: Vec<u8>,
}

pub const T_UNKNOWN: i32 = 0;
pub const T_HEAD: i32 = 1;
pub const T_DATA: i32 = 2;

/// 中継するパケットの送り先 (`readBroadcastAtoms`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// `chanMgr->broadcastPacketUp`
    Up = 0,
    /// `servMgr->broadcastPacket(..., Servent::T_COUT)`
    Cout = 1,
    /// 同 `T_CIN`
    Cin = 2,
    /// 同 `T_RELAY`
    Relay = 3,
}

/// PCP の処理から見た C++ 側。`Result` を返すものは、C++ の例外が起きたら `Abort`。
pub trait Host {
    fn session_id(&mut self) -> [u8; 16];
    fn is_root(&mut self) -> bool;
    fn time(&mut self) -> u32;
    fn log(&mut self, level: Level, msg: &[u8]);
    /// `routeList.add(fromID)`
    fn route_add(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort>;
    /// `chanMgr->setUpdateInterval`
    fn set_update_interval(&mut self, si: i32) -> std::result::Result<(), Abort>;
    /// 新しい版がある: `servMgr->downloadURL` に `url` を入れて `NT_UPGRADE` を通知
    fn upgrade(&mut self, url: &[u8]) -> std::result::Result<(), Abort>;
    /// `chanMgr->broadcastTrackerUpdate(remoteID, true)`
    fn tracker_update(&mut self) -> std::result::Result<(), Abort>;
    /// ルートからのメッセージ (`servMgr->rootMsg` と違えば入れ替えて通知)
    fn root_message(&mut self, msg: &[u8]) -> std::result::Result<(), Abort>;
    /// `add` なら `chanMgr->addHit`、でなければ `delHit`
    fn hit(&mut self, hit: &Hit, add: bool) -> std::result::Result<(), Abort>;
    /// 自分宛ての `push` (GIV の接続を始める)
    fn push(&mut self, ip: Option<Ip>, port: Option<i32>, chan_id: &[u8; 16]) -> std::result::Result<(), Abort>;
    /// `readChanAtoms` の始め: `bcs.chanID` のチャンネルとヒットリストを探し、newInfo を用意する
    fn chan_begin(&mut self, chan_id: &[u8; 16]) -> std::result::Result<(), Abort>;
    /// そのチャンネルがあるか (`ch` が NULL でないか)
    fn chan_has_channel(&mut self) -> bool;
    /// `readPktAtoms` のチャンネル側の処理 (rawData への書き込みなど)
    fn chan_packet(&mut self, pkt: &Packet) -> std::result::Result<(), Abort>;
    /// newInfo の文字列のメンバーに、`readString` のとおり `bytes` を写す (URL は http(s) だけ残す)
    fn chan_info_string(&mut self, field: InfoField, bytes: &[u8]) -> std::result::Result<(), Abort>;
    fn chan_info_bitrate(&mut self, bitrate: i32) -> std::result::Result<(), Abort>;
    fn chan_bcid(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort>;
    /// newInfo.id を変え、チャンネルとヒットリストを探し直す
    fn chan_id(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort>;
    /// `readChanAtoms` の終わり (ヒットリストの更新、チャンネルのログ、updateInfo)
    fn chan_end(&mut self) -> std::result::Result<(), Abort>;
    fn broadcast(&mut self, target: Target, pack: &[u8], chan_id: &[u8; 16], dest_id: &[u8; 16]) -> std::result::Result<(), Abort>;
}

/// PCPStream の状態のうち、パケットの処理で読み書きするもの
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub bcs: BroadcastState,
    pub next_root_packet: u32,
}

/// `readPacket` の、受け取ったパケットを処理する部分 (`mem.rewind()` から `procAtom` まで)。
/// `buf` はパケットのバッファ全体 (`ChanPacket::data`)。返り値は `procAtom` の値。
pub fn proc_packet(host: &mut dyn Host, buf: &mut [u8], state: &mut State) -> Result<i32> {
    let mut atom = AtomStream::new(MemStream::new(buf));
    let (id, numc, numd) = atom.read()?;
    Pcp { host, state }.proc_atom(&mut atom, id, numc, numd, 0)
}

struct Pcp<'h, 's> {
    host: &'h mut dyn Host,
    state: &'s mut State,
}

/// C++ の `String` のバッファに `readString` で書いたときの文字列 (NUL の手前まで)。まだ書いて
/// いない部分は、C++ 版では初期化されていない値だった。Rust 版は 0 とみなす。
pub(crate) fn c_string(bytes: &[u8], max: usize) -> Vec<u8> {
    let bytes = &bytes[..bytes.len().min(max - 1)];
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..n].to_vec()
}

fn gnuid_str(id: &[u8; 16]) -> Vec<u8> {
    crate::gnuid::to_str(id).to_vec()
}

fn is_set(id: &[u8; 16]) -> bool {
    id.iter().any(|&b| b != 0)
}

impl Pcp<'_, '_> {
    fn log(&mut self, level: Level, parts: &[&[u8]]) {
        self.host.log(level, &parts.concat());
    }

    fn log_skip(&mut self, id: &Id4, c: i32, d: i32) {
        let msg = format!(", {}, {}", c, d);
        self.log(Level::Debug, &[b"PCP skip: ", id_str(id), msg.as_bytes()]);
    }

    fn proc_atom(&mut self, atom: &mut MemAtom, id: Id4, numc: i32, dlen: i32, depth: i32) -> Result<i32> {
        if depth > MAX_PROC_DEPTH {
            return Err(Error::Stream("PCP: atom nesting too deep"));
        }
        let mut r = 0;
        if id == PCP_CHAN {
            self.read_chan_atoms(atom, numc)?;
        } else if id == PCP_ROOT {
            if self.host.is_root() {
                return Err(Error::Stream("Unauthorized root message"));
            }
            self.read_root_atoms(atom, numc)?;
        } else if id == PCP_HOST {
            self.read_host_atoms(atom, numc)?;
        } else if id == PCP_MESG_ASCII || id == PCP_MESG {
            let msg = atom.read_string(STRING_MAX, dlen)?;
            let msg = c_string(&msg, STRING_MAX as usize);
            self.log(Level::Debug, &[b"PCP got text: ", &msg]);
        } else if id == PCP_BCST {
            r = self.read_broadcast_atoms(atom, numc, depth)?;
        } else if id == PCP_HELO {
            atom.skip(numc, dlen)?;
            // 返事の oleh を、受け取ったパケットのバッファに書く (送られはしない。C++ 版のとおり)
            atom.write_parent(PCP_OLEH, 1)?;
            let sid = self.host.session_id();
            atom.write_bytes(PCP_HELO_SESSIONID, &sid)?;
        } else if id == PCP_PUSH {
            self.read_push_atoms(atom, numc)?;
        } else if id == PCP_OK {
            atom.read_int()?;
        } else if id == PCP_QUIT {
            r = atom.read_int()?;
            if r == 0 {
                r = PCP_ERROR_QUIT;
            }
        } else if id == PCP_ATOM {
            for _ in 0..numc {
                let (aid, nc, nd) = atom.read()?;
                let ar = self.proc_atom(atom, aid, nc, nd, depth + 1)?;
                if ar != 0 {
                    r = ar;
                }
            }
        } else {
            self.log(Level::Error, &[b"PCP unknown or misplaced atom: ", id_str(&id)]);
            return Err(Error::Stream("Protocol error"));
        }
        Ok(r)
    }

    fn read_push_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        let mut ip = None;
        let mut port = None;
        let mut chan_id = [0u8; 16];
        for _ in 0..numc {
            // バッファの終わりに着いたら、残りはどれも ID 0 の atom を読み飛ばすだけ (ログは 1 回)
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            if id == PCP_PUSH_IP {
                ip = Some(atom.read_address()?);
            } else if id == PCP_PUSH_PORT {
                port = Some(atom.read_short()?);
            } else if id == PCP_PUSH_CHANID {
                chan_id = atom.read_bytes16()?;
            } else {
                self.log_skip(&id, c, d);
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }
        if self.state.bcs.for_me {
            self.host.push(ip, port, &chan_id)?;
        }
        Ok(())
    }

    fn read_root_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        let mut url: Vec<u8> = Vec::new();
        for _ in 0..numc {
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            if id == PCP_ROOT_UPDINT {
                let si = atom.read_int()?;
                self.host.set_update_interval(si)?;
                self.log(Level::Debug, &[format!("PCP got new host update interval: {}s", si).as_bytes()]);
            } else if id == PCP_ROOT_URL {
                url = b"http://www.peercast.org/".to_vec();
                let loc = atom.read_string(STRING_MAX, d)?;
                let loc = c_string(&loc, STRING_MAX as usize);
                // String::append: 合わせて MAX_LEN - 1 未満のときだけ付け足す
                if loc.len() + url.len() < (STRING_MAX - 1) as usize {
                    url.extend_from_slice(&loc);
                }
            } else if id == PCP_ROOT_CHECKVER {
                let new_ver = atom.read_int()? as u32;
                if new_ver > PCP_CLIENT_VERSION {
                    self.host.upgrade(&url)?;
                }
                self.log(Level::Debug, &[format!("PCP got version check: {} / {}", new_ver as i32, PCP_CLIENT_VERSION).as_bytes()]);
            } else if id == PCP_ROOT_NEXT {
                let time = atom.read_int()? as u32;
                if time != 0 {
                    let ctime = self.host.time();
                    self.state.next_root_packet = ctime.wrapping_add(time);
                    self.log(Level::Debug, &[format!("PCP expecting next root packet in {}s", time).as_bytes()]);
                } else {
                    self.state.next_root_packet = 0;
                }
            } else if id == PCP_ROOT_UPDATE {
                atom.skip(c, d)?;
                self.host.tracker_update()?;
            } else if id == PCP_MESG_ASCII || id == PCP_MESG {
                let msg = atom.read_string(STRING_MAX, d)?;
                self.host.root_message(&c_string(&msg, STRING_MAX as usize))?;
            } else {
                self.log_skip(&id, c, d);
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }
        Ok(())
    }

    fn read_pkt_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        let mut pack = Packet::default();
        for _ in 0..numc {
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            if id == PCP_CHAN_PKT_TYPE {
                let t = atom.read_id4()?;
                pack.kind = if t == PCP_CHAN_PKT_HEAD {
                    T_HEAD
                } else if t == PCP_CHAN_PKT_DATA {
                    T_DATA
                } else {
                    T_UNKNOWN
                };
            } else if id == PCP_CHAN_PKT_POS {
                pack.pos = atom.read_int()? as u32;
            } else if id == PCP_CHAN_PKT_CONTINUATION {
                pack.cont = atom.read_char()? != 0;
            } else if id == PCP_CHAN_PKT_DATA {
                if d > MAX_DATALEN as i32 {
                    return Err(Error::Stream("Data size too large"));
                }
                pack.data = atom.read_bytes(d)?;
            } else {
                self.log_skip(&id, c, d);
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }

        // readChanAtoms はチャンネルがあるときだけ呼ぶ
        self.host.chan_packet(&pack)?;

        let bcs = &mut self.state.bcs;
        if pack.pos != 0 && (bcs.stream_pos == 0 || pack.pos < bcs.stream_pos) {
            bcs.stream_pos = pack.pos;
        }
        Ok(())
    }

    fn read_host_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        let mut hit = Hit { chan_id: self.state.bcs.chan_id, ..Default::default() };
        let mut ip_num = 0usize;
        for _ in 0..numc {
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            if id == PCP_HOST_IP {
                hit.rhost_ip[ip_num] = Some(atom.read_address()?);
            } else if id == PCP_HOST_PORT {
                let port = atom.read_short()?;
                hit.rhost_port[ip_num] = Some(port);
                ip_num = 1;
            } else if id == PCP_HOST_NUML {
                hit.num_listeners = Some(atom.read_int()?);
            } else if id == PCP_HOST_NUMR {
                hit.num_relays = Some(atom.read_int()?);
            } else if id == PCP_HOST_UPTIME {
                hit.up_time = Some(atom.read_int()?);
            } else if id == PCP_HOST_OLDPOS {
                hit.oldest_pos = Some(atom.read_int()?);
            } else if id == PCP_HOST_NEWPOS {
                hit.newest_pos = Some(atom.read_int()?);
            } else if id == PCP_HOST_VERSION {
                hit.version = Some(atom.read_int()?);
            } else if id == PCP_HOST_VERSION_VP {
                hit.version_vp = Some(atom.read_int()?);
            } else if id == PCP_HOST_VERSION_EX_PREFIX {
                let v = atom.read_bytes(2)?;
                hit.version_ex_prefix = Some([v[0], v[1]]);
            } else if id == PCP_HOST_VERSION_EX_NUMBER {
                hit.version_ex_number = Some(atom.read_short()?);
            } else if id == PCP_HOST_FLAGS1 {
                hit.flags1 = Some(atom.read_char()?);
            } else if id == PCP_HOST_ID {
                hit.session_id = Some(atom.read_bytes16()?);
            } else if id == PCP_HOST_CHANID {
                hit.chan_id = atom.read_bytes16()?;
            } else if id == PCP_HOST_UPHOST_IP {
                hit.uphost_ip = Some(atom.read_address()?);
            } else if id == PCP_HOST_UPHOST_PORT {
                hit.uphost_port = Some(atom.read_int()?);
            } else if id == PCP_HOST_UPHOST_HOPS {
                hit.uphost_hops = Some(atom.read_int()?);
            } else {
                self.log_skip(&id, c, d);
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }
        hit.num_hops = self.state.bcs.num_hops;
        // ChanHit::init() の recv は true
        let recv = hit.flags1.map_or(true, |f| f & PCP_HOST_FLAGS1_RECV != 0);
        self.host.hit(&hit, recv)?;
        Ok(())
    }

    /// `ChanInfo::readInfoAtoms` / `readTrackAtoms` の文字列
    fn info_string(&mut self, atom: &mut MemAtom, field: InfoField, d: i32) -> Result<()> {
        let bytes = atom.read_string(STRING_MAX, d)?;
        self.host.chan_info_string(field, &bytes)?;
        Ok(())
    }

    fn read_info_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        for _ in 0..numc {
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            let field = if id == PCP_CHAN_INFO_NAME {
                Some(InfoField::Name)
            } else if id == PCP_CHAN_INFO_GENRE {
                Some(InfoField::Genre)
            } else if id == PCP_CHAN_INFO_URL {
                Some(InfoField::Url)
            } else if id == PCP_CHAN_INFO_DESC {
                Some(InfoField::Desc)
            } else if id == PCP_CHAN_INFO_COMMENT {
                Some(InfoField::Comment)
            } else if id == PCP_CHAN_INFO_TYPE {
                Some(InfoField::ContentType)
            } else if id == PCP_CHAN_INFO_STREAMTYPE {
                Some(InfoField::MimeType)
            } else if id == PCP_CHAN_INFO_STREAMEXT {
                Some(InfoField::StreamExt)
            } else {
                None
            };
            if let Some(f) = field {
                self.info_string(atom, f, d)?;
            } else if id == PCP_CHAN_INFO_BITRATE {
                let b = atom.read_int()?;
                self.host.chan_info_bitrate(b)?;
            } else {
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }
        Ok(())
    }

    fn read_track_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        for _ in 0..numc {
            let stuck = atom.io.stuck();
            let (id, c, d) = atom.read()?;
            let field = if id == PCP_CHAN_TRACK_TITLE {
                Some(InfoField::TrackTitle)
            } else if id == PCP_CHAN_TRACK_CREATOR {
                Some(InfoField::TrackCreator)
            } else if id == PCP_CHAN_TRACK_URL {
                Some(InfoField::TrackUrl)
            } else if id == PCP_CHAN_TRACK_ALBUM {
                Some(InfoField::TrackAlbum)
            } else {
                None
            };
            if let Some(f) = field {
                self.info_string(atom, f, d)?;
            } else {
                atom.skip(c, d)?;
            }
            if stuck {
                break;
            }
        }
        Ok(())
    }

    fn read_chan_atoms(&mut self, atom: &mut MemAtom, numc: i32) -> Result<()> {
        let chan_id = self.state.bcs.chan_id;
        self.host.chan_begin(&chan_id)?;
        for _ in 0..numc {
            let (id, c, d) = atom.read()?;
            if id == PCP_CHAN_PKT && self.host.chan_has_channel() {
                self.read_pkt_atoms(atom, c)?;
            } else if id == PCP_CHAN_INFO {
                self.read_info_atoms(atom, c)?;
            } else if id == PCP_CHAN_TRACK {
                self.read_track_atoms(atom, c)?;
            } else if id == PCP_CHAN_BCID {
                let b = atom.read_bytes16()?;
                self.host.chan_bcid(&b)?;
            } else if id == PCP_CHAN_KEY {
                let mut b = atom.read_bytes16()?;
                b[0] = 0; // clear flags
                self.host.chan_bcid(&b)?;
            } else if id == PCP_CHAN_ID {
                let b = atom.read_bytes16()?;
                self.host.chan_id(&b)?;
            } else {
                let msg = format!(", {}, {}", c, d);
                self.log(Level::Error, &[b"PCP unknown or misplaced atom: ", id_str(&id), msg.as_bytes()]);
                return Err(Error::Stream("Protocol error"));
            }
        }
        self.host.chan_end()?;
        Ok(())
    }

    fn read_broadcast_atoms(&mut self, atom: &mut MemAtom, numc: i32, depth: i32) -> Result<i32> {
        let mut ttl = 1i32;
        let mut ver = 0i32;
        let mut ver_ex_prefix = [b'*', b'*'];
        let mut from_id = [0u8; 16];
        let mut dest_id = [0u8; 16];

        {
            let bcs = &mut self.state.bcs;
            bcs.for_me = false;
            bcs.group = 0;
            bcs.num_hops = 0;
            bcs.bc_id = [0; 16];
            bcs.chan_id = [0; 16];
        }

        // 中継するパケットを組み立てるバッファ (C++ 版の ChanPacket::data。まだ書いていない部分は
        // 初期化されていなかった)
        let mut buf = vec![0u8; MAX_DATALEN];
        let mut patom = AtomStream::new(MemStream::new(&mut buf));
        patom.write_parent(PCP_BCST, numc)?;

        for _ in 0..numc {
            let (id, c, d) = atom.read()?;
            if id == PCP_BCST_TTL {
                ttl = atom.read_char()? - 1;
                patom.write_char(id, ttl as u8)?;
            } else if id == PCP_BCST_HOPS {
                self.state.bcs.num_hops = atom.read_char()? + 1;
                patom.write_char(id, self.state.bcs.num_hops as u8)?;
            } else if id == PCP_BCST_FROM {
                from_id = atom.read_bytes16()?;
                patom.write_bytes(id, &from_id)?;
                self.host.route_add(&from_id)?;
            } else if id == PCP_BCST_GROUP {
                self.state.bcs.group = atom.read_char()?;
                patom.write_char(id, self.state.bcs.group as u8)?;
            } else if id == PCP_BCST_DEST {
                dest_id = atom.read_bytes16()?;
                patom.write_bytes(id, &dest_id)?;
                self.state.bcs.for_me = dest_id == self.host.session_id();
            } else if id == PCP_BCST_CHANID {
                self.state.bcs.chan_id = atom.read_bytes16()?;
                let cid = self.state.bcs.chan_id;
                patom.write_bytes(id, &cid)?;
            } else if id == PCP_BCST_VERSION {
                ver = atom.read_int()?;
                patom.write_int(id, ver)?;
                self.log(Level::Debug, &[format!("PCP bcst version {}", ver).as_bytes()]);
            } else if id == PCP_BCST_VERSION_VP {
                let ver_vp = atom.read_int()?;
                patom.write_int(id, ver_vp)?;
                self.log(Level::Debug, &[format!("PCP bcst VP version {}", ver_vp).as_bytes()]);
            } else if id == PCP_BCST_VERSION_EX_PREFIX {
                let v = atom.read_bytes(2)?;
                ver_ex_prefix = [v[0], v[1]];
                patom.write_bytes(id, &ver_ex_prefix)?;
            } else if id == PCP_BCST_VERSION_EX_NUMBER {
                let n = atom.read_short()? as i16;
                patom.write_short(id, n)?;
                let prefix = c_string(&ver_ex_prefix, 3);
                self.log(Level::Debug, &[b"PCP bcst ex version ", &prefix, n.to_string().as_bytes()]);
            } else {
                // 写してから処理する
                let old_pos = patom.io.pos;
                patom.write_atoms(id, &mut atom.io, c, d)?;
                patom.io.pos = old_pos;
                let (aid, nc, nd) = patom.read()?;
                self.proc_atom(&mut patom, aid, nc, nd, depth + 1)?;
            }
        }

        let bcs = self.state.bcs.clone();
        let msg = format!(
            "PCP bcst: group={}, hops={}, ver={}, from=",
            bcs.group, bcs.num_hops, ver
        );
        self.log(
            Level::Debug,
            &[msg.as_bytes(), &gnuid_str(&from_id), b", dest=", &gnuid_str(&dest_id)],
        );
        if is_set(&from_id) && from_id == self.host.session_id() {
            self.log(Level::Error, &[b"BCST loopback"]);
            return Ok(PCP_ERROR_BCST + PCP_ERROR_LOOPBACK);
        }

        // ttl が残っていれば中継する
        if ttl > 0 && !bcs.for_me {
            let len = patom.io.pos;
            let pack = patom.io.data(len).to_vec();
            let g = bcs.group;
            let all = PCP_BCST_GROUP_ROOT | PCP_BCST_GROUP_TRACKERS | PCP_BCST_GROUP_RELAYS;
            if g & all != 0 {
                self.host.broadcast(Target::Up, &pack, &bcs.chan_id, &dest_id)?;
            }
            if g & all != 0 {
                self.host.broadcast(Target::Cout, &pack, &bcs.chan_id, &dest_id)?;
            }
            if g & (PCP_BCST_GROUP_RELAYS | PCP_BCST_GROUP_TRACKERS) != 0 {
                self.host.broadcast(Target::Cin, &pack, &bcs.chan_id, &dest_id)?;
            }
            if g & PCP_BCST_GROUP_RELAYS != 0 {
                self.host.broadcast(Target::Relay, &pack, &bcs.chan_id, &dest_id)?;
            }
        }
        Ok(0)
    }
}
