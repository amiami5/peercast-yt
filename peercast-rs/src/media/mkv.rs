//! Matroska / WebM (core/common/mkv.cpp の `MKVStream` と matroska.h の `VInt`)。
//!
//! Segment の中の Cluster より前の要素をヘッダーパケットにし、そのあとは Cluster を 1 つずつ
//! 読んで、なるべく要素の境目で 15 KiB 以下のデータパケットに分ける。
//!
//! C++ 版の `std::runtime_error` ("bad data" など) は、`readHeader` と `readPacket` が
//! 同じメッセージの `StreamException` に変えていたので、Rust 版も `Error::Stream` にする。

use super::{fail, read_exact, Error, HeadKind, Host, LogLevel, MemStream, Result, MAX_DATALEN};

/// `VInt::MAX_SIZE`: 要素のサイズとして受け付ける上限
pub const MAX_SIZE: u64 = 256 * 1024 * 1024;
/// パケットの大きさの目安
const PACKET_SIZE: usize = 15 * 1024;

/// `ID_TO_NAME`
fn id_name(id: &[u8]) -> &'static str {
    match id {
        [0x1A, 0x45, 0xDF, 0xA3] => "EBML",
        [0x18, 0x53, 0x80, 0x67] => "Segment",
        [0x1F, 0x43, 0xB6, 0x75] => "Cluster",
        [0xA3] => "SimpleBlock",
        [0x42, 0x86] => "EBMLVersion",
        [0x42, 0xF7] => "EBMLReadVersion",
        [0x42, 0xF2] => "EBMLMaxIDLength",
        [0x42, 0xF3] => "EBMLMaxSizeLength",
        [0x42, 0x82] => "DocType",
        [0x42, 0x87] => "DocTypeVersion",
        [0x42, 0x85] => "DocTypeReadVersion",
        [0x11, 0x4D, 0x9B, 0x74] => "SeekHead",
        [0xEC] => "Void",
        [0x16, 0x54, 0xAE, 0x6B] => "Tracks",
        [0xAE] => "TrackEntry",
        [0xD7] => "TrackNumber",
        [0x73, 0xC5] => "TrackUID",
        [0x83] => "TrackType",
        [0x12, 0x54, 0xC3, 0x67] => "Tags",
        [0x15, 0x49, 0xA9, 0x66] => "Info",
        [0xE7] => "Timecode",
        [0x1C, 0x53, 0xBB, 0x6B] => "Cues",
        [0x2A, 0xD7, 0xB1] => "TimecodeScale",
        [0x4D, 0x80] => "MuxingApp",
        [0x44, 0x89] => "Duration",
        [0x57, 0x41] => "WritingApp",
        [0x73, 0xA4] => "SegmentUID",
        [0x73, 0x73] => "Tag",
        [0x63, 0xC0] => "Targets",
        [0x67, 0xC8] => "SimpleTag",
        [0x63, 0xC5] => "TagTrackUID",
        [0x44, 0x87] => "TagString",
        [0x45, 0xA3] => "TagName",
        [0x4D, 0xBB] => "Seek",
        [0x53, 0xAB] => "SeekID",
        [0x53, 0xAC] => "SeekPosition",
        [0xBF] => "CRC-32",
        _ => "Unknown",
    }
}

/// EBML の可変長整数 (ID や要素のサイズ)。読んだバイト列をそのまま持つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VInt {
    pub bytes: Vec<u8>,
}

/// `VInt::numLeadingZeroes`
pub fn num_leading_zeroes(b: u8) -> usize {
    b.leading_zeros() as usize
}

impl VInt {
    /// `VInt::read`: 1 バイト目の先頭の 0 の数だけ後ろにバイトが続く
    fn read(mut read_char: impl FnMut() -> Result<u8>) -> Result<VInt> {
        let b = read_char()?;
        let nzeroes = num_leading_zeroes(b);
        if nzeroes > 7 {
            return fail("bad data");
        }
        let mut bytes = vec![b];
        for _ in 0..nzeroes {
            bytes.push(read_char()?);
        }
        VInt::new(bytes)
    }

    pub fn new(bytes: Vec<u8>) -> Result<VInt> {
        if bytes[0] == 0xff {
            return fail("UNKNOWN value not supported");
        }
        if num_leading_zeroes(bytes[0]) > 7 {
            return fail("bad data");
        }
        Ok(VInt { bytes })
    }

    /// `VInt::uint`: 長さを示すビットを除いた値
    pub fn uint(&self) -> u64 {
        let zeroes = num_leading_zeroes(self.bytes[0]);
        let len = zeroes + 1;
        let mut value = (((self.bytes[0] as u32) << len) & 0xff) as u64 >> len;
        for i in 0..zeroes {
            value = (value << 8) | self.bytes[i + 1] as u64;
        }
        value
    }

    /// `VInt::checkedUint`: 要素のサイズとして受け付けられる値か確かめる
    pub fn checked_uint(&self) -> Result<usize> {
        let v = self.uint();
        if v > MAX_SIZE {
            return fail("MKV: element size too large");
        }
        Ok(v as usize)
    }

    pub fn name(&self) -> &'static str {
        id_name(&self.bytes)
    }
}

/// `MKVStream::unpackUnsignedInt`: ビッグエンディアンの符号なし整数 (8 バイトを超える分は溢れる)
pub fn unpack_unsigned_int(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() {
        return fail("empty string");
    }
    Ok(bytes.iter().fold(0u64, |res, &b| (res << 8) | b as u64))
}

/// C++ の `StringStream` からの読み出し (Tracks と Info の中身を読むのに使われていた)。
/// 途中までの読み出しはでき、末尾で読もうとすると例外。
struct StrStream<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl StrStream<'_> {
    /// `StringStream::read`: 読めた分だけ読む。末尾なら例外。
    fn read(&mut self, n: usize) -> Result<&[u8]> {
        if self.pos == self.buf.len() {
            return fail("End of stream");
        }
        let start = self.pos;
        self.pos = (self.pos + n).min(self.buf.len());
        Ok(&self.buf[start..self.pos])
    }

    fn read_char(&mut self) -> Result<u8> {
        Ok(self.read(1)?[0])
    }

    fn eof(&self) -> bool {
        self.pos == self.buf.len()
    }

    /// `Stream::skip`: 4096 バイトずつ読み捨てる
    fn skip(&mut self, n: usize) -> Result<()> {
        let mut len = n;
        while len > 0 {
            let r = len.min(4096);
            self.read(r)?;
            len -= r;
        }
        Ok(())
    }

    /// `Stream::read(int)`: 4096 バイトずつ、読めた分を集める
    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        let mut remaining = n;
        while remaining > 0 {
            let r = self.read(remaining.min(4096))?;
            remaining -= r.len();
            res.extend_from_slice(r);
        }
        Ok(res)
    }
}

pub struct Mkv {
    video_track_number: u64,
    has_key_frame: bool,
    /// ナノ秒
    timecode_scale: u64,
    start_time: u32,
}

impl Default for Mkv {
    fn default() -> Self {
        Mkv { video_track_number: 1, has_key_frame: false, timecode_scale: 1_000_000, start_time: 0 }
    }
}

fn host_vint(h: &mut dyn Host) -> Result<VInt> {
    VInt::read(|| Ok(h.read_char()?))
}

fn mem_vint(m: &mut MemStream) -> Result<VInt> {
    VInt::read(|| Ok(m.read_char()))
}

impl Mkv {
    /// `sendPacket`: data を type パケットとして送る
    fn send_packet(&mut self, head: bool, data: &[u8], cont: bool, h: &mut dyn Host) -> Result<()> {
        if data.len() > MAX_DATALEN {
            return fail("MKV packet too big");
        }
        // rateLimit で律速するので checkReadDelay は使わない。
        if head {
            h.head(HeadKind::Mkv, data)?;
        } else {
            h.packet(data, cont, false)?;
        }
        Ok(())
    }

    /// `hasKeyFrame`: Cluster の中に、映像トラックのキーフレームの SimpleBlock があるか
    fn has_key_frame(&mut self, cluster: &[u8]) -> Result<bool> {
        let mut m = MemStream::new(cluster);
        mem_vint(&mut m)?;
        let size = mem_vint(&mut m)?;

        let mut payload_remaining = size.uint() as i64;
        while payload_remaining > 0 {
            let id = mem_vint(&mut m)?;
            let size = mem_vint(&mut m)?;
            let block = m.read_exact(size.checked_uint()?)?;

            if id.name() == "SimpleBlock" {
                let mut mem = MemStream::new(&block);
                let trackno = mem_vint(&mut mem)?;
                if trackno.uint() == self.video_track_number {
                    // トラック番号、タイムコード (2 バイト) のあとのフラグ。
                    // C++ 版は SimpleBlock が短いとデータの外を読んでいた。Rust 版は 0 とみなす。
                    let flags = block.get(trackno.bytes.len() + 2).copied().unwrap_or(0);
                    if flags & 0x80 != 0 {
                        self.has_key_frame = true;
                        return Ok(true); // キーフレームがある
                    }
                }
            }
            payload_remaining -= (id.bytes.len() + size.bytes.len()) as i64 + size.uint() as i64;
        }
        if payload_remaining != 0 {
            return fail("MKV Parse error");
        }
        Ok(false)
    }

    /// `rateLimit`: Timecode の時刻まで待つ
    fn rate_limit(&mut self, timecode: u64, h: &mut dyn Host) {
        // Timecode は単調増加ではないが、少しのジッターはバッファーが吸収してくれるだろう。
        let seconds_from_start = (timecode.wrapping_mul(self.timecode_scale) / 1_000_000_000) as u32;
        let ctime = h.time();

        let target = self.start_time.wrapping_add(seconds_from_start);
        if target > ctime {
            let diff = target.wrapping_sub(ctime) as i32;
            h.log(LogLevel::Debug, &format!("rateLimit: diff = {} sec", diff));
            h.sleep(diff.wrapping_mul(1000));
        }
    }

    /// `sendCluster`: 非継続パケットの頭出しができないクライアントのために、なるべく要素を
    /// パケットの先頭にして送信する
    fn send_cluster(&mut self, cluster: &[u8], h: &mut dyn Host) -> Result<()> {
        let mut cont = if self.has_key_frame(cluster)? { false } else { self.has_key_frame };

        let mut m = MemStream::new(cluster);
        let id = mem_vint(&mut m)?;
        let size = mem_vint(&mut m)?;
        let mut buffer = [id.bytes, size.bytes.clone()].concat();

        let mut payload_remaining = size.uint() as i64;
        while payload_remaining > 0 {
            let id = mem_vint(&mut m)?;
            let size = mem_vint(&mut m)?;
            h.log(LogLevel::Debug, &format!("Got {} size={}", id.name(), size.uint()));

            let element_len = (id.bytes.len() + size.bytes.len()) as u64 + size.uint();
            if !buffer.is_empty() && buffer.len() as u64 + element_len > PACKET_SIZE as u64 {
                self.send_packet(false, &buffer, cont, h)?;
                cont = true;
                buffer.clear();
            }

            let payload = m.read_exact(size.checked_uint()?)?;

            if id.name() == "Timecode" && h.read_delay() {
                let tc = unpack_unsigned_int(&payload)?;
                self.rate_limit(tc, h);
            }

            if element_len > PACKET_SIZE as u64 {
                if !buffer.is_empty() {
                    return fail("Logic error");
                }
                let element = [&id.bytes[..], &size.bytes, &payload].concat();
                for chunk in element.chunks(PACKET_SIZE) {
                    self.send_packet(false, chunk, cont, h)?;
                    cont = true;
                }
            } else {
                buffer.extend_from_slice(&id.bytes);
                buffer.extend_from_slice(&size.bytes);
                buffer.extend_from_slice(&payload);
            }
            payload_remaining -= element_len as i64;
        }

        if !buffer.is_empty() {
            self.send_packet(false, &buffer, cont, h)?;
        }
        Ok(())
    }

    /// `readTracks`: Tracks 要素からビデオトラックのトラック番号を調べる。
    fn read_tracks(&mut self, data: &[u8], h: &mut dyn Host) -> Result<()> {
        let mut mem = StrStream { buf: data, pos: 0 };

        while !mem.eof() {
            let id = VInt::read(|| mem.read_char())?;
            let size = VInt::read(|| mem.read_char())?;
            h.log(LogLevel::Debug, &format!("Got LEVEL2 {} size={}", id.name(), size.uint()));

            if id.name() == "TrackEntry" {
                let end = mem.pos as i64 + size.checked_uint()? as i64;
                let mut trackno: i32 = -1;
                let mut tracktype: i32 = -1;

                while (mem.pos as i64) < end {
                    let id = VInt::read(|| mem.read_char())?;
                    let size = VInt::read(|| mem.read_char())?;

                    // 値は 1 バイト目だけを見る (C++ 版と同じ)
                    match id.name() {
                        "TrackNumber" => trackno = mem.read_char()? as i32,
                        "TrackType" => tracktype = mem.read_char()? as i32,
                        _ => mem.skip(size.checked_uint()?)?,
                    }
                }

                if tracktype == 1 {
                    // TrackNumber がなければ -1 (を符号なしにした値) になる (C++ 版と同じ)
                    self.video_track_number = trackno as i64 as u64;
                    h.log(LogLevel::Debug, &format!("MKV video track number is {}", trackno));
                }
            } else {
                mem.skip(size.checked_uint()?)?;
            }
        }
        Ok(())
    }

    /// `readInfo`: TimecodeScale の値を調べる。壊れていてもエラーにはしない。
    fn read_info(&mut self, data: &[u8], h: &mut dyn Host) {
        h.log(LogLevel::Trace, &format!("Info length = {}", data.len()));
        let mut mem = StrStream { buf: data, pos: 0 };
        let r = (|| -> Result<()> {
            while mem.pos < mem.buf.len() {
                let id = VInt::read(|| mem.read_char())?;
                let size = VInt::read(|| mem.read_char())?;
                let n = size.checked_uint()?;
                let data = mem.read_exact(n)?;
                if id.name() == "TimecodeScale" {
                    let scale = unpack_unsigned_int(&data)?;
                    h.log(LogLevel::Debug, &format!("TimecodeScale = {} nanoseconds", scale as i32));
                    self.timecode_scale = scale;
                }
            }
            Ok(())
        })();
        if let Err(Error::Stream(e)) = r {
            h.log(LogLevel::Error, &format!("Failed to parse Info: {}", e));
        }
    }

    pub fn read_header(&mut self, h: &mut dyn Host) -> Result<()> {
        // ヘッダーは MAX_DATALEN を超えると送れない (sendPacket が例外を投げる)。C++ 版は
        // Cluster まで全部を溜めていたが、Rust 版は超えた分を溜めずに長さだけ数える。
        let mut header = Header::default();

        loop {
            let id = host_vint(h)?;
            let size = host_vint(h)?;
            h.log(LogLevel::Debug, &format!("Got LEVEL0 {} size={}", id.name(), size.uint()));

            header.add(&id.bytes);
            header.add(&size.bytes);

            if id.name() != "Segment" {
                // Segment 以外のレベル 0 要素は単にヘッドパケットに追加する
                let data = read_exact(h, size.checked_uint()?)?;
                header.add(&data);
                continue;
            }

            // Segment 内のレベル 1 要素を読む
            loop {
                let id = host_vint(h)?;
                let size = host_vint(h)?;
                h.log(LogLevel::Debug, &format!("Got LEVEL1 {} size={}", id.name(), size.uint()));

                if id.name() != "Cluster" {
                    // Cluster 以外の要素はヘッドパケットに追加する
                    header.add(&id.bytes);
                    header.add(&size.bytes);
                    let data = read_exact(h, size.checked_uint()?)?;
                    if id.name() == "Tracks" {
                        self.read_tracks(&data, h)?;
                    }
                    header.add(&data);
                    if id.name() == "Info" {
                        self.read_info(&data, h);
                    }
                } else {
                    // ヘッダーパケットを送信
                    if header.overflow {
                        return fail("MKV packet too big");
                    }
                    self.send_packet(true, &header.data, false, h)?;
                    self.start_time = h.time();

                    // もうIDとサイズを読んでしまったので、最初のクラスターを送信
                    let mut cluster = [id.bytes, size.bytes.clone()].concat();
                    cluster.extend(read_exact(h, size.checked_uint()?)?);
                    return self.send_cluster(&cluster, h);
                }
            }
        }
    }

    /// `checkBitrate` のあと、Cluster 要素を 1 つ読んで送る
    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        h.raise_bitrate()?;

        let id = host_vint(h)?;
        let size = host_vint(h)?;
        if id.name() != "Cluster" {
            h.log(LogLevel::Error, &format!("Cluster expected, but got {}", id.name()));
            return fail("Logic error");
        }

        let mut cluster = [id.bytes, size.bytes.clone()].concat();
        cluster.extend(read_exact(h, size.checked_uint()?)?);
        self.send_cluster(&cluster, h)
    }
}

/// `readHeader` で溜めるヘッダーパケット
#[derive(Default)]
struct Header {
    data: Vec<u8>,
    overflow: bool,
}

impl Header {
    fn add(&mut self, b: &[u8]) {
        if self.overflow || self.data.len() + b.len() > MAX_DATALEN {
            self.overflow = true;
            self.data.clear();
        } else {
            self.data.extend_from_slice(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testhost::{run, Rng, TestHost};
    use super::super::Kind;
    use super::*;

    #[test]
    fn vint() {
        assert_eq!(num_leading_zeroes(0), 8);
        assert_eq!(num_leading_zeroes(1), 7);
        assert_eq!(num_leading_zeroes(0x40), 1);
        assert_eq!(num_leading_zeroes(0x80), 0);
        assert_eq!(VInt::new(vec![0x81]).unwrap().uint(), 1);
        assert_eq!(VInt::new(vec![0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).unwrap().uint(), 0x00ff_ffff_ffff_ffff);
        assert_eq!(VInt::new(vec![0x7f, 0xff]).unwrap().uint(), 0x3fff);
        assert_eq!(VInt::new(vec![0xff]), Err(Error::Stream("UNKNOWN value not supported".into())));
        assert_eq!(VInt::new(vec![0x1A, 0x45, 0xDF, 0xA3]).unwrap().name(), "EBML");
        assert_eq!(VInt::new(vec![0x1A, 0x45, 0xDF]).unwrap().name(), "Unknown");
    }

    #[test]
    fn unpack() {
        assert!(unpack_unsigned_int(b"").is_err());
        assert_eq!(unpack_unsigned_int(b"\x01"), Ok(1));
        assert_eq!(unpack_unsigned_int(b"\x01\x02"), Ok(258));
    }

    /// 要素 (ID とサイズと中身)。サイズは 8 バイトの VInt で書く
    fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.push(0x01);
        v.extend(&(body.len() as u64).to_be_bytes()[1..]);
        v.extend(body);
        v
    }

    fn block(track: u8, key: bool, len: usize) -> Vec<u8> {
        let mut b = vec![0x80 | track, 0, 0, if key { 0x80 } else { 0 }];
        b.extend(vec![0x33; len]);
        el(&[0xA3], &b)
    }

    fn sample() -> Vec<u8> {
        let mut v = el(&[0x1A, 0x45, 0xDF, 0xA3], &el(&[0x42, 0x82], b"webm"));
        let info = el(&[0x15, 0x49, 0xA9, 0x66], &el(&[0x2A, 0xD7, 0xB1], &[0x0f, 0x42, 0x40]));
        let entry = |no: u8, ty: u8| el(&[0xAE], &[el(&[0xD7], &[no]), el(&[0x73, 0xC5], &[1, 2]), el(&[0x83], &[ty])].concat());
        let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &[entry(1, 2), entry(2, 1)].concat());
        let c1 = el(&[0x1F, 0x43, 0xB6, 0x75], &[el(&[0xE7], &[0x00]), block(2, true, 20000), block(1, false, 100)].concat());
        let c2 = el(&[0x1F, 0x43, 0xB6, 0x75], &[el(&[0xE7], &[0x03, 0xE8]), block(1, true, 100), block(2, false, 9000), block(2, false, 9000)].concat());
        let mut seg = vec![0x18, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0, 0, 0];
        seg.extend([info, tracks, c1, c2].concat());
        v.extend(seg);
        v
    }

    #[test]
    fn header_and_clusters() {
        let mut h = TestHost::new(&sample());
        h.read_delay = true;
        h.now = 100.0;
        assert_eq!(run(Kind::Mkv, &mut h, 2), Err(Error::Abort));
        assert_eq!(
            h.events,
            [
                // EBML (26) + Segment の ID とサイズ (12) + Info (26) + Tracks (94)
                "head Mkv 0 158",
                // 1 つ目の Cluster: 映像 (トラック 2) のキーフレームがあるので、先頭は非継続。
                // 大きい SimpleBlock の前で区切り、それ自体は 15 KiB ずつに分ける。
                "data 158 22 cont=false delay=false",
                "data 180 15360 cont=true delay=false",
                "data 15540 4653 cont=true delay=false",
                "data 20193 113 cont=true delay=false",
                // 2 つ目: Timecode 1000 (TimecodeScale 1ms) で 1 秒待つ。キーフレームはない
                "sleep 1000",
                "data 20306 9149 cont=true delay=false",
                "data 29455 9013 cont=true delay=false",
            ]
        );
    }

    #[test]
    fn huge_element_rejected() {
        let mut v = el(&[0x1A, 0x45, 0xDF, 0xA3], b"");
        v.extend([0x18, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0, 0, 0]);
        v.extend([0x16, 0x54, 0xAE, 0x6B, 0x01, 0, 0, 0, 0xff, 0xff, 0xff, 0xf0]);
        assert_eq!(run(Kind::Mkv, &mut TestHost::new(&v), 0), Err(Error::Stream("MKV: element size too large".into())));
    }

    #[test]
    fn fuzz() {
        let mut rng = Rng(99);
        let s = sample();
        for _ in 0..3000 {
            let mut h = TestHost::new(&rng.mutate(&s));
            h.read_delay = rng.below(2) == 0;
            let _ = run(Kind::Mkv, &mut h, 20);
        }
    }
}
