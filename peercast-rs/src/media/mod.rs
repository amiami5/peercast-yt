//! メディアコンテナの解析 (core/common の flv, mkv, ogg, mp3, mp4 の `ChannelStream`)。
//!
//! 各解析器は、入力の `Stream` から読んでパケットに切り分け、チャンネルに流す。チャンネル
//! (`Channel` クラス) とのやりとりは `Host` トレイトのメソッドだけで行う。C++ からは
//! `ffi::CMediaHost` (C の型は `pcrs_media_host`) がこれを実装し、`Channel` のメンバーを
//! C++ 版と同じ順序で読み書きする。
//!
//! C++ 版が初期化していないメモリを読んでいた箇所 (読み出しが足りなかったときのバッファの
//! 残りなど) は、どれも 0 として扱う。

pub mod flv;
pub mod mkv;
pub mod mp3;
pub mod mp4;
pub mod ogg;

use crate::reader::{Abort, Reader};

/// `ChanPacket::MAX_DATALEN`
pub const MAX_DATALEN: usize = 16384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// C++ のコールバックの中で例外が起きた (C++ 側で投げ直す)
    Abort,
    /// C++ 版が `StreamException` (MKV では `std::runtime_error` も) を投げていたところ。
    /// C++ 側で同じメッセージの `StreamException` にする。
    Stream(String),
}

impl From<Abort> for Error {
    fn from(_: Abort) -> Self {
        Error::Abort
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn fail<T>(msg: &str) -> Result<T> {
    Err(Error::Stream(msg.to_string()))
}

/// ヘッダーパケットの送り方 (`Host::head`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadKind {
    /// FLV と MP4: `rawData.init()`、`streamIndex++` のあと、`headPack` を位置 0 で送り、
    /// `streamPos` をその長さにする。
    NewStream = 0,
    /// MKV: `streamIndex++`、`rawData.init()`、`streamPos = 0` のあと、新しいパケットを
    /// `headPack` に代入してから送り、`streamPos` を進める。
    Mkv = 1,
    /// OGG: `head_append` で溜めた `headPack` を、現在の `streamPos` の位置で送る
    /// (`startTime` を今の時刻にする)。`data` は使わない。
    Ogg = 2,
}

/// ログの重要度 (C++ の `LOG_TRACE` などに対応する)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

/// OGG Vorbis のコメントから取った曲の情報 (`ChanInfo::track`)。`None` は空のまま。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Track {
    pub artist: Option<Vec<u8>>,
    pub title: Option<Vec<u8>>,
    pub genre: Option<Vec<u8>>,
    pub contact: Option<Vec<u8>>,
    pub album: Option<Vec<u8>>,
}

/// 解析器から見たチャンネルと入力。`Reader` のメソッドは入力の `Stream` を読む。
pub trait Host: Reader {
    /// `in.readReady()`
    fn ready(&mut self) -> std::result::Result<bool, Abort>;
    /// 入力の実測のビットレート (`in.stat.bytesInPerSecAvg() / 1000 * 8`) がチャンネルの
    /// 公称値を超えていれば、公称値を更新する (`updateInfo`)。
    fn raise_bitrate(&mut self) -> std::result::Result<(), Abort>;
    /// データパケットを現在の `streamPos` の位置で送り (`newPacket`)、`streamPos` を進める。
    /// `read_delay` なら、送ったあと `checkReadDelay` を呼ぶ (MP3)。
    fn packet(&mut self, data: &[u8], cont: bool, read_delay: bool) -> std::result::Result<(), Abort>;
    /// ヘッダーパケットを送る。
    fn head(&mut self, kind: HeadKind, data: &[u8]) -> std::result::Result<(), Abort>;
    /// `headPack.len`
    fn head_len(&mut self) -> u32;
    /// `headPack.len = 0`
    fn head_clear(&mut self);
    /// `headPack` の後ろに付け足す (呼ぶ側が `MAX_DATALEN` 未満に収まることを確かめる)。
    fn head_append(&mut self, data: &[u8]) -> std::result::Result<(), Abort>;
    /// チャンネル情報のビットレートを `updateInfo` で変える (FLV)。
    fn set_bitrate(&mut self, bitrate: i32) -> std::result::Result<(), Abort>;
    /// `info.bitrate` を直接書き換え、`ogm` なら `info.contentType` を `T_OGM` にする (OGG)。
    fn ogg_set_info(&mut self, bitrate: i32, ogm: bool);
    /// `info.track` を空にしてから `track` の値を入れ、`updateInfo` する (OGG Vorbis のコメント)。
    fn set_track(&mut self, track: &Track) -> std::result::Result<(), Abort>;
    /// `processMp3Metadata` (NUL で終わる ICY メタデータ)
    fn mp3_metadata(&mut self, buf: &[u8]) -> std::result::Result<(), Abort>;
    /// `icyMetaInterval`
    fn icy_meta_interval(&mut self) -> i32;
    /// `readDelay`
    fn read_delay(&mut self) -> bool;
    /// `sys->getDTime()`
    fn dtime(&mut self) -> f64;
    /// `sys->getTime()`
    fn time(&mut self) -> u32;
    /// `sys->sleep(ms)`
    fn sleep(&mut self, ms: i32);
    /// `Channel::sleepUntil`
    fn sleep_until(&mut self, t: f64);
    fn log(&mut self, level: LogLevel, msg: &str);
}

/// 入力から最大 `buf.len()` バイト読み (`Stream::read(void*, int)`)、`buf` の先頭に書く。
/// 足りなかった残りは書き換えない (C++ 版と同じ)。
pub(crate) fn read_into<R: Reader + ?Sized>(r: &mut R, buf: &mut [u8]) -> std::result::Result<(), Abort> {
    let got = r.read_some(buf.len())?;
    let n = got.len().min(buf.len());
    buf[..n].copy_from_slice(&got[..n]);
    Ok(())
}

/// `Stream::read(int)`: 4096 バイトずつ `read(void*, int)` を呼んで、ちょうど `n` バイト読む。
/// 1 回も読めなければ例外 (C++ 版と同じ処理を Rust で行う。大きな要素でも、届いた分しか
/// メモリを確保しない)。
pub(crate) fn read_exact<R: Reader + ?Sized>(r: &mut R, n: usize) -> Result<Vec<u8>> {
    let mut res = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        let chunk = r.read_some(remaining.min(4096))?;
        if chunk.is_empty() {
            return fail("Stream::read: premature end of stream");
        }
        remaining -= chunk.len().min(remaining);
        res.extend_from_slice(&chunk);
    }
    Ok(res)
}

/// C++ の `MemoryStream` からの読み出し。足りないときは例外を投げず、読み先を 0 で埋めて
/// 0 を返す (`readChar` は 0、`readLong` は 0 になる)。
pub(crate) struct MemStream<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> MemStream<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        MemStream { buf, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// `MemoryStream::read(void*, int)`: 全部読めるか、何も読まない (0 で埋める) か。
    pub fn read_into(&mut self, out: &mut [u8]) -> usize {
        let n = out.len();
        if n <= self.remaining() {
            out.copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            n
        } else {
            out.fill(0);
            0
        }
    }

    pub fn read_char(&mut self) -> u8 {
        let mut b = [0u8; 1];
        self.read_into(&mut b);
        b[0]
    }

    /// `Stream::readLong` を `int` で受けたもの (リトルエンディアンの 4 バイト)。
    ///
    /// C++ 版は CPU のバイト順でメモリに読んでいたので、ビッグエンディアンの CPU では値が
    /// 違った。Rust 版は、どの CPU でもリトルエンディアンの CPU での C++ 版と同じ値になる。
    pub fn read_long(&mut self) -> i32 {
        let mut b = [0u8; 4];
        self.read_into(&mut b);
        i32::from_le_bytes(b)
    }

    /// `Stream::skip`: 4096 バイトずつ読み捨てる (読めなかった塊は位置が進まない)。
    pub fn skip(&mut self, n: i32) -> Result<()> {
        if n < 0 {
            return fail("Stream::skip: negative length");
        }
        let mut len = n as usize;
        let mut tmp = [0u8; 4096];
        while len > 0 {
            let r = len.min(tmp.len());
            self.read_into(&mut tmp[..r]);
            len -= r;
        }
        Ok(())
    }

    /// `Stream::read(int)`: 4096 バイトずつ読み、読めない塊があれば例外。
    pub fn read_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        let mut remaining = n;
        while remaining > 0 {
            let size = remaining.min(4096);
            if size > self.remaining() {
                return fail("Stream::read: premature end of stream");
            }
            res.extend_from_slice(&self.buf[self.pos..self.pos + size]);
            self.pos += size;
            remaining -= size;
        }
        Ok(res)
    }
}

/// AMF0 のデシリアライザ (FLV のメタデータ) に `MemoryStream` として渡す。
impl Reader for MemStream<'_> {
    fn read_char(&mut self) -> std::result::Result<u8, Abort> {
        Ok(MemStream::read_char(self))
    }

    fn read_exact(&mut self, n: usize) -> std::result::Result<Vec<u8>, Abort> {
        MemStream::read_exact(self, n).map_err(|_| Abort)
    }

    fn read_some(&mut self, n: usize) -> std::result::Result<Vec<u8>, Abort> {
        let mut v = vec![0u8; n];
        let got = self.read_into(&mut v);
        v.truncate(got);
        Ok(v)
    }

    fn eof(&mut self) -> std::result::Result<bool, Abort> {
        Ok(self.pos >= self.buf.len())
    }
}

/// 解析器の種類 (C の `PCRS_MEDIA_*`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mp3 = 1,
    Flv = 2,
    Ogg = 3,
    Mkv = 4,
    Mp4 = 5,
}

impl Kind {
    pub fn from_i32(v: i32) -> Option<Kind> {
        match v {
            1 => Some(Kind::Mp3),
            2 => Some(Kind::Flv),
            3 => Some(Kind::Ogg),
            4 => Some(Kind::Mkv),
            5 => Some(Kind::Mp4),
            _ => None,
        }
    }
}

/// 1 本の入力を解析する状態 (C++ の `ChannelStream` の派生クラス 1 つ分)
pub enum Parser {
    Mp3(mp3::Mp3),
    Flv(Box<flv::Flv>),
    Ogg(Box<ogg::Ogg>),
    Mkv(mkv::Mkv),
    Mp4(mp4::Mp4),
}

impl Parser {
    pub fn new(kind: Kind) -> Parser {
        match kind {
            Kind::Mp3 => Parser::Mp3(mp3::Mp3),
            Kind::Flv => Parser::Flv(Box::default()),
            Kind::Ogg => Parser::Ogg(Box::default()),
            Kind::Mkv => Parser::Mkv(mkv::Mkv::default()),
            Kind::Mp4 => Parser::Mp4(mp4::Mp4),
        }
    }

    /// `readHeader`
    pub fn read_header(&mut self, h: &mut dyn Host) -> Result<()> {
        match self {
            Parser::Mp3(_) => Ok(()),
            Parser::Flv(p) => p.read_header(h),
            Parser::Ogg(_) => Ok(()),
            Parser::Mkv(p) => p.read_header(h),
            Parser::Mp4(p) => p.read_header(h),
        }
    }

    /// `readPacket`
    pub fn read_packet(&mut self, h: &mut dyn Host) -> Result<()> {
        match self {
            Parser::Mp3(p) => p.read_packet(h),
            Parser::Flv(p) => p.read_packet(h),
            Parser::Ogg(p) => p.read_packet(h),
            Parser::Mkv(p) => p.read_packet(h),
            Parser::Mp4(p) => p.read_packet(h),
        }
    }
}

#[cfg(test)]
pub(crate) mod testhost;
