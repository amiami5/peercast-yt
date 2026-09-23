//! MP3 (core/common/mp3.cpp の `MP3Stream`)。
//!
//! 中身は解析せず、決まった長さずつデータパケットにする。ICY メタデータの間隔
//! (`icyMetaInterval`) があれば、その間隔ごとにストリームに埋め込まれたメタデータを取り出して
//! `processMp3Metadata` に渡す。

use super::{fail, read_into, Host, Result, MAX_DATALEN};

/// `ChanMgr::MAX_METAINT`
pub const MAX_METAINT: i32 = 8192;

pub struct Mp3;

impl Mp3 {
    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        // C++ 版の `ChanPacket pack` (1 回の readPacket の中で使い回す)
        let mut pack = vec![0u8; MAX_DATALEN];
        let interval = h.icy_meta_interval();

        if interval != 0 {
            let mut rlen = interval;
            while rlen != 0 {
                let rl = rlen.min(MAX_METAINT);
                // ChanPacket::init: 長さは unsigned int として比べる (負の間隔はここで止まる)
                if rl < 0 || rl as usize > MAX_DATALEN {
                    return fail("Packet data too large");
                }
                let rl = rl as usize;
                read_into(h, &mut pack[..rl])?;
                h.packet(&pack[..rl], false, true)?;
                rlen -= rl as i32;
            }

            let mut len = [0u8; 1];
            read_into(h, &mut len)?;
            let mut len = len[0] as usize;
            if len != 0 {
                if len * 16 > 1024 {
                    len = 1024 / 16;
                }
                // processMp3Metadata() は C 文字列として読むので、NUL で終わるようにする
                let mut buf = [0u8; 1024 + 1];
                read_into(h, &mut buf[..len * 16])?;
                h.mp3_metadata(&buf)?;
            }
        } else {
            let rl = MAX_METAINT as usize;
            read_into(h, &mut pack[..rl])?;
            h.packet(&pack[..rl], false, true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testhost::{run, Rng, TestHost};
    use super::super::{Error, Kind};

    #[test]
    fn plain_chunks() {
        let mut h = TestHost::new(&vec![7u8; 8192 * 2 + 5]);
        assert_eq!(run(Kind::Mp3, &mut h, 10), Err(Error::Abort));
        // 最後の 5 バイトは途中までの読み出しで、残りは前の内容のまま
        assert_eq!(h.events, ["data 0 8192 cont=false delay=true", "data 8192 8192 cont=false delay=true", "data 16384 8192 cont=false delay=true"]);
    }

    #[test]
    fn icy_metadata() {
        let mut data = vec![b'x'; 16];
        data.push(1);
        data.extend_from_slice(b"StreamTitle='a';");
        data.extend(vec![b'y'; 16]);
        data.push(0);
        let mut h = TestHost::new(&data);
        h.icy = 16;
        assert_eq!(run(Kind::Mp3, &mut h, 2), Ok(()));
        assert_eq!(h.events.len(), 2);
        assert_eq!(h.metadata.len(), 1);
        assert_eq!(&h.metadata[0][..17], b"StreamTitle='a';\0");
        assert_eq!(h.metadata[0].len(), 1025);
    }

    #[test]
    fn icy_metadata_is_capped_and_terminated() {
        let mut data = vec![b'x'; 16];
        data.push(255); // 255 * 16 バイトだが 1024 バイトまで
        data.extend(vec![b'A'; 4080]);
        let mut h = TestHost::new(&data);
        h.icy = 16;
        assert_eq!(run(Kind::Mp3, &mut h, 1), Ok(()));
        assert_eq!(h.pos, 16 + 1 + 1024);
        assert_eq!(h.metadata[0][1024], 0);
    }

    #[test]
    fn negative_interval() {
        let mut h = TestHost::new(b"abc");
        h.icy = -1;
        assert_eq!(run(Kind::Mp3, &mut h, 1), Err(Error::Stream("Packet data too large".into())));
    }

    #[test]
    fn fuzz() {
        let mut rng = Rng(0x1234_5678);
        for _ in 0..2000 {
            let n = rng.below(20000);
            let data: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            let mut h = TestHost::new(&data);
            h.icy = [0, 1, 16, 100, 8192, 9000, -5][rng.below(7)];
            let _ = run(Kind::Mp3, &mut h, 100);
        }
    }
}
