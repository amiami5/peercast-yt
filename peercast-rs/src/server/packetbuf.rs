//! チャンネルのパケットとそのバッファ (core/common/chanpacket.cpp の `ChanPacket` と `ChanPacketBuffer`)。
//! 位置の計算とパケットの出し入れは段階 6c の `crate::chanpacket`。

use std::sync::Mutex;

use super::error::{Error, Result};
use super::stream::Stream;
use super::sys;
use crate::chanpacket as cp;

pub use crate::chanpacket::{MAX_DATALEN, MAX_PACKETS};

/// `ChanPacket::TYPE`
pub const T_UNKNOWN: i32 = 0;
pub const T_HEAD: i32 = 1;
pub const T_DATA: i32 = 2;
pub const T_META: i32 = 4;
pub const T_PCP: i32 = 16;
pub const T_ALL: i32 = 0xff;

/// `ChanPacket` (中身は `data[..len]`)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChanPacket {
    pub ty: i32,
    pub pos: u32,
    pub sync: u32,
    pub cont: bool,
    pub data: Vec<u8>,
}

impl ChanPacket {
    /// `init(type, data, len, pos)`: 長すぎれば例外
    pub fn new(ty: i32, data: &[u8], pos: u32) -> Result<ChanPacket> {
        if data.len() > MAX_DATALEN {
            return Err(Error::stream("Packet data too large"));
        }
        Ok(ChanPacket { ty, pos, sync: 0, cont: false, data: data.to_vec() })
    }

    pub fn len(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `writeRaw`
    pub fn write_raw(&self, out: &mut dyn Stream) -> Result<()> {
        out.write(&self.data)
    }

    fn to_cp(&self, dst: &mut cp::Packet) {
        dst.kind = self.ty;
        dst.len = self.data.len() as u32;
        dst.pos = self.pos;
        dst.sync = self.sync;
        dst.cont = self.cont;
        let n = self.data.len().min(MAX_DATALEN);
        dst.data[..n].copy_from_slice(&self.data[..n]);
    }

    fn from_cp(src: &cp::Packet) -> ChanPacket {
        let n = (src.len as usize).min(MAX_DATALEN);
        ChanPacket { ty: src.kind, pos: src.pos, sync: src.sync, cont: src.cont, data: src.data[..n].to_vec() }
    }
}

fn empty_packet() -> cp::Packet {
    cp::Packet { kind: 0, len: 0, pos: 0, sync: 0, cont: false, data: [0; MAX_DATALEN] }
}

struct Inner {
    packets: Vec<cp::Packet>,
    p: cp::Positions,
}

impl Inner {
    fn buf(&mut self) -> cp::Buffer<'_> {
        cp::Buffer { packets: &mut self.packets, p: &mut self.p }
    }
}

/// `ChanPacketBuffer`: 最近の 64 個のパケット (自分のロックを持つ)
pub struct PacketBuffer {
    inner: Mutex<Inner>,
}

impl Default for PacketBuffer {
    fn default() -> Self {
        PacketBuffer::new()
    }
}

impl PacketBuffer {
    pub fn new() -> PacketBuffer {
        let packets = (0..MAX_PACKETS).map(|_| empty_packet()).collect();
        PacketBuffer { inner: Mutex::new(Inner { packets, p: cp::Positions::default() }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `init` (受け付ける種類は残す。C++ 版も `accept` は init で 0 にするが、呼ぶ側が直後に設定する)
    pub fn init(&self) {
        self.lock().buf().init();
    }

    /// `init` して受け付ける種類を決める
    pub fn init_accept(&self, accept: i32) {
        let mut g = self.lock();
        g.buf().init();
        g.p.accept = accept as u32;
    }

    pub fn accept(&self) -> u32 {
        self.lock().p.accept
    }

    pub fn num_pending(&self) -> i32 {
        self.lock().buf().num_pending()
    }

    pub fn will_skip(&self) -> bool {
        self.lock().buf().will_skip()
    }

    pub fn write_pos(&self) -> u32 {
        self.lock().p.write_pos
    }

    pub fn last_write_time(&self) -> u32 {
        self.lock().p.last_write_time
    }

    pub fn set_last_write_time(&self, t: u32) {
        self.lock().p.last_write_time = t;
    }

    /// `writePacket`: 書いたら true (`pack.sync` に通し番号が入る)
    pub fn write_packet(&self, pack: &mut ChanPacket, update_read_pos: bool) -> bool {
        if pack.data.is_empty() {
            return false;
        }
        // C++ 版と同じく、willSkip は書き込みとは別のロックで確かめる
        if self.will_skip() {
            return false;
        }
        let mut g = self.lock();
        let now = sys::get_time();
        let mut tmp = empty_packet();
        pack.to_cp(&mut tmp);
        let r = g.buf().write_packet(&mut tmp, update_read_pos, now);
        pack.sync = tmp.sync;
        r
    }

    /// `readPacket`: 次のパケットを読む。遅れすぎていれば例外、なければ 30 秒まで待つ
    pub fn read_packet(&self) -> Result<ChanPacket> {
        let tim = sys::get_time();
        {
            let mut g = self.lock();
            if g.buf().too_far_behind() {
                return Err(Error::stream("Read too far behind"));
            }
        }
        loop {
            {
                let mut g = self.lock();
                if !g.buf().is_empty() {
                    let mut tmp = empty_packet();
                    g.buf().take(&mut tmp);
                    drop(g);
                    sys::sleep_idle();
                    return Ok(ChanPacket::from_cp(&tmp));
                }
            }
            sys::sleep_idle();
            if sys::get_time().wrapping_sub(tim) > 30 {
                return Err(Error::timeout());
            }
        }
    }

    /// `findPacket`
    pub fn find_packet(&self, spos: u32) -> Option<ChanPacket> {
        let mut g = self.lock();
        let mut tmp = empty_packet();
        if g.buf().find_packet(spos, &mut tmp) {
            Some(ChanPacket::from_cp(&tmp))
        } else {
            None
        }
    }

    pub fn latest_pos(&self) -> u32 {
        self.lock().buf().latest_pos()
    }

    pub fn oldest_pos(&self) -> u32 {
        self.lock().buf().oldest_pos()
    }

    pub fn find_oldest_pos(&self, spos: u32) -> u32 {
        self.lock().buf().find_oldest_pos(spos)
    }

    pub fn latest_non_continuation_pos(&self) -> u32 {
        self.lock().buf().latest_non_continuation_pos()
    }

    pub fn oldest_non_continuation_pos(&self) -> u32 {
        self.lock().buf().oldest_non_continuation_pos()
    }

    /// `getStatistics`
    pub fn statistics(&self) -> cp::Stat {
        self.lock().buf().statistics()
    }

    /// `getLastSync` はないが、`lastPos` の位置の終わり (`getStreamPosEnd(lastPos)`)
    pub fn stream_pos_end_last(&self) -> u32 {
        let mut g = self.lock();
        let last = g.p.last_pos;
        g.buf().stream_pos_end(last)
    }

    /// `copyFrom`
    pub fn copy_from(&self, src: &PacketBuffer, req_pos: u32) -> i32 {
        if std::ptr::eq(self, src) {
            return 0;
        }
        let mut d = self.lock();
        let mut s = src.lock();
        let sb = s.buf();
        d.buf().copy_from(&sb, req_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read() {
        let b = PacketBuffer::new();
        b.init_accept(T_HEAD | T_DATA);
        let mut p = ChanPacket::new(T_DATA, b"abc", 10).unwrap();
        assert!(b.write_packet(&mut p, false));
        assert_eq!(b.num_pending(), 1);
        let r = b.read_packet().unwrap();
        assert_eq!((r.data, r.pos, r.sync), (b"abc".to_vec(), 10, 0));
        assert_eq!(b.latest_pos(), 10);
        assert!(b.find_packet(5).is_some());
        assert!(b.find_packet(11).is_none());
        let mut e = ChanPacket::new(T_DATA, b"", 0).unwrap();
        assert!(!b.write_packet(&mut e, false));
    }
}
