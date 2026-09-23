//! FLV (core/common/flv.cpp の `FLVStream`, `FLVTag`, `FLVTagBuffer`)。
//!
//! ファイルヘッダーと、onMetaData・最初の映像タグ (AVC のヘッダー)・最初の音声タグ (AAC の
//! ヘッダー) をまとめてヘッダーパケットにする。それ以外のタグは、キーフレームで区切って
//! まとめたり分けたりしながらデータパケットにする。

use super::{fail, read_into, HeadKind, Host, LogLevel, MemStream, Result, MAX_DATALEN};
use crate::amf0::{self, Builder, ObjectKind};

pub const T_AUDIO: u8 = 8;
pub const T_VIDEO: u8 = 9;
pub const T_SCRIPT: u8 = 18;

/// `FLVTagBuffer::MAX_OUTGOING_PACKET_SIZE`
pub const MAX_OUTGOING_PACKET_SIZE: usize = 15 * 1024;
/// `FLVTagBuffer::FLUSH_THRESHOLD`
pub const FLUSH_THRESHOLD: usize = 4 * 1024;

/// FLV のタグ 1 つ (11 バイトのヘッダー、データ、4 バイトの PreviousTagSize)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub packet: Vec<u8>,
}

impl Tag {
    /// `FLVTag::read`。足りない分は 0 のまま。
    fn read(h: &mut dyn Host) -> Result<Tag> {
        let mut binary = [0u8; 11];
        read_into(h, &mut binary)?;
        let size = u32::from_be_bytes([0, binary[1], binary[2], binary[3]]) as usize;
        let mut packet = vec![0u8; 11 + size + 4];
        packet[..11].copy_from_slice(&binary);
        read_into(h, &mut packet[11..])?;
        Ok(Tag { packet })
    }

    pub fn tag_type(&self) -> u8 {
        self.packet[0]
    }

    /// データの長さ (ヘッダーの値)
    pub fn size(&self) -> usize {
        self.packet.len() - 15
    }

    pub fn data(&self) -> &[u8] {
        &self.packet[11..11 + self.size()]
    }

    /// `getTimestamp`: 下位 24 ビットのあとに拡張 8 ビット (上位) が続く
    pub fn timestamp(&self) -> i32 {
        timestamp_of(&self.packet)
    }

    fn clear_timestamp(&mut self) {
        self.packet[4..8].fill(0);
    }

    /// `isKeyFrame`: 映像タグで、フレームの種類がキーフレーム (1) か生成されたキーフレーム (4)
    pub fn is_key_frame(&self) -> bool {
        if self.tag_type() != T_VIDEO {
            return false;
        }
        let frame_type = self.packet[11] >> 4;
        frame_type == 1 || frame_type == 4
    }
}

fn timestamp_of(packet: &[u8]) -> i32 {
    i32::from_be_bytes([packet[7], packet[4], packet[5], packet[6]])
}

/// `FLVStream::readMetaData`: スクリプトタグのペイロードが onMetaData なら、videodatarate と
/// audiodatarate の和 (切り上げ) を返す。onMetaData でないか、どちらのプロパティもなければ
/// (または形式が壊れていれば) `None`。
///
/// C++ 版は和を `int` に変換するとき範囲を確かめておらず、2^31 以上の値が CPU によって違う値に
/// なっていた (x86 では負の数)。Rust 版は `int` の範囲に収める。
pub fn read_meta_data(data: &[u8]) -> std::result::Result<Option<i32>, String> {
    let mut mem = MemStream::new(data);

    let mut cmd = Meta::default();
    amf0::read_value(&mut mem, &mut cmd).map_err(amf0_message)?;
    match cmd.top {
        Top::String(s) if s == b"onMetaData" => {}
        Top::String(_) => return Ok(None),
        _ => return Err("not a string".into()),
    }

    let mut obj = Meta::default();
    amf0::read_value(&mut mem, &mut obj).map_err(amf0_message)?;
    if obj.top != Top::Object {
        return Err("not an object or an array".into());
    }

    // 同じキーが何度もあれば最後の値 (std::map への代入と同じ)
    let lookup = |key: &[u8]| obj.entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| *v);
    let mut bitrate = 0.0;
    for key in [&b"videodatarate"[..], b"audiodatarate"] {
        match lookup(key) {
            Some(Some(n)) => bitrate += n,
            Some(None) => return Err("not a number".into()),
            None => {}
        }
    }
    if bitrate > 0.0 {
        Ok(Some(f64::ceil(bitrate) as i32))
    } else {
        Ok(None)
    }
}

fn amf0_message(e: amf0::Error) -> String {
    match e {
        amf0::Error::Abort => "Stream::read: premature end of stream".into(),
        amf0::Error::TooDeep => "AMF0: nesting too deep".into(),
        amf0::Error::TooMany => "AMF0: too many values".into(),
        amf0::Error::UnknownType(t) => format!("unknown AMF value type {}", t),
    }
}

/// AMF0 の値のうち、`readMetaData` が見る部分だけを覚える
#[derive(Default)]
struct Meta {
    depth: usize,
    top: Top,
    key: Vec<u8>,
    /// 一番外側のオブジェクトの (キー, 数値ならその値)
    entries: Vec<(Vec<u8>, Option<f64>)>,
}

#[derive(Default, PartialEq, Debug)]
enum Top {
    #[default]
    None,
    String(Vec<u8>),
    Object,
    Other,
}

impl Meta {
    fn leaf(&mut self, top: Top, number: Option<f64>) {
        match self.depth {
            0 => self.top = top,
            1 if self.top == Top::Object => {
                let key = std::mem::take(&mut self.key);
                self.entries.push((key, number));
            }
            _ => {}
        }
    }

    fn begin(&mut self, top: Top) {
        self.leaf(top, None);
        self.depth += 1;
    }
}

impl Builder for Meta {
    fn number(&mut self, v: f64) {
        self.leaf(Top::Other, Some(v));
    }
    fn string(&mut self, s: &[u8]) {
        self.leaf(Top::String(s.to_vec()), None);
    }
    fn boolean(&mut self, _: bool) {
        self.leaf(Top::Other, None);
    }
    fn null(&mut self) {
        self.leaf(Top::Other, None);
    }
    fn date(&mut self, _: f64, _: u16) {
        self.leaf(Top::Other, None);
    }
    fn begin_object(&mut self, _: ObjectKind) {
        self.begin(Top::Object);
    }
    fn key(&mut self, k: &[u8]) {
        if self.depth == 1 {
            self.key = k.to_vec();
        }
    }
    fn end_object(&mut self) {
        self.depth -= 1;
    }
    fn begin_strict_array(&mut self) {
        self.begin(Top::Other);
    }
    fn end_strict_array(&mut self) {
        self.depth -= 1;
    }
}

/// `FLVTagBuffer`: キーフレームでないタグを溜めて、まとめて送る
#[derive(Default)]
struct TagBuffer {
    mem: Vec<u8>,
    stream_has_key_frames: bool,
    start_time: f64,
}

impl TagBuffer {
    /// `put`: パケットを送ったら true
    fn put(&mut self, tag: &Tag, h: &mut dyn Host) -> Result<bool> {
        let size = tag.packet.len();
        if tag.is_key_frame() {
            self.stream_has_key_frames = true;
            if !self.mem.is_empty() {
                self.flush(h)?;
            }
            self.send_immediately(tag, h)?;
            Ok(true)
        } else if self.mem.len() + size > MAX_OUTGOING_PACKET_SIZE {
            self.flush(h)?;
            self.send_immediately(tag, h)?;
            Ok(true)
        } else if self.mem.len() + size > FLUSH_THRESHOLD {
            self.flush(h)?;
            self.mem.extend_from_slice(&tag.packet);
            if self.mem.len() > FLUSH_THRESHOLD {
                self.flush(h)?;
            }
            Ok(true)
        } else {
            self.mem.extend_from_slice(&tag.packet);
            Ok(false)
        }
    }

    /// `rateLimit`: タイムスタンプの時刻まで待つ
    fn rate_limit(&mut self, timestamp: u32, h: &mut dyn Host) {
        let diff = (self.start_time + timestamp as f64 / 1000.0) - h.dtime();
        if diff > 10.0 {
            // 10秒は長すぎるので、タイムスタンプがジャンプしてるっぽい。基準時刻をリセット。
            h.log(LogLevel::Debug, "Timestamp way into the future. Resetting referece point.");
            self.start_time = h.dtime();
        } else if diff < -10.0 {
            h.log(LogLevel::Debug, "Timestamp way back in the past. Resetting referece point.");
            self.start_time = h.dtime();
        } else if diff > 0.0 {
            h.log(LogLevel::Trace, &format!("Sleeping {:.2} s", diff));
            h.sleep((diff * 1000.0) as i32);
        }
    }

    fn send_immediately(&mut self, tag: &Tag, h: &mut dyn Host) -> Result<()> {
        if h.read_delay() {
            self.rate_limit(tag.timestamp() as u32, h);
        }
        let mut cont = false;
        for (i, chunk) in tag.packet.chunks(MAX_OUTGOING_PACKET_SIZE).enumerate() {
            if self.stream_has_key_frames {
                cont = !(i == 0 && tag.is_key_frame());
            }
            h.packet(chunk, cont, false)?;
        }
        Ok(())
    }

    fn flush(&mut self, h: &mut dyn Host) -> Result<()> {
        if self.mem.is_empty() {
            return Ok(());
        }
        if h.read_delay() {
            // 溜めた先頭のタグのタイムスタンプ
            self.rate_limit(timestamp_of(&self.mem) as u32, h);
        }
        // キーフレームでないタグだけがバッファリングされる。
        let mem = std::mem::take(&mut self.mem);
        h.packet(&mem, self.stream_has_key_frames, false)?;
        Ok(())
    }
}

#[derive(Default)]
pub struct Flv {
    meta_bitrate: i32,
    /// `fileHeader` (readHeader を呼ぶまでは `None`)
    file_header: Option<[u8; 13]>,
    meta_data: Option<Tag>,
    aac_header: Option<Tag>,
    avc_header: Option<Tag>,
    buffer: TagBuffer,
}

impl Flv {
    pub fn read_header(&mut self, h: &mut dyn Host) -> Result<()> {
        self.meta_bitrate = 0;
        // 足りない分は前の内容のまま (FLVFileHeader::read)
        let mut data = self.file_header.unwrap_or([0; 13]);
        read_into(h, &mut data)?;
        self.file_header = Some(data);
        self.buffer.start_time = h.dtime();
        Ok(())
    }

    /// C++ 版は、パケットを送らなかったとき、続きが読めるなら readPacket を再帰呼び出ししていた。
    /// Rust 版は同じことをループで行う。
    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        loop {
            if self.read_tag(h)? {
                return Ok(());
            }
            if !h.ready()? {
                return Ok(());
            }
        }
    }

    /// タグを 1 つ読んで処理する。パケットを送ったら (またはヘッダーを更新したら) true。
    fn read_tag(&mut self, h: &mut dyn Host) -> Result<bool> {
        let mut header_update = false;
        let tag = Tag::read(h)?;

        match tag.tag_type() {
            T_SCRIPT => match read_meta_data(tag.data()) {
                Ok(Some(bitrate)) => {
                    self.meta_bitrate = bitrate;
                    self.meta_data = Some(tag.clone());
                    header_update = true;
                }
                Ok(None) => {}
                Err(e) => h.log(LogLevel::Error, &format!("readMetaData: {}", e)),
            },
            T_VIDEO => {
                // AVC Header
                if self.avc_header.is_none() {
                    h.log(LogLevel::Debug, "Got AVC header");
                    let mut t = tag.clone();
                    if t.timestamp() != 0 {
                        h.log(LogLevel::Info, "AVC header has non-zero timestamp. Cleared to zero.");
                        t.clear_timestamp();
                    }
                    self.avc_header = Some(t);
                    header_update = true;
                }
            }
            T_AUDIO => {
                // AAC Header
                if self.aac_header.is_none() {
                    h.log(LogLevel::Debug, "Got AAC header");
                    let mut t = tag.clone();
                    if t.timestamp() != 0 {
                        h.log(LogLevel::Info, "AAC header has non-zero timestamp. Cleared to zero.");
                        t.clear_timestamp();
                    }
                    self.aac_header = Some(t);
                    header_update = true;
                }
            }
            _ => h.log(LogLevel::Error, "Invalid FLV tag!"),
        }

        // メタ情報からのビットレートが無い場合、ストリームからの実測値が現在の公称値を超えていれば
        // 公称値を更新する。
        if self.meta_bitrate == 0 {
            h.raise_bitrate()?;
        }

        match (header_update, &self.file_header) {
            (true, Some(file_header)) => {
                let parts = [&self.meta_data, &self.avc_header, &self.aac_header];
                let len = 13 + parts.iter().filter_map(|t| t.as_ref()).map(|t| t.packet.len()).sum::<usize>();
                // headPack.data の容量を超えるとバッファオーバーフローになる。
                if len > MAX_DATALEN {
                    return fail("head packet too large");
                }
                let mut head = file_header.to_vec();
                for t in parts.iter().filter_map(|t| t.as_ref()) {
                    head.extend_from_slice(&t.packet);
                }

                // メタ情報からのビットレートがあればその値を設定。無ければ、前回のエンコード
                // セッションからの値をクリアするために 0 を設定する。
                h.set_bitrate(self.meta_bitrate)?;
                self.buffer.flush(h)?;
                h.head(HeadKind::NewStream, &head)?;
                Ok(true)
            }
            _ => self.buffer.put(&tag, h),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testhost::{run, Rng, TestHost};
    use super::super::{Error, Kind};
    use super::*;

    fn amf_string(s: &str) -> Vec<u8> {
        let mut v = vec![2];
        v.extend((s.len() as u16).to_be_bytes());
        v.extend(s.as_bytes());
        v
    }

    fn amf_object(pairs: &[(&str, f64)]) -> Vec<u8> {
        let mut v = vec![3];
        for (k, n) in pairs {
            v.extend((k.len() as u16).to_be_bytes());
            v.extend(k.as_bytes());
            v.push(0);
            v.extend(n.to_be_bytes());
        }
        v.extend([0, 0, 9]);
        v
    }

    fn meta(pairs: &[(&str, f64)]) -> Vec<u8> {
        let mut v = amf_string("onMetaData");
        v.extend(amf_object(pairs));
        v
    }

    fn tag(ty: u8, ts: u32, body: &[u8]) -> Vec<u8> {
        let n = body.len() as u32;
        let mut t = vec![ty, (n >> 16) as u8, (n >> 8) as u8, n as u8, (ts >> 16) as u8, (ts >> 8) as u8, ts as u8, (ts >> 24) as u8, 0, 0, 0];
        t.extend(body);
        t.extend((n + 11).to_be_bytes());
        t
    }

    const FILE_HEADER: &[u8] = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00";

    #[test]
    fn meta_data() {
        assert_eq!(read_meta_data(&meta(&[("videodatarate", 100.0), ("audiodatarate", 50.0)])), Ok(Some(150)));
        assert_eq!(read_meta_data(&meta(&[("videodatarate", 100.0)])), Ok(Some(100)));
        assert_eq!(read_meta_data(&meta(&[("audiodatarate", 50.0)])), Ok(Some(50)));
        assert_eq!(read_meta_data(&meta(&[("audiodatarate", 0.5)])), Ok(Some(1)));
        assert_eq!(read_meta_data(&meta(&[])), Ok(None));
        let mut v = amf_string("hoge");
        v.extend(amf_object(&[("videodatarate", 100.0)]));
        assert_eq!(read_meta_data(&v), Ok(None));
        // 同じキーは最後の値
        assert_eq!(read_meta_data(&meta(&[("videodatarate", 100.0), ("videodatarate", 7.0)])), Ok(Some(7)));
        // 範囲を超える値は int の最大値
        assert_eq!(read_meta_data(&meta(&[("videodatarate", 1e300)])), Ok(Some(i32::MAX)));
        // キーの長さがデータの残りより大きい
        let mut v = amf_string("onMetaData");
        v.extend(b"\x03\x00\x0dshort");
        assert!(read_meta_data(&v).is_err());
        // 数値でない
        let mut v = amf_string("onMetaData");
        v.extend(b"\x03\x00\x0dvideodatarate\x02\x00\x01a\x00\x00\x09");
        assert_eq!(read_meta_data(&v), Err("not a number".into()));
    }

    #[test]
    fn header_then_buffered_tags() {
        let mut s = FILE_HEADER.to_vec();
        s.extend(tag(T_SCRIPT, 0, &meta(&[("videodatarate", 100.0)])));
        s.extend(tag(T_VIDEO, 5, &[0x17, 0, 0, 0, 0])); // AVC ヘッダー (タイムスタンプを 0 にする)
        s.extend(tag(T_AUDIO, 0, &[0xaf, 0]));
        s.extend(tag(T_VIDEO, 40, &[0x17, 1, 2, 3])); // キーフレーム
        s.extend(tag(T_VIDEO, 80, &[0x27, 1]));
        s.extend(tag(T_AUDIO, 81, &[0xaf, 1]));
        let mut h = TestHost::new(&s);
        // 5 回目の readPacket は、最後の 2 つのタグを溜めたまま (続きが読めないので) 戻る
        assert_eq!(run(Kind::Flv, &mut h, 5), Ok(()));
        assert_eq!(
            h.events,
            [
                "bitrate 100",
                "head NewStream 0 69",
                "bitrate 100",
                "head NewStream 0 89",
                "bitrate 100",
                "head NewStream 0 106",
                "data 106 19 cont=false delay=false",
            ]
        );
        // ヘッダーパケットの中の AVC ヘッダー (13 + 56 バイト目から)。タイムスタンプは 0 になる
        assert_eq!(&h.packets[2][69..77], &[9, 0, 0, 5, 0, 0, 0, 0][..]);
    }

    #[test]
    fn key_frames_split_packets() {
        let mut s = FILE_HEADER.to_vec();
        s.extend(tag(T_VIDEO, 0, &[0x17, 0])); // AVC ヘッダー
        s.extend(tag(T_VIDEO, 40, &[0x27; 10]));
        s.extend(tag(T_VIDEO, 80, &[0x17; 20000])); // キーフレーム (15 KiB で分かれる)
        s.extend(tag(T_VIDEO, 120, &[0x27; 10]));
        let mut h = TestHost::new(&s);
        h.read_delay = true;
        assert_eq!(run(Kind::Flv, &mut h, 10), Err(Error::Abort));
        // キーフレームが来ると、溜めていたタグを (続きのパケットとして) 先に送る。
        // readDelay なので、送る前にタイムスタンプの時刻まで待つ。
        assert_eq!(
            h.events,
            [
                "bitrate 0",
                "head NewStream 0 30",
                "sleep 40",
                "data 30 25 cont=true delay=false",
                "sleep 80",
                "data 55 15360 cont=false delay=false",
                "data 15415 4655 cont=true delay=false",
            ]
        );
    }

    #[test]
    fn head_packet_too_large() {
        let mut m = meta(&[("videodatarate", 100.0)]);
        m.extend(vec![0; 17000]);
        let mut s = FILE_HEADER.to_vec();
        s.extend(tag(T_SCRIPT, 0, &m));
        assert_eq!(run(Kind::Flv, &mut TestHost::new(&s), 1), Err(Error::Stream("head packet too large".into())));
    }

    #[test]
    fn fuzz() {
        let mut rng = Rng(7);
        let mut s = FILE_HEADER.to_vec();
        s.extend(tag(T_SCRIPT, 0, &meta(&[("videodatarate", 100.0), ("audiodatarate", 3.0)])));
        s.extend(tag(T_VIDEO, 0, &[0x17, 0]));
        s.extend(tag(T_AUDIO, 0, &[0xaf, 0]));
        for i in 0..20 {
            s.extend(tag(if i % 3 == 0 { T_AUDIO } else { T_VIDEO }, i * 33, &vec![if i % 7 == 0 { 0x17 } else { 0x27 }; 100 + i as usize * 700]));
        }
        for _ in 0..3000 {
            let mut h = TestHost::new(&rng.mutate(&s));
            h.read_delay = rng.below(2) == 0;
            let _ = run(Kind::Flv, &mut h, 50);
        }
    }
}
