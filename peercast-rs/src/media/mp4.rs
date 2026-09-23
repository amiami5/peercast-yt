//! Fragmented MP4 (core/common/mp4.cpp の `MP4Stream`)。
//!
//! 先頭の ftyp と moov をヘッダーパケットにし、そのあとは moof と mdat の組を 1 つずつ読んで、
//! 15 KiB ずつのデータパケットにする。

use super::{fail, read_into, HeadKind, Host, LogLevel, Result, MAX_DATALEN};

/// `MAX_BOX_SIZE`
pub const MAX_BOX_SIZE: usize = 64 * 1024 * 1024;
/// `MP4Stream::MAX_OUTGOING_PACKET_SIZE`
pub const MAX_OUTGOING_PACKET_SIZE: usize = 15 * 1024;

pub struct Mp4;

/// `readBox`: 4 バイトのサイズ (ビッグエンディアン、ヘッダーを含む) のボックスを丸ごと読む。
fn read_box(h: &mut dyn Host) -> Result<Vec<u8>> {
    h.log(LogLevel::Debug, "readBox");
    let mut s = [0u8; 4];
    read_into(h, &mut s)?;
    let size = u32::from_be_bytes(s) as usize;
    h.log(LogLevel::Debug, &format!("size: {}", size as i32));

    // ボックスヘッダー (サイズ 4 バイト + 種別 4 バイト) より小さいものと、巨大なものは受け付けない
    if size < 8 {
        return fail("MP4: invalid box size");
    }
    if size > MAX_BOX_SIZE {
        return fail("MP4: box too large");
    }

    let mut data = vec![0u8; size];
    data[..4].copy_from_slice(&s);
    read_into(h, &mut data[4..])?;
    h.log(LogLevel::Trace, &format!("Box type {}", String::from_utf8_lossy(box_type(&data))));
    h.log(LogLevel::Debug, "readBox End");
    Ok(data)
}

fn box_type(b: &[u8]) -> &[u8] {
    &b[4..8]
}

impl Mp4 {
    pub fn read_header(&mut self, h: &mut dyn Host) -> Result<()> {
        h.log(LogLevel::Debug, "MP4Stream::readHeader");

        let ftyp = read_box(h)?;
        if box_type(&ftyp) != b"ftyp" {
            return fail("ftyp expected");
        }
        h.log(LogLevel::Debug, &format!("Got ftyp: {} bytes", ftyp.len()));

        let moov = read_box(h)?;
        if box_type(&moov) != b"moov" {
            return fail("moov expected");
        }
        h.log(LogLevel::Debug, &format!("Got moov: {} bytes", moov.len()));

        if ftyp.len() + moov.len() > MAX_DATALEN {
            return fail("head packet too large");
        }

        let mut head = ftyp;
        head.extend_from_slice(&moov);
        h.head(HeadKind::NewStream, &head)?;
        Ok(())
    }

    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        h.log(LogLevel::Debug, "MP4Stream::readPacket");

        let moof = read_box(h)?;
        if box_type(&moof) != b"moof" {
            return fail("moof expected");
        }
        let mdat = read_box(h)?;
        if box_type(&mdat) != b"mdat" {
            return fail("mdat expected");
        }

        let mut buf = moof;
        buf.extend_from_slice(&mdat);

        // C++ 版と同じく、1 秒をパケットの数 (切り捨て) で割った間隔で送る
        let npackets = buf.len() / MAX_OUTGOING_PACKET_SIZE;
        let sleep_seconds = if npackets > 0 { 1.0 / npackets as f64 } else { 0.0 };
        h.log(LogLevel::Trace, &format!("sleepSeconds = {}", sleep_seconds));

        for (i, chunk) in buf.chunks(MAX_OUTGOING_PACKET_SIZE).enumerate() {
            h.log(LogLevel::Trace, "newPacket");
            h.packet(chunk, i != 0, false)?;
            h.sleep((sleep_seconds * 1000.0) as i32);
        }

        // ストリームからの実測値が現在の公称値を超えていれば公称値を更新する
        h.raise_bitrate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testhost::{run, Rng, TestHost};
    use super::super::{Error, Kind};

    fn mp4box(ty: &[u8; 4], body_len: usize) -> Vec<u8> {
        let mut b = ((body_len + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(ty);
        b.extend(vec![0x55; body_len]);
        b
    }

    fn sample() -> Vec<u8> {
        let mut v = mp4box(b"ftyp", 16);
        v.extend(mp4box(b"moov", 500));
        v.extend(mp4box(b"moof", 100));
        v.extend(mp4box(b"mdat", 40000));
        v.extend(mp4box(b"moof", 100));
        v.extend(mp4box(b"mdat", 10));
        v
    }

    #[test]
    fn header_and_fragments() {
        let mut h = TestHost::new(&sample());
        assert_eq!(run(Kind::Mp4, &mut h, 3), Err(Error::Abort));
        assert_eq!(
            h.events,
            [
                "head NewStream 0 532",
                "data 532 15360 cont=false delay=false",
                "sleep 500",
                "data 15892 15360 cont=true delay=false",
                "sleep 500",
                "data 31252 9396 cont=true delay=false",
                "sleep 500",
                "data 40648 126 cont=false delay=false",
                "sleep 0",
            ]
        );
    }

    #[test]
    fn bad_boxes() {
        let mut v = 2u32.to_be_bytes().to_vec();
        v.extend(b"ftyp");
        v.extend([0; 64]);
        assert_eq!(run(Kind::Mp4, &mut TestHost::new(&v), 0), Err(Error::Stream("MP4: invalid box size".into())));
        let mut v = 0xffff_fff0u32.to_be_bytes().to_vec();
        v.extend(b"ftyp");
        assert_eq!(run(Kind::Mp4, &mut TestHost::new(&v), 0), Err(Error::Stream("MP4: box too large".into())));
        assert_eq!(run(Kind::Mp4, &mut TestHost::new(&mp4box(b"moov", 0)), 0), Err(Error::Stream("ftyp expected".into())));
        let mut v = mp4box(b"ftyp", 0);
        v.extend(mp4box(b"moov", 16380));
        assert_eq!(run(Kind::Mp4, &mut TestHost::new(&v), 0), Err(Error::Stream("head packet too large".into())));
    }

    #[test]
    fn fuzz() {
        let mut rng = Rng(42);
        let s = sample();
        for _ in 0..3000 {
            let _ = run(Kind::Mp4, &mut TestHost::new(&rng.mutate(&s)), 10);
        }
    }
}
