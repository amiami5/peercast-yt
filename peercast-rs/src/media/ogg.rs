//! Ogg Vorbis / Theora (core/common/ogg.cpp の `OGGStream`, `OggPage`, `OggPacket`,
//! `OggVorbisSubStream`, `OggTheoraSubStream`)。
//!
//! ページを 1 つずつ読む。BOS (ストリームの始まり) のあと、Vorbis と Theora の最初の 3 つの
//! パケット (ヘッダー) を含むページをヘッダーパケットに溜め、そろったら送る。それ以外のページは
//! そのままデータパケットにし、グラニュール位置の時刻まで待つ。

use super::{fail, read_into, HeadKind, Host, LogLevel, MemStream, Result, Track, MAX_DATALEN};

/// `OggPacket::MAX_BODYLEN`
const MAX_PACKET_BODYLEN: usize = 65536;
/// `OggPacket::MAX_PACKETS`
const MAX_PACKETS: usize = 256;
/// `OggPage::MAX_BODYLEN`
const MAX_PAGE_BODYLEN: usize = 65536;
/// `OggPage::MAX_HEADERLEN`
const MAX_HEADERLEN: usize = 27 + 256;

/// ページ 1 つ (ヘッダーとセグメントテーブルと本体)
struct Page {
    data: Vec<u8>,
    head_len: usize,
    body_len: usize,
    gran_pos: i64,
}

impl Page {
    /// `OggPage::read`: キャプチャパターン "OggS" まで読み飛ばしてから、ページを読む
    fn read(h: &mut dyn Host) -> Result<Page> {
        loop {
            // 途中で合わなかった文字は読み捨てる (C++ 版と同じ。"OOggS" では同期できない)
            let got = h.read_char()? == b'O' && h.read_char()? == b'g' && h.read_char()? == b'g' && h.read_char()? == b'S';
            if got {
                break;
            }
            h.log(LogLevel::Info, "Skipping OGG packet");
        }

        let mut header = [0u8; 27];
        header[..4].copy_from_slice(b"OggS");
        read_into(h, &mut header[4..])?;

        let num_segs = header[26] as usize;
        let mut data = header.to_vec();
        data.resize(27 + num_segs, 0);
        read_into(h, &mut data[27..])?;
        let body_len: usize = data[27..].iter().map(|&s| s as usize).sum();

        if body_len >= MAX_PAGE_BODYLEN {
            return fail("OGG body too big");
        }
        let head_len = 27 + num_segs;
        if head_len > MAX_HEADERLEN {
            return fail("OGG header too big");
        }

        data.resize(head_len + body_len, 0);
        read_into(h, &mut data[head_len..])?;

        // C++ 版は CPU のバイト順で読んでいた。Rust 版はリトルエンディアン (Ogg の仕様どおり)
        let gran_pos = i64::from_le_bytes(data[6..14].try_into().unwrap());
        Ok(Page { data, head_len, body_len, gran_pos })
    }

    fn is_bos(&self) -> bool {
        self.data[5] & 0x02 != 0
    }

    fn is_eos(&self) -> bool {
        self.data[5] & 0x04 != 0
    }

    fn serial_no(&self) -> u32 {
        u32::from_le_bytes(self.data[14..18].try_into().unwrap())
    }

    fn segments(&self) -> &[u8] {
        &self.data[27..self.head_len]
    }

    fn body(&self) -> &[u8] {
        &self.data[self.head_len..]
    }

    /// 本体の 2 バイト目からが `name` か (ページが短ければ一致しない)
    fn detect(&self, name: &[u8; 6]) -> bool {
        self.data.get(self.head_len + 1..self.head_len + 7) == Some(&name[..])
    }
}

/// `OggPacket`: ヘッダーのパケットを溜める
#[derive(Default)]
struct Packets {
    body: Vec<u8>,
    sizes: Vec<u32>,
}

impl Packets {
    /// 完成したパケットの数
    fn num_packets(&self) -> usize {
        self.sizes.len() - 1
    }

    fn reset(&mut self) {
        self.body.clear();
        self.sizes.clear();
        self.sizes.push(0);
    }

    /// `addLacing`: セグメントの長さ 255 未満でパケットが終わる
    fn add_lacing(&mut self, page: &Page) -> Result<()> {
        for &seg in page.segments() {
            *self.sizes.last_mut().unwrap() += seg as u32;
            if seg < 255 {
                self.sizes.push(0);
                if self.num_packets() >= MAX_PACKETS {
                    return fail("Too many OGG packets");
                }
            }
        }
        Ok(())
    }

    /// 完成したパケットを順に `MemoryStream` として取り出す
    fn each(&self) -> impl Iterator<Item = MemStream<'_>> {
        let mut ptr = 0usize;
        self.sizes[..self.num_packets()].iter().map(move |&n| {
            let start = ptr.min(self.body.len());
            ptr += n as usize;
            MemStream::new(&self.body[start..ptr.min(self.body.len())])
        })
    }
}

/// `OggSubStream` の共通部分
struct Sub {
    bitrate: i32,
    pack: Packets,
    max_headers: usize,
    serial_no: u32,
}

impl Default for Sub {
    fn default() -> Self {
        let mut pack = Packets::default();
        pack.reset();
        Sub { bitrate: 0, pack, max_headers: 0, serial_no: 0 }
    }
}

impl Sub {
    fn need_header(&self) -> bool {
        self.max_headers != 0 && self.pack.num_packets() < self.max_headers
    }

    fn eos(&mut self) {
        self.max_headers = 0;
        self.serial_no = 0;
    }

    fn bos(&mut self, ser: u32) {
        self.max_headers = 3;
        self.pack.reset();
        self.serial_no = ser;
        self.bitrate = 0;
    }

    fn is_active(&self) -> bool {
        self.serial_no != 0
    }

    /// `readHeader`: ページをヘッダーパケットとパケットの本体に足す。そろったら true
    fn read_header(&mut self, h: &mut dyn Host, page: &Page) -> Result<bool> {
        if self.pack.body.len() + page.body_len >= MAX_PACKET_BODYLEN {
            return fail("OGG packet too big");
        }
        // コピー先は ch->headPack.data (ChanPacket::MAX_DATALEN バイト)。
        if h.head_len() as usize + page.data.len() >= MAX_DATALEN {
            return fail("OGG packet too big for headPack");
        }

        // copy complete packet into head packet
        h.head_append(&page.data)?;
        // add body to packet
        self.pack.body.extend_from_slice(page.body());
        self.pack.add_lacing(page)?;

        Ok(self.pack.num_packets() >= self.max_headers)
    }
}

/// C++ の `Stream::readBits` (上位ビットから読む)
struct Bits<'a> {
    mem: MemStream<'a>,
    buffer: u8,
    pos: u32,
}

impl Bits<'_> {
    fn read(&mut self, cnt: u32) -> i32 {
        let mut v: u32 = 0;
        for i in (0..cnt).rev() {
            if self.pos == 0 {
                self.buffer = self.mem.read_char();
            }
            if self.buffer & (1 << (7 - self.pos)) != 0 && i < 32 {
                v |= 1 << i;
            }
            self.pos = (self.pos + 1) & 7;
        }
        v as i32
    }

    /// 値を使わずに読み飛ばす
    fn skip(&mut self, cnt: u32) {
        for _ in 0..cnt {
            self.read(1);
        }
    }
}

#[derive(Default)]
pub struct Ogg {
    vorbis: Sub,
    samplerate: i32,
    theora: Sub,
    granpos_shift: i32,
    frame_time: f64,
}

/// `stristr`: 大文字小文字を区別せずに探す (ASCII の a〜z だけ)
fn stristr(s: &[u8], pat: &[u8]) -> Option<usize> {
    s.windows(pat.len()).position(|w| w.eq_ignore_ascii_case(pat))
}

impl Ogg {
    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        let page = Page::read(h)?;
        let serial = page.serial_no();

        if page.is_bos() {
            if !self.vorbis.need_header() && !self.theora.need_header() {
                h.head_clear();
            }
            if page.detect(b"vorbis") {
                self.vorbis.bos(serial);
            }
            if page.detect(b"theora") {
                self.theora.bos(serial);
            }
        }

        if page.is_eos() {
            if serial == self.vorbis.serial_no {
                h.log(LogLevel::Info, "Vorbis stream: EOS");
                self.vorbis.eos();
            }
            if serial == self.theora.serial_no {
                h.log(LogLevel::Info, "Theora stream: EOS");
                self.theora.eos();
            }
        }

        if self.vorbis.need_header() || self.theora.need_header() {
            // eos() や未使用のサブストリームの serialNo は 0 なので、needHeader() も確認する。
            if self.vorbis.need_header() && serial == self.vorbis.serial_no {
                if self.vorbis.read_header(h, &page)? {
                    self.proc_vorbis_headers(h)?;
                }
            } else if self.theora.need_header() && serial == self.theora.serial_no {
                if self.theora.read_header(h, &page)? {
                    self.proc_theora_headers(h);
                }
            } else {
                return fail("Bad OGG serial no.");
            }

            if !self.vorbis.need_header() && !self.theora.need_header() {
                let mut bitrate = 0i32;
                if self.vorbis.is_active() {
                    bitrate = bitrate.wrapping_add(self.vorbis.bitrate);
                }
                let ogm = self.theora.is_active();
                if ogm {
                    bitrate = bitrate.wrapping_add(self.theora.bitrate);
                }
                h.ogg_set_info(bitrate, ogm);
                h.head(HeadKind::Ogg, &[])?;
                let len = h.head_len();
                h.log(LogLevel::Info, &format!("Got {} bytes of headers", len));
            }
        } else {
            // ChanPacket::init
            if page.data.len() > MAX_DATALEN {
                return fail("Packet data too large");
            }
            h.packet(&page.data, false, false)?;

            if self.theora.is_active() {
                if serial == self.theora.serial_no {
                    h.sleep_until(self.theora_time(&page));
                }
            } else if self.vorbis.is_active() && serial == self.vorbis.serial_no {
                h.sleep_until(page.gran_pos as f64 / self.samplerate as f64);
            }
        }
        Ok(())
    }

    /// `OggTheoraSubStream::getTime`: キーフレームの番号と、そこからのフレーム数の和
    fn theora_time(&self, page: &Page) -> f64 {
        let shift = self.granpos_shift as u32;
        let iframe = page.gran_pos >> shift;
        let pframe = page.gran_pos.wrapping_sub(iframe.wrapping_shl(shift));
        iframe.wrapping_add(pframe) as f64 * self.frame_time
    }

    /// `OggVorbisSubStream::procHeaders`
    fn proc_vorbis_headers(&mut self, h: &mut dyn Host) -> Result<()> {
        let pack = std::mem::take(&mut self.vorbis.pack);
        let r = (|| {
            for mut vin in pack.each() {
                let mut id = [0u8; 7];
                vin.read_into(&mut id);
                match id[0] {
                    1 => {
                        h.log(LogLevel::Info, &format!("OGG Vorbis Header: Ident ({} bytes)", vin.len()));
                        self.read_ident(&mut vin, h)?;
                    }
                    3 => {
                        h.log(LogLevel::Info, &format!("OGG Vorbis Header: Comment ({} bytes)", vin.len()));
                        let track = read_comment(&mut vin, h)?;
                        h.set_track(&track)?;
                    }
                    5 => {
                        h.log(LogLevel::Info, &format!("OGG Vorbis Header: Setup ({} bytes)", vin.len()));
                    }
                    _ => return fail("Unknown Vorbis packet header type"),
                }
            }
            Ok(())
        })();
        self.vorbis.pack = pack;
        r
    }

    /// `OggVorbisSubStream::readIdent`
    fn read_ident(&mut self, vin: &mut MemStream, h: &mut dyn Host) -> Result<()> {
        let ver = vin.read_long();
        let chans = vin.read_char() as i8;
        self.samplerate = vin.read_long();
        let br_max = vin.read_long();
        let br_nom = vin.read_long();
        let br_low = vin.read_long();

        vin.read_char(); // skip blocksize 0+1

        h.log(
            LogLevel::Info,
            &format!(
                "OGG Vorbis Ident: ver={}, chans={}, rate={}, brMax={}, brNom={}, brLow={}",
                ver, chans, self.samplerate, br_max, br_nom, br_low
            ),
        );

        self.vorbis.bitrate = br_nom / 1000;

        let frame = vin.read_char(); // framing bit
        if frame == 0 {
            return fail("Bad Indent frame");
        }
        Ok(())
    }

    /// `OggTheoraSubStream::procHeaders`
    fn proc_theora_headers(&mut self, h: &mut dyn Host) {
        let pack = std::mem::take(&mut self.theora.pack);
        for mut vin in pack.each() {
            let mut id = [0u8; 7];
            vin.read_into(&mut id);
            let len = vin.len();
            match id[0] {
                128 => {
                    h.log(LogLevel::Info, &format!("OGG Theora Header: Info ({} bytes)", len));
                    self.read_theora_info(vin, h);
                }
                n => h.log(LogLevel::Info, &format!("OGG Theora Header: Unknown {} ({} bytes)", n, len)),
            }
        }
        self.theora.pack = pack;
    }

    /// `OggTheoraSubStream::readInfo`
    fn read_theora_info(&mut self, vin: MemStream, h: &mut dyn Host) {
        let mut b = Bits { mem: vin, buffer: 0, pos: 0 };
        b.skip(8 * 3); // verMaj, verMin, verSub

        let enc_width = b.read(16).wrapping_shl(4);
        let enc_height = b.read(16).wrapping_shl(4);

        b.skip(24 + 24 + 8 + 8);

        let fps_num = b.read(32);
        let fps_den = b.read(32);

        let fps = fps_num as f32 / fps_den as f32;
        self.frame_time = fps_den as f64 / fps_num as f64;

        b.skip(24 + 24 + 8);

        self.theora.bitrate = b.read(24) / 1000;
        let quality = b.read(6);

        self.granpos_shift = b.read(5);

        h.log(
            LogLevel::Info,
            &format!(
                "OGG Theora Info: {}x{}x{:.1}fps {}kbps {}Q {}G",
                enc_width, enc_height, fps, self.theora.bitrate, quality, self.granpos_shift
            ),
        );
    }
}

/// `OggVorbisSubStream::readComment`: コメント ("ARTIST=..." など) から曲の情報を取る
fn read_comment(vin: &mut MemStream, h: &mut dyn Host) -> Result<Track> {
    let v_len = vin.read_long(); // vendor len
    vin.skip(v_len)?;

    // C++ 版の char argBuf[8192]。前のコメントが残る (読めなかったときは 0 で埋まる)。
    // 長さがちょうど 8192 のとき、C++ 版は終端の NUL をバッファの外に書いていた。
    let mut arg_buf = vec![0u8; 8192 + 1];
    let mut track = Track::default();

    let c_len = vin.read_long(); // comment len
    for _ in 0..c_len.max(0) {
        // 残りが 4 バイト未満なら、それ以降は長さ 0 のコメントを読むだけで何も変わらない。
        // C++ 版はコメント数の値だけ (最大で約 21 億回) 空回りしていた。
        if vin.remaining() < 4 {
            break;
        }
        let l = vin.read_long();
        if !(0..=8192).contains(&l) {
            return fail("Comment string too long");
        }
        let l = l as usize;
        vin.read_into(&mut arg_buf[..l]);
        arg_buf[l] = 0;
        let s = &arg_buf[..arg_buf.iter().position(|&c| c == 0).unwrap()];
        h.log(LogLevel::Info, &format!("OGG Comment: {}", String::from_utf8_lossy(s)));

        let fields: [(&[u8], &mut Option<Vec<u8>>); 5] = [
            (b"ARTIST=", &mut track.artist),
            (b"TITLE=", &mut track.title),
            (b"GENRE=", &mut track.genre),
            (b"CONTACT=", &mut track.contact),
            (b"ALBUM=", &mut track.album),
        ];
        for (key, field) in fields {
            if let Some(p) = stristr(s, key) {
                *field = Some(s[p + key.len()..].to_vec());
                break;
            }
        }
    }

    let frame = vin.read_char(); // framing bit
    if frame == 0 {
        return fail("Bad Comment frame");
    }
    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::super::testhost::{run, Rng, TestHost};
    use super::super::{Error, Kind};
    use super::*;

    /// ページ。`packets` の各パケットを 255 のレーシングで区切る
    fn page(flags: u8, serial: u32, granpos: i64, packets: &[Vec<u8>]) -> Vec<u8> {
        let mut segs = Vec::new();
        let mut body: Vec<u8> = Vec::new();
        for p in packets {
            let mut n = p.len();
            while n >= 255 {
                segs.push(255);
                n -= 255;
            }
            segs.push(n as u8);
            body.extend(p);
        }
        let mut v = b"OggS".to_vec();
        v.push(0);
        v.push(flags);
        v.extend(granpos.to_le_bytes());
        v.extend(serial.to_le_bytes());
        v.extend([0; 8]);
        v.push(segs.len() as u8);
        v.extend(segs);
        v.extend(body);
        v
    }

    fn ident(rate: u32, nominal: u32) -> Vec<u8> {
        let mut v = b"\x01vorbis".to_vec();
        v.extend(0u32.to_le_bytes());
        v.push(2);
        v.extend(rate.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(nominal.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.push(0xb8);
        v.push(1);
        v
    }

    fn comment(comments: &[&str]) -> Vec<u8> {
        let mut v = b"\x03vorbis".to_vec();
        v.extend(3u32.to_le_bytes());
        v.extend(b"xyz");
        v.extend((comments.len() as u32).to_le_bytes());
        for c in comments {
            v.extend((c.len() as u32).to_le_bytes());
            v.extend(c.as_bytes());
        }
        v.push(1);
        v
    }

    fn vorbis_stream() -> Vec<u8> {
        let mut v = page(2, 0x1234, 0, &[ident(44100, 128000)]);
        v.extend(page(0, 0x1234, 0, &[comment(&["title=Song", "Artist=Me", "x=GENRE=Rock"]), b"\x05vorbis-setup".to_vec()]));
        v.extend(page(0, 0x1234, 44100, &[vec![0x11; 300]]));
        v
    }

    #[test]
    fn vorbis() {
        let mut h = TestHost::new(&vorbis_stream());
        assert_eq!(run(Kind::Ogg, &mut h, 4), Err(Error::Abort));
        let head_len = vorbis_stream().len() - (27 + 2 + 300);
        assert_eq!(
            h.events,
            ["ogginfo 128 false".to_string(), format!("head Ogg 0 {}", head_len), format!("data {} 329 cont=false delay=false", head_len), "until 1".into()]
        );
        let t = h.track.unwrap();
        assert_eq!(t.title.as_deref(), Some(&b"Song"[..]));
        assert_eq!(t.artist.as_deref(), Some(&b"Me"[..]));
        assert_eq!(t.genre.as_deref(), Some(&b"Rock"[..]));
        assert_eq!(t.album, None);
    }

    #[test]
    fn bad_serial() {
        let mut v = page(2, 0x1234, 0, &[ident(44100, 128000)]);
        v.extend(page(0, 0x9999, 0, &[vec![1; 10]]));
        assert_eq!(run(Kind::Ogg, &mut TestHost::new(&v), 2), Err(Error::Stream("Bad OGG serial no.".into())));
    }

    #[test]
    fn page_for_inactive_substream() {
        // Theora の BOS のあとに、シリアル番号 0 のページ (Vorbis は bos() されていない)
        let mut v = page(2, 0x1234, 0, &[[b"\x80theora".to_vec(), vec![b'x'; 20]].concat()]);
        v.extend(page(0, 0, 0, &[vec![b'y'; 30]]));
        assert_eq!(run(Kind::Ogg, &mut TestHost::new(&v), 2), Err(Error::Stream("Bad OGG serial no.".into())));
    }

    #[test]
    fn theora_time() {
        let mut o = Ogg { granpos_shift: 6, frame_time: 0.5, ..Default::default() };
        let p = Page { data: vec![], head_len: 0, body_len: 0, gran_pos: (3 << 6) | 5 };
        assert_eq!(o.theora_time(&p), 4.0);
        // 負の値: iframe = -1、pframe = -1 - (-1 << 31) = 2^31 - 1
        o.granpos_shift = 31;
        let p = Page { gran_pos: -1, ..p };
        assert_eq!(o.theora_time(&p), 1073741823.0);
    }

    #[test]
    fn comments() {
        let data = comment(&["ALBUM=a", "contact=http://x"]);
        let mut m = MemStream::new(&data[7..]);
        let t = read_comment(&mut m, &mut TestHost::default()).unwrap();
        assert_eq!(t.album.as_deref(), Some(&b"a"[..]));
        assert_eq!(t.contact.as_deref(), Some(&b"http://x"[..]));
        // 長すぎるコメント
        let mut data = 0u32.to_le_bytes().to_vec();
        data.extend(1u32.to_le_bytes());
        data.extend(8193u32.to_le_bytes());
        assert_eq!(read_comment(&mut MemStream::new(&data), &mut TestHost::default()), Err(Error::Stream("Comment string too long".into())));
        // コメント数が巨大でも空回りしない
        let mut data = 0u32.to_le_bytes().to_vec();
        data.extend(0x7fff_ffffu32.to_le_bytes());
        assert_eq!(read_comment(&mut MemStream::new(&data), &mut TestHost::default()), Err(Error::Stream("Bad Comment frame".into())));
    }

    #[test]
    fn fuzz() {
        let mut rng = Rng(5);
        let mut s = vorbis_stream();
        s.extend(page(2, 77, 0, &[[b"\x80theora".to_vec(), vec![3; 40]].concat()]));
        s.extend(page(0, 77, 0, &[b"\x81theora".to_vec(), b"\x82theora".to_vec()]));
        s.extend(page(0, 77, 1 << 6, &[vec![0; 200]]));
        s.extend(page(4, 0x1234, 88200, &[vec![0; 100]]));
        for _ in 0..3000 {
            let _ = run(Kind::Ogg, &mut TestHost::new(&rng.mutate(&s)), 20);
        }
    }
}
