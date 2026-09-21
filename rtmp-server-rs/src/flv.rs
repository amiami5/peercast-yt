//! FLV への書き出し。C++ 版 (rtmp-server/flvwriter.h) と同じ状態遷移:
//! ファイルヘッダー → スクリプトタグ (メタデータ) → 音声・映像タグ。

use crate::{Error, Result};
use std::io::Write;

const TT_AUDIO: u8 = 8;
const TT_VIDEO: u8 = 9;
const TT_SCRIPT: u8 = 18;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    ExpectFileHeader,
    ExpectScriptTag,
    ExpectDataTag,
}

pub struct FlvWriter<W: Write> {
    out: W,
    state: State,
    previous_tag_size: u32,
}

impl<W: Write> FlvWriter<W> {
    pub fn new(out: W) -> FlvWriter<W> {
        FlvWriter { out, state: State::ExpectFileHeader, previous_tag_size: 0 }
    }

    pub fn into_inner(self) -> W {
        self.out
    }

    pub fn write_file_header(&mut self, audio: bool, video: bool) -> Result<()> {
        if self.state != State::ExpectFileHeader {
            return Err(Error::protocol("writeFileHeader: invalid operation"));
        }
        let flags = ((audio as u8) << 2) | (video as u8);
        let mut h = Vec::with_capacity(9);
        h.extend_from_slice(b"FLV");
        h.push(1);
        h.push(flags);
        h.extend_from_slice(&9u32.to_be_bytes());
        self.out.write_all(&h)?;
        self.state = State::ExpectScriptTag;
        Ok(())
    }

    pub fn write_script_tag(&mut self, timestamp: u32, data: &[u8]) -> Result<()> {
        if self.state != State::ExpectScriptTag {
            return Err(Error::protocol("writeScriptTag: invalid operation"));
        }
        self.write_tag(TT_SCRIPT, timestamp, data)?;
        self.state = State::ExpectDataTag;
        Ok(())
    }

    pub fn write_video_tag(&mut self, timestamp: u32, data: &[u8]) -> Result<()> {
        if self.state != State::ExpectDataTag {
            return Err(Error::protocol("writeVideoTag: invalid operation"));
        }
        self.write_tag(TT_VIDEO, timestamp, data)
    }

    pub fn write_audio_tag(&mut self, timestamp: u32, data: &[u8]) -> Result<()> {
        if self.state != State::ExpectDataTag {
            return Err(Error::protocol("writeAudioTag: invalid operation"));
        }
        self.write_tag(TT_AUDIO, timestamp, data)
    }

    /// タグ 1 個 (直前のタグのサイズ + ヘッダー + 本体) を 1 回の write で出す。
    fn write_tag(&mut self, tag_type: u8, timestamp: u32, data: &[u8]) -> Result<()> {
        if data.len() > 0xff_ffff {
            return Err(Error::protocol("FLV tag too large"));
        }
        let mut buf = Vec::with_capacity(4 + 11 + data.len());
        buf.extend_from_slice(&self.previous_tag_size.to_be_bytes());
        buf.push(tag_type);
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        // タイムスタンプは下位 24 ビット + 拡張 8 ビットの順。
        buf.extend_from_slice(&timestamp.to_be_bytes()[1..]);
        buf.push((timestamp >> 24) as u8);
        buf.extend_from_slice(&[0, 0, 0]); // StreamID
        buf.extend_from_slice(data);
        self.out.write_all(&buf)?;
        self.previous_tag_size = data.len() as u32 + 11;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_tags() {
        let mut w = FlvWriter::new(Vec::new());
        assert!(w.write_script_tag(0, b"x").is_err()); // ヘッダーより先は不可
        w.write_file_header(true, true).unwrap();
        w.write_script_tag(0, b"meta").unwrap();
        w.write_video_tag(0x0102_0304, b"v").unwrap();
        let out = w.into_inner();
        assert_eq!(&out[..13], &[b'F', b'L', b'V', 1, 0x05, 0, 0, 0, 9, 0, 0, 0, 0]);
        // 動画タグ: 直前のタグサイズ 4+11=15, type 9, size 1, ts 下位 02 03 04, 拡張 01
        let tail = &out[out.len() - (4 + 11 + 1)..];
        assert_eq!(tail, &[0, 0, 0, 15, 9, 0, 0, 1, 2, 3, 4, 1, 0, 0, 0, b'v']);
    }

    #[test]
    fn header_only_once() {
        let mut w = FlvWriter::new(Vec::new());
        w.write_file_header(false, false).unwrap();
        assert!(w.write_file_header(false, false).is_err());
    }
}
