//! チャンネルのパケットのバッファ (core/common/chanpacket.cpp の `ChanPacketBuffer`)。
//!
//! 最近の 64 個のパケットを輪の形に持つ。パケットには書いた順に通し番号 (`write_pos`) を振り、
//! 番号 i のパケットは `packets[i % 64]` にある。`first_pos` から `last_pos` までが残っている
//! パケットの番号、`read_pos` は次に読む番号。
//!
//! 今は、パケットと位置は C++ のクラスのメンバーにあり、ロックも C++ 側で取る。Rust は、その
//! 記憶領域を借りて位置の計算とパケットの出し入れをする (`Buffer`)。

/// `ChanPacket::MAX_DATALEN`
pub const MAX_DATALEN: usize = 16384;
/// `ChanPacketBuffer::MAX_PACKETS`
pub const MAX_PACKETS: u32 = 64;
/// `ChanPacketBuffer::NUM_SAFEPACKETS`
pub const NUM_SAFEPACKETS: u32 = 56;

/// `ChanPacket` (C の `pcrs_chan_packet`。C++ のクラスと同じ並び)
#[repr(C)]
pub struct Packet {
    /// `ChanPacket::TYPE`
    pub kind: i32,
    pub len: u32,
    /// パケットのストリーム中でのバイト位置
    pub pos: u32,
    pub sync: u32,
    /// 続きのパケット (キーフレームの途中など) か
    pub cont: bool,
    pub data: [u8; MAX_DATALEN],
}

impl Packet {
    /// `ChanPacket::operator=`: 中身は `len` バイトだけ写す
    pub fn assign(&mut self, other: &Packet) {
        self.kind = other.kind;
        self.len = other.len;
        self.pos = other.pos;
        self.sync = other.sync;
        self.cont = other.cont;
        let n = (other.len as usize).min(MAX_DATALEN);
        self.data[..n].copy_from_slice(&other.data[..n]);
    }
}

/// `ChanPacketBuffer` の位置など
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Positions {
    pub last_pos: u32,
    pub first_pos: u32,
    pub safe_pos: u32,
    pub read_pos: u32,
    pub write_pos: u32,
    pub accept: u32,
    pub last_write_time: u32,
}

/// `getStatistics` の結果
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub packet_lengths: Vec<u32>,
    pub continuations: i32,
    pub non_continuations: i32,
}

/// パケットのバッファ (64 個) と位置を借りたもの
pub struct Buffer<'a> {
    pub packets: &'a mut [Packet],
    pub p: &'a mut Positions,
}

/// `first..=last` の番号 (C++ 版は `last` が `UINT_MAX` だと終わらなかった。Rust 版は 1 周で終える)
fn range(first: u32, last: u32) -> impl DoubleEndedIterator<Item = u32> {
    let end = if first <= last { last as u64 + 1 } else { first as u64 };
    (first as u64..end).map(|i| i as u32)
}

impl Buffer<'_> {
    fn at(&self, index: u32) -> &Packet {
        &self.packets[(index % MAX_PACKETS) as usize]
    }

    pub fn init(&mut self) {
        *self.p = Positions::default();
    }

    pub fn num_pending(&self) -> i32 {
        self.p.write_pos.wrapping_sub(self.p.read_pos) as i32
    }

    /// バッファがいっぱい (読む側が 64 個以上遅れている) なら true
    pub fn will_skip(&self) -> bool {
        self.p.write_pos.wrapping_sub(self.p.read_pos) >= MAX_PACKETS
    }

    /// `writePacket`。書いたら true。`pack.sync` に通し番号を入れる。`now` は `sys->getTime()`。
    pub fn write_packet(&mut self, pack: &mut Packet, update_read_pos: bool, now: u32) -> bool {
        if pack.len == 0 || self.will_skip() {
            return false;
        }
        let w = self.p.write_pos;
        pack.sync = w;
        self.packets[(w % MAX_PACKETS) as usize].assign(pack);
        self.p.last_pos = w;
        self.p.write_pos = w.wrapping_add(1);
        let w = self.p.write_pos;
        self.p.first_pos = if w >= MAX_PACKETS { w - MAX_PACKETS } else { 0 };
        self.p.safe_pos = if w >= NUM_SAFEPACKETS { w - NUM_SAFEPACKETS } else { 0 };
        if update_read_pos {
            self.p.read_pos = w;
        }
        self.p.last_write_time = now;
        true
    }

    /// `readPacket` の、読む側が遅れすぎたか (`Read too far behind`)
    pub fn too_far_behind(&self) -> bool {
        self.p.read_pos < self.p.first_pos
    }

    /// `readPacket` の、まだ読むものがないか (C++ 側で待つ)
    pub fn is_empty(&self) -> bool {
        self.p.read_pos >= self.p.write_pos
    }

    /// `readPacket` の、次のパケットを `pack` に写して進めるところ
    pub fn take(&mut self, pack: &mut Packet) {
        let r = self.p.read_pos;
        pack.assign(&self.packets[(r % MAX_PACKETS) as usize]);
        self.p.read_pos = r.wrapping_add(1);
    }

    /// ストリーム位置が `spos` か、それより新しいパケットのうち一番古いものを `pack` に写す
    pub fn find_packet(&self, spos: u32, pack: &mut Packet) -> bool {
        if self.p.write_pos == 0 {
            return false;
        }
        let mut candidate: Option<u32> = None;
        for i in range(self.p.first_pos, self.p.last_pos) {
            let pos = self.at(i).pos;
            if pos >= spos && candidate.map_or(true, |c| pos < self.at(c).pos) {
                candidate = Some(i);
            }
        }
        match candidate {
            Some(c) => {
                pack.assign(self.at(c));
                true
            }
            None => false,
        }
    }

    pub fn stream_pos(&self, index: u32) -> u32 {
        self.at(index).pos
    }

    pub fn stream_pos_end(&self, index: u32) -> u32 {
        let p = self.at(index);
        p.pos.wrapping_add(p.len)
    }

    /// 一番新しいパケットのストリーム位置 (まだなければ 0)
    pub fn latest_pos(&self) -> u32 {
        if self.p.write_pos == 0 {
            0
        } else {
            self.stream_pos(self.p.last_pos)
        }
    }

    /// 一番古いパケットのストリーム位置 (まだなければ 0)
    pub fn oldest_pos(&self) -> u32 {
        if self.p.write_pos == 0 {
            0
        } else {
            self.stream_pos(self.p.first_pos)
        }
    }

    pub fn find_oldest_pos(&self, spos: u32) -> u32 {
        let min = self.stream_pos(self.p.safe_pos);
        let max = self.stream_pos(self.p.last_pos);
        if min > spos {
            min
        } else if max < spos {
            max
        } else {
            spos
        }
    }

    pub fn latest_non_continuation_pos(&self) -> u32 {
        if self.p.write_pos == 0 {
            return 0;
        }
        range(self.p.first_pos, self.p.last_pos).rev().map(|i| self.at(i)).find(|p| !p.cont).map_or(0, |p| p.pos)
    }

    pub fn oldest_non_continuation_pos(&self) -> u32 {
        if self.p.write_pos == 0 {
            return 0;
        }
        range(self.p.first_pos, self.p.last_pos).map(|i| self.at(i)).find(|p| !p.cont).map_or(0, |p| p.pos)
    }

    pub fn statistics(&self) -> Stat {
        let mut s = Stat::default();
        if self.p.write_pos == 0 {
            return s;
        }
        for i in range(self.p.first_pos, self.p.last_pos) {
            let p = self.at(i);
            s.packet_lengths.push(p.len);
            if p.cont {
                s.continuations = s.continuations.wrapping_add(1);
            } else {
                s.non_continuations = s.non_continuations.wrapping_add(1);
            }
        }
        s
    }

    /// `copyFrom` (使われていない)。`src` の `req_pos` 以降で `accept` の種類のパケットを写す。
    /// C++ 版と同じく `write_pos` は 0 に戻さない。
    pub fn copy_from(&mut self, src: &Buffer, req_pos: u32) -> i32 {
        self.p.first_pos = 0;
        self.p.last_pos = 0;
        self.p.safe_pos = 0;
        self.p.read_pos = 0;
        for i in range(src.p.first_pos, src.p.last_pos) {
            let s = src.at(i);
            if (s.kind as u32) & self.p.accept != 0 && s.pos >= req_pos {
                self.p.last_pos = self.p.write_pos;
                let w = self.p.write_pos;
                self.packets[(w % MAX_PACKETS) as usize].assign(s);
                self.p.write_pos = w.wrapping_add(1);
            }
        }
        self.p.last_pos.wrapping_sub(self.p.first_pos) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packets() -> Vec<Packet> {
        (0..MAX_PACKETS)
            .map(|_| Packet { kind: 0, len: 0, pos: 0, sync: 0, cont: false, data: [0; MAX_DATALEN] })
            .collect()
    }

    fn pkt(pos: u32, len: u32, cont: bool) -> Box<Packet> {
        let mut p = Box::new(Packet { kind: 2, len, pos, sync: 0, cont, data: [0; MAX_DATALEN] });
        p.data[0] = pos as u8;
        p
    }

    #[test]
    fn write_and_read() {
        let mut ps = packets();
        let mut pos = Positions::default();
        let mut b = Buffer { packets: &mut ps, p: &mut pos };
        assert!(!b.write_packet(&mut pkt(0, 0, false), false, 1));
        assert!(b.write_packet(&mut pkt(0, 10, false), false, 12345));
        assert_eq!(*b.p, Positions { write_pos: 1, last_write_time: 12345, ..Default::default() });
        let mut out = pkt(0, 0, false);
        assert!(!b.too_far_behind() && !b.is_empty());
        b.take(&mut out);
        assert_eq!((out.pos, out.len, out.sync), (0, 10, 0));
        assert!(b.is_empty());
    }

    #[test]
    fn ring_and_positions() {
        let mut ps = packets();
        let mut pos = Positions::default();
        let mut b = Buffer { packets: &mut ps, p: &mut pos };
        for i in 0..64 {
            assert!(b.write_packet(&mut pkt(i * 100, 100, i % 3 == 0), false, 0));
        }
        // 読む側が 64 個遅れているので書けない
        assert!(!b.write_packet(&mut pkt(6400, 100, false), true, 0));
        b.p.read_pos = 64;
        assert!(b.write_packet(&mut pkt(6400, 100, false), true, 0));
        assert_eq!(b.p.read_pos, 65);
        let mut out = pkt(0, 0, false);
        assert!(b.find_packet(150, &mut out));
        assert_eq!(out.pos, 200);
        assert_eq!(b.p.first_pos, 1);
        assert_eq!(b.p.safe_pos, 9);
        assert_eq!(b.latest_pos(), 6400);
        assert_eq!(b.oldest_pos(), 100);
        assert_eq!(b.find_oldest_pos(0), 900);
        assert_eq!(b.find_oldest_pos(99999), 6400);
        assert_eq!(b.latest_non_continuation_pos(), 6400);
        assert_eq!(b.oldest_non_continuation_pos(), 100);
        let s = b.statistics();
        assert_eq!(s.packet_lengths.len(), 64);
        assert_eq!(s.continuations, 21);
        assert!(!b.find_packet(6401, &mut out));
    }

    #[test]
    fn range_terminates_at_max() {
        assert_eq!(range(u32::MAX - 1, u32::MAX).count(), 2);
        assert_eq!(range(5, 4).count(), 0);
    }
}
