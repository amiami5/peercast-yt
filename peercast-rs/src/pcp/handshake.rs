//! PCP のハンドシェイクで受け取る atom の読み取り (servent.cpp の `handshakeIncomingPCP`、
//! `handshakeOutgoingPCP`、`pingHost` の `helo` / `oleh` と、pcp.cpp の `PCPStream::readVersion`)。
//!
//! ソケットなどの C++ の `Stream` から、`Reader` のコールバックで直接読む。返事を書くこと
//! (`oleh`、`quit`) と、読んだ値を使った処理 (servMgr の更新など) は C++ に残る。

use super::atom::{id_str, AtomIo, AtomStream, Id4, Ip};
use super::{c_string, Error, Result, PCP_HELO, PCP_HELO_SESSIONID, PCP_OLEH};
use crate::reader::Reader;

pub const PCP_HELO_AGENT: Id4 = super::atom::id4(b"agnt");
pub const PCP_HELO_OSTYPE: Id4 = super::atom::id4(b"ostp");
pub const PCP_HELO_PORT: Id4 = super::atom::id4(b"port");
pub const PCP_HELO_PING: Id4 = super::atom::id4(b"ping");
pub const PCP_HELO_REMOTEIP: Id4 = super::atom::id4(b"rip");
pub const PCP_HELO_VERSION: Id4 = super::atom::id4(b"ver");
pub const PCP_HELO_BCID: Id4 = super::atom::id4(b"bcid");
pub const PCP_HELO_DISABLE: Id4 = super::atom::id4(b"dis");

/// C++ の `Stream` を `Reader` で読む `AtomIo`。書き込みは C++ 側で行うので使わない。
pub struct StreamIo<R> {
    pub r: R,
}

impl<R: Reader> AtomIo for StreamIo<R> {
    /// `Stream::read(void*, int)`。ソケットは全部読むか例外を投げる。読めなかった分は 0 で埋める
    /// (`MemoryStream` と同じ)。
    fn read(&mut self, l: i64) -> Result<Vec<u8>> {
        if l < 0 {
            return Err(Error::Stream("Stream::read: negative length"));
        }
        let mut v = self.r.read_some(l as usize)?;
        v.resize(l as usize, 0);
        Ok(v)
    }

    fn write(&mut self, _: &[u8]) -> Result<()> {
        Err(Error::Stream("StreamIo: write is not supported"))
    }
}

/// 読む atom の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `handshakeIncomingPCP`: 相手からの `helo`
    Helo = 0,
    /// `handshakeOutgoingPCP`: 相手からの `oleh`
    Oleh = 1,
    /// `pingHost`: 相手からの `oleh` (セッション ID だけ読む)
    Ping = 2,
}

/// 読んだ値。`None` は、その atom がなかった (C++ 側の変数は元のまま)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hello {
    /// 最初の atom の見出しを読み、ID が期待したもの (`helo` か `oleh`) だった
    pub header_ok: bool,
    /// 最初の atom の ID が期待したもの (`helo` か `oleh`) でなかったとき、その ID
    pub unexpected: Option<Id4>,
    /// `agent.set(arg)` の値 (NUL の手前まで)
    pub agent: Option<Vec<u8>>,
    pub version: Option<i32>,
    pub disable: Option<i32>,
    /// `rid` (ping ではその `sid`) に書いたもの
    pub session_id: Option<[u8; 16]>,
    pub bcid: Option<[u8; 16]>,
    pub os_type: Option<i32>,
    /// `readShort` の値 (short を int に広げたもの)
    pub port: Option<i32>,
    pub ping: Option<i32>,
    pub remote_ip: Option<Ip>,
    /// ping で、呼ぶ前の `sid` の値 (長さの足りない atom では頭だけ上書きする)
    pub ping_sid_init: [u8; 16],
}

/// `helo` / `oleh` を読む。エラーのときも、それまでに読んだ値は `out` に入っている (C++ 版では
/// 呼んだ側の変数 `rid` や `agent` に書かれている)。`my_sid` は `servMgr->sessionID`。
pub fn read_hello<IO: AtomIo>(
    atom: &mut AtomStream<IO>,
    kind: Kind,
    my_sid: &[u8; 16],
    log: &mut dyn FnMut(&[u8]),
    out: &mut Hello,
) -> Result<()> {
    let (id, numc, _) = atom.read()?;
    let expected = if kind == Kind::Helo { PCP_HELO } else { PCP_OLEH };
    if id != expected {
        out.unexpected = Some(id);
        return Ok(());
    }
    out.header_ok = true;

    // C++ 版の `char arg[64]` (ループの外にあり、エージェントの atom ごとに頭から上書きする)。
    // 初期化されていなかった部分は 0 とみなす
    let mut arg = [0u8; 64];
    // pingHost の `GnuID sid` (長さの足りない atom では頭だけ上書きする)
    let mut sid = out.ping_sid_init;

    for _ in 0..numc {
        let (id, c, dlen) = atom.read()?;
        if kind == Kind::Ping {
            if id == PCP_HELO_SESSIONID {
                let v = atom.read_bytes_max(16, dlen)?;
                sid[..v.len()].copy_from_slice(&v);
                out.session_id = Some(sid);
            } else {
                atom.skip(c, dlen)?;
            }
            continue;
        }

        if id == PCP_HELO_AGENT {
            let v = atom.read_string(64, dlen)?;
            arg[..v.len()].copy_from_slice(&v);
            arg[63] = 0;
            out.agent = Some(c_string(&arg, 64));
        } else if id == PCP_HELO_VERSION {
            out.version = Some(atom.read_int()?);
        } else if id == PCP_HELO_SESSIONID {
            let v = atom.read_bytes16()?;
            out.session_id = Some(v);
            if &v == my_sid {
                return Err(Error::Stream("Servent loopback"));
            }
        } else if kind == Kind::Helo && id == PCP_HELO_BCID {
            out.bcid = Some(atom.read_bytes16()?);
        } else if kind == Kind::Helo && id == PCP_HELO_OSTYPE {
            out.os_type = Some(atom.read_int()?);
        } else if id == PCP_HELO_PORT {
            out.port = Some(atom.read_short()?);
        } else if kind == Kind::Helo && id == PCP_HELO_PING {
            out.ping = Some(atom.read_short()?);
        } else if kind == Kind::Oleh && id == PCP_HELO_REMOTEIP {
            out.remote_ip = Some(atom.read_address()?);
        } else if kind == Kind::Oleh && id == PCP_HELO_DISABLE {
            out.disable = Some(atom.read_int()?);
        } else {
            log(&[b"PCP handshake skip: ", id_str(&id)].concat());
            atom.skip(c, dlen)?;
        }
    }
    Ok(())
}

/// `PCPStream::readVersion`: 長さ (4 でなければ例外) と版を読み、版を返す。
pub fn read_version<IO: AtomIo>(io: &mut IO) -> Result<i32> {
    let len = io.read_i32()?;
    if len != 4 {
        return Err(Error::Stream("Invalid PCP"));
    }
    io.read_i32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcp::tests::{ser, A};
    use crate::reader::SliceReader;

    fn read(kind: Kind, a: &A, sid: [u8; 16]) -> (Result<()>, Hello, Vec<String>) {
        let mut buf = Vec::new();
        ser(a, &mut buf);
        let mut atom = AtomStream::new(StreamIo { r: SliceReader { data: &buf, pos: 0 } });
        let mut out = Hello::default();
        out.ping_sid_init = [9; 16];
        let mut logs = Vec::new();
        let r = read_hello(&mut atom, kind, &sid, &mut |m| logs.push(String::from_utf8_lossy(m).into_owned()), &mut out);
        (r, out, logs)
    }

    #[test]
    fn helo() {
        let a = A::Parent(
            b"helo",
            vec![
                A::Leaf(b"agnt", b"PeerCast/0.1218\0".to_vec()),
                A::Leaf(b"ver", 1218i32.to_le_bytes().to_vec()),
                A::Leaf(b"sid", vec![7; 16]),
                A::Leaf(b"port", 7144i16.to_le_bytes().to_vec()),
                A::Leaf(b"ping", (-2i16).to_le_bytes().to_vec()),
                A::Leaf(b"rip", vec![1, 2, 3, 4]),
            ],
        );
        let (r, h, logs) = read(Kind::Helo, &a, [0; 16]);
        assert_eq!(r, Ok(()));
        assert_eq!(h.agent.as_deref(), Some(&b"PeerCast/0.1218"[..]));
        assert_eq!(h.version, Some(1218));
        assert_eq!(h.session_id, Some([7; 16]));
        assert_eq!(h.port, Some(7144));
        assert_eq!(h.ping, Some(-2));
        // helo の rip は読み飛ばす
        assert_eq!(h.remote_ip, None);
        assert_eq!(logs, vec!["PCP handshake skip: rip"]);

        // 自分自身
        let (r, h, _) = read(Kind::Helo, &a, [7; 16]);
        assert_eq!(r, Err(Error::Stream("Servent loopback")));
        assert_eq!(h.agent.as_deref(), Some(&b"PeerCast/0.1218"[..]));
        assert_eq!(h.port, None);

        // 期待した atom でない
        let (r, h, _) = read(Kind::Oleh, &a, [0; 16]);
        assert_eq!(r, Ok(()));
        assert_eq!(h.unexpected, Some(PCP_HELO));
    }

    #[test]
    fn agent_reuses_buffer() {
        // 2 つめのエージェントが NUL で終わらず短いと、1 つめの続きが残る (C++ 版の arg と同じ)
        let a = A::Parent(b"oleh", vec![A::Leaf(b"agnt", b"abcdef\0".to_vec()), A::Leaf(b"agnt", b"XY".to_vec())]);
        let (r, h, _) = read(Kind::Oleh, &a, [0; 16]);
        assert_eq!(r, Ok(()));
        assert_eq!(h.agent.as_deref(), Some(&b"XYcdef"[..]));
        // 64 バイトを超えると例外
        let a = A::Parent(b"oleh", vec![A::Leaf(b"agnt", vec![b'a'; 65])]);
        assert_eq!(read(Kind::Oleh, &a, [0; 16]).0, Err(Error::Stream("checkData: Bad atom data")));
    }

    #[test]
    fn oleh_and_ping() {
        let a = A::Parent(
            b"oleh",
            vec![
                A::Leaf(b"rip", (0u8..16).collect()),
                A::Leaf(b"dis", 1i32.to_le_bytes().to_vec()),
                A::Leaf(b"sid", vec![5; 16]),
                A::Leaf(b"bcid", vec![1; 16]),
            ],
        );
        let (r, h, logs) = read(Kind::Oleh, &a, [0; 16]);
        assert_eq!(r, Ok(()));
        assert_eq!(h.disable, Some(1));
        assert!(matches!(h.remote_ip, Some(Ip::V6(v)) if v[0] == 15));
        assert_eq!(logs, vec!["PCP handshake skip: bcid"]);

        // ping: sid だけ読む (長さが 16 に足りなければ頭だけ)
        let a = A::Parent(b"oleh", vec![A::Leaf(b"sid", vec![9; 16]), A::Leaf(b"sid", vec![3; 4]), A::Leaf(b"agnt", b"x".to_vec())]);
        let (r, h, logs) = read(Kind::Ping, &a, [9; 16]);
        assert_eq!(r, Ok(()));
        assert_eq!(h.session_id, Some([3, 3, 3, 3, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]));
        assert_eq!(h.agent, None);
        assert!(logs.is_empty());
    }

    #[test]
    fn version() {
        let mut data = Vec::new();
        data.extend_from_slice(&4i32.to_le_bytes());
        data.extend_from_slice(&1i32.to_le_bytes());
        let mut io = StreamIo { r: SliceReader { data: &data, pos: 0 } };
        assert_eq!(read_version(&mut io), Ok(1));
        let data = 5i32.to_le_bytes();
        let mut io = StreamIo { r: SliceReader { data: &data, pos: 0 } };
        assert_eq!(read_version(&mut io), Err(Error::Stream("Invalid PCP")));
    }
}
