//! `Stream` (core/common/stream.h / stream.cpp、sstream.cpp) と、その上の読み書きの関数。
//!
//! C++ 版と同じく、`read` の意味は実装ごとに違う: ソケットは要求した長さを全部読むか例外、
//! `StringStream` は読めた分だけ、`FileStream` は 1 バイト以上読めた分だけ。

use std::sync::Mutex;

use super::error::{Error, Result};
use super::sys;

/// `Stream::Stat`: 読み書きしたバイト数と、1 秒あたりの量
#[derive(Debug, Default)]
pub struct Stat {
    inner: Mutex<StatInner>,
}

#[derive(Debug, Default, Clone, Copy)]
struct StatInner {
    total_in: u32,
    total_out: u32,
    last_in: u32,
    last_out: u32,
    in_per_sec: u32,
    out_per_sec: u32,
    in_avg: f64,
    out_avg: f64,
    last_update: f64,
    start_time: f64,
}

impl StatInner {
    fn update(&mut self, i: u32, o: u32) {
        const EXP: f64 = 9.0 / 10.0;
        let now = sys::get_dtime();
        if self.last_update == 0.0 {
            self.start_time = now;
            self.last_update = now;
        }
        self.total_in = self.total_in.wrapping_add(i);
        self.total_out = self.total_out.wrapping_add(o);
        let tdiff = now - self.last_update;
        if tdiff >= 1.0 {
            self.in_per_sec = (self.total_in.wrapping_sub(self.last_in) as f64 / tdiff) as u32;
            self.out_per_sec = (self.total_out.wrapping_sub(self.last_out) as f64 / tdiff) as u32;
            self.in_avg = EXP * self.in_avg + (1.0 - EXP) * self.in_per_sec as f64;
            self.out_avg = EXP * self.out_avg + (1.0 - EXP) * self.out_per_sec as f64;
            self.last_in = self.total_in;
            self.last_out = self.total_out;
            self.last_update = now;
        }
    }
}

impl Stat {
    pub fn update(&self, i: u32, o: u32) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).update(i, o);
    }

    fn get<R>(&self, f: impl FnOnce(&StatInner) -> R) -> R {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.update(0, 0);
        f(&s)
    }

    pub fn total_bytes_in(&self) -> u32 {
        self.get(|s| s.total_in)
    }
    pub fn total_bytes_out(&self) -> u32 {
        self.get(|s| s.total_out)
    }
    pub fn last_bytes_in(&self) -> u32 {
        self.get(|s| s.last_in)
    }
    pub fn last_bytes_out(&self) -> u32 {
        self.get(|s| s.last_out)
    }
    pub fn bytes_in_per_sec(&self) -> u32 {
        self.get(|s| s.in_per_sec)
    }
    pub fn bytes_out_per_sec(&self) -> u32 {
        self.get(|s| s.out_per_sec)
    }
    /// `bytesInPerSecAvg` (`double` を `unsigned int` にする)
    pub fn bytes_in_per_sec_avg(&self) -> u32 {
        self.get(|s| s.in_avg as u32)
    }
    pub fn bytes_out_per_sec_avg(&self) -> u32 {
        self.get(|s| s.out_avg as u32)
    }
}

/// `Stream`
pub trait Stream: Send {
    /// `read(void*, int)`
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// `readUpto`: 読めるだけ読む (相手が閉じたら短くなる)
    fn read_upto(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn eof(&mut self) -> Result<bool> {
        Err(Error::stream("Stream can`t eof"))
    }
    fn rewind(&mut self) -> Result<()> {
        Err(Error::stream("Stream can`t rewind"))
    }
    fn seek_to(&mut self, _pos: i32) -> Result<()> {
        Err(Error::stream("Stream can`t seek"))
    }
    fn close(&mut self) {}
    fn set_read_timeout(&mut self, _ms: u32) {
        crate::log_warn!("Stream::setReadTimeout null implementation called");
    }
    fn set_write_timeout(&mut self, _ms: u32) {
        crate::log_warn!("Stream::setWriteTimeout null implementation called");
    }
    fn position(&mut self) -> i32 {
        0
    }
    /// `readReady`: `ms` ミリ秒以内に読めるようになるか
    fn read_ready(&mut self, _ms: u32) -> bool {
        true
    }
    fn num_pending(&mut self) -> Result<usize> {
        Ok(0)
    }
    /// `writeCRLF`: `writeLine` の行の終わりを CRLF にするか
    fn write_crlf(&self) -> bool {
        true
    }
    /// 読み書きの量 (`Stream::stat`)
    fn stat(&self) -> Option<&Stat> {
        None
    }
}

/// `Stream` の上の読み書き (C++ の `Stream` のメンバー関数のうち、仮想でないもの)
pub trait StreamExt: Stream {
    /// `readChar`: 1 バイト読む (読めなかったら 0)
    fn read_char(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read(&mut b)?;
        Ok(b[0])
    }

    /// `read(p, n)` で、読めなかった分を 0 で埋めたもの (`readShort` などが使う)
    fn read_fixed(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut v = vec![0u8; n];
        self.read(&mut v)?;
        Ok(v)
    }

    fn read_u32_le(&mut self) -> Result<u32> {
        let b = self.read_fixed(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u16_le(&mut self) -> Result<u16> {
        let b = self.read_fixed(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// `std::string read(int remaining)`: ちょうど `n` バイト読む
    fn read_n(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut res = Vec::with_capacity(n.min(1 << 20));
        let mut remaining = n;
        let mut buf = [0u8; 4096];
        while remaining > 0 {
            let want = remaining.min(4096);
            let r = self.read(&mut buf[..want])?;
            if r == 0 {
                return Err(Error::stream("Stream::read: premature end of stream"));
            }
            res.extend_from_slice(&buf[..r]);
            remaining -= r;
        }
        Ok(res)
    }

    /// `std::string readLine(size_t max)`: LF まで (CR は捨てる)。`max` バイトを超えると例外
    fn read_line(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        loop {
            let c = self.read_char()?;
            if c == b'\n' {
                break;
            }
            if c == b'\r' {
                continue;
            }
            if res.len() == max {
                return Err(Error::stream("Line too long"));
            }
            res.push(c);
        }
        Ok(res)
    }

    /// `int readLine(char*, int max)`: `max - 1` バイトまで (超えた分は次の読み出しに残る)
    fn read_line_buf(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        if max == 0 {
            return Ok(res);
        }
        let mut left = max - 1;
        while left > 0 {
            left -= 1;
            let c = self.read_char()?;
            if c == b'\n' {
                break;
            }
            if c == b'\r' {
                continue;
            }
            res.push(c);
        }
        Ok(res)
    }

    /// `readWord`: 空白で区切られた語 (`max - 1` バイトまで)
    fn read_word(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        while !self.eof()? {
            let c = self.read_char()?;
            if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
                if !res.is_empty() {
                    break;
                }
                continue;
            }
            if res.len() + 1 >= max {
                break;
            }
            res.push(c);
        }
        Ok(res)
    }

    /// `skip`: 4096 バイトずつ読んで捨てる
    fn skip(&mut self, len: usize) -> Result<()> {
        let mut buf = [0u8; 4096];
        let mut len = len;
        while len > 0 {
            let r = len.min(4096);
            self.read(&mut buf[..r])?;
            len -= r;
        }
        Ok(())
    }

    /// `writeTo`: 4096 バイトずつ読んで `out` に書く (読めなかった分は 0)
    fn write_to(&mut self, out: &mut dyn Stream, len: usize) -> Result<()> {
        let mut buf = [0u8; 4096];
        let mut len = len;
        while len > 0 {
            let r = len.min(4096);
            buf[..r].iter_mut().for_each(|b| *b = 0);
            self.read(&mut buf[..r])?;
            out.write(&buf[..r])?;
            len -= r;
        }
        Ok(())
    }

    fn write_string(&mut self, s: impl AsRef<[u8]>) -> Result<()> {
        self.write(s.as_ref())
    }

    /// `writeLine`: 文字列と行の終わり (`writeCRLF` なら CRLF)
    fn write_line(&mut self, s: impl AsRef<[u8]>) -> Result<()> {
        self.write(s.as_ref())?;
        if self.write_crlf() {
            self.write(b"\r\n")
        } else {
            self.write(b"\n")
        }
    }

    fn write_char(&mut self, c: u8) -> Result<()> {
        self.write(&[c])
    }

    fn write_u32_le(&mut self, v: u32) -> Result<()> {
        self.write(&v.to_le_bytes())
    }

    fn write_u16_le(&mut self, v: u16) -> Result<()> {
        self.write(&v.to_le_bytes())
    }

    /// `writeUTF8`
    fn write_utf8(&mut self, code: u32) -> Result<usize> {
        let b: Vec<u8> = if code < 0x80 {
            vec![code as u8]
        } else if code < 0x800 {
            vec![(code >> 6 | 0xc0) as u8, (code & 0x3f | 0x80) as u8]
        } else if code < 0x10000 {
            vec![(code >> 12 | 0xe0) as u8, (code >> 6 & 0x3f | 0x80) as u8, (code & 0x3f | 0x80) as u8]
        } else {
            vec![
                (code >> 18 | 0xf0) as u8,
                (code >> 12 & 0x3f | 0x80) as u8,
                (code >> 6 & 0x3f | 0x80) as u8,
                (code & 0x3f | 0x80) as u8,
            ]
        };
        self.write(&b)?;
        Ok(b.len())
    }
}

impl<T: Stream + ?Sized> StreamExt for T {}

/// `StringStream`: メモリー上の読み書き
#[derive(Debug, Default, Clone)]
pub struct StringStream {
    pub buf: Vec<u8>,
    pub pos: usize,
}

impl StringStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from(data: impl Into<Vec<u8>>) -> Self {
        StringStream { buf: data.into(), pos: 0 }
    }

    /// `str()`
    pub fn str(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Stream for StringStream {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.pos >= self.buf.len() {
            return Err(Error::stream("End of stream"));
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn read_upto(&mut self, out: &mut [u8]) -> Result<usize> {
        let n = out.len().min(self.buf.len().saturating_sub(self.pos));
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        let end = self.pos + data.len();
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.buf[self.pos..end].copy_from_slice(data);
        self.pos = end;
        Ok(())
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.pos >= self.buf.len())
    }

    fn rewind(&mut self) -> Result<()> {
        self.pos = 0;
        Ok(())
    }

    fn seek_to(&mut self, pos: i32) -> Result<()> {
        let pos = pos.max(0) as usize;
        if pos > self.buf.len() {
            self.buf.resize(pos, 0);
        }
        self.pos = pos;
        Ok(())
    }

    fn position(&mut self) -> i32 {
        self.pos as i32
    }
}

/// `MemoryStream`: 長さの決まったバッファ。読み出しがはみ出すと 0 で埋めて 0 を返す (例外なし)
#[derive(Debug, Default, Clone)]
pub struct MemoryStream {
    pub buf: Vec<u8>,
    pub pos: usize,
}

impl MemoryStream {
    pub fn new(buf: Vec<u8>) -> Self {
        MemoryStream { buf, pos: 0 }
    }

    pub fn with_len(n: usize) -> Self {
        MemoryStream { buf: vec![0; n], pos: 0 }
    }
}

impl Stream for MemoryStream {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.pos + out.len() <= self.buf.len() {
            out.copy_from_slice(&self.buf[self.pos..self.pos + out.len()]);
            self.pos += out.len();
            Ok(out.len())
        } else {
            out.iter_mut().for_each(|b| *b = 0);
            Ok(0)
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.pos + data.len() > self.buf.len() {
            return Err(Error::stream("Stream - premature end of write()"));
        }
        self.buf[self.pos..self.pos + data.len()].copy_from_slice(data);
        self.pos += data.len();
        Ok(())
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.pos >= self.buf.len())
    }

    fn rewind(&mut self) -> Result<()> {
        self.pos = 0;
        Ok(())
    }

    fn seek_to(&mut self, pos: i32) -> Result<()> {
        self.pos = pos.max(0) as usize;
        Ok(())
    }

    fn position(&mut self) -> i32 {
        self.pos as i32
    }
}

/// `FileStream`
pub struct FileStream {
    file: Option<std::fs::File>,
    at_eof: bool,
    crlf: bool,
    stat: std::sync::Arc<Stat>,
}

impl Default for FileStream {
    fn default() -> Self {
        FileStream { file: None, at_eof: false, crlf: true, stat: std::sync::Arc::new(Stat::default()) }
    }
}

impl FileStream {
    fn open(path: &[u8], opts: &std::fs::OpenOptions) -> Result<FileStream> {
        let p = sys::bytes_to_path(path).ok_or_else(|| Error::stream("Unable to open file"))?;
        let f = opts.open(p).map_err(|_| Error::stream("Unable to open file"))?;
        Ok(FileStream { file: Some(f), ..Default::default() })
    }

    /// `openReadOnly`
    pub fn open_read(path: &[u8]) -> Result<FileStream> {
        Self::open(path, std::fs::OpenOptions::new().read(true))
    }

    /// `openWriteReplace`
    pub fn open_write(path: &[u8]) -> Result<FileStream> {
        Self::open(path, std::fs::OpenOptions::new().write(true).create(true).truncate(true))
    }

    /// `openWriteAppend`
    pub fn open_append(path: &[u8]) -> Result<FileStream> {
        Self::open(path, std::fs::OpenOptions::new().append(true).create(true))
    }

    pub fn from_file(f: std::fs::File) -> FileStream {
        FileStream { file: Some(f), ..Default::default() }
    }

    /// ほかのスレッドから読み書きの量を見るため
    pub fn shared_stat(&self) -> std::sync::Arc<Stat> {
        self.stat.clone()
    }

    pub fn set_crlf(&mut self, v: bool) {
        self.crlf = v;
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// `length`
    pub fn length(&mut self) -> i32 {
        self.file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len() as i32)
    }

    pub fn flush(&mut self) {
        use std::io::Write;
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

impl Stream for FileStream {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        use std::io::Read;
        let f = match self.file.as_mut() {
            Some(f) => f,
            None => return Ok(0),
        };
        if self.at_eof {
            return Err(Error::stream("End of file"));
        }
        // fread と同じく、読めるだけ読む
        let mut n = 0;
        while n < out.len() {
            match f.read(&mut out[n..]) {
                Ok(0) => {
                    self.at_eof = true;
                    break;
                }
                Ok(r) => n += r,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.at_eof = true;
                    break;
                }
            }
        }
        if n > 0 {
            self.stat.update(n as u32, 0);
            // C++ 版は 1 バイト先読みして EOF を調べる
            if !self.at_eof {
                let mut b = [0u8; 1];
                match f.read(&mut b) {
                    Ok(0) => self.at_eof = true,
                    Ok(_) => {
                        use std::io::Seek;
                        let _ = f.seek(std::io::SeekFrom::Current(-1));
                    }
                    Err(_) => {}
                }
            }
            Ok(n)
        } else {
            Err(Error::stream("End of file"))
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(data);
            self.stat.update(0, data.len() as u32);
        }
        Ok(())
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.file.is_none() || self.at_eof)
    }

    fn rewind(&mut self) -> Result<()> {
        use std::io::Seek;
        if let Some(f) = self.file.as_mut() {
            let _ = f.seek(std::io::SeekFrom::Start(0));
            self.at_eof = false;
        }
        Ok(())
    }

    fn seek_to(&mut self, pos: i32) -> Result<()> {
        use std::io::Seek;
        if let Some(f) = self.file.as_mut() {
            let _ = f.seek(std::io::SeekFrom::Start(pos.max(0) as u64));
            self.at_eof = false;
        }
        Ok(())
    }

    fn close(&mut self) {
        self.file = None;
    }

    fn position(&mut self) -> i32 {
        use std::io::Seek;
        self.file.as_mut().and_then(|f| f.stream_position().ok()).map_or(0, |p| p as i32)
    }

    fn write_crlf(&self) -> bool {
        self.crlf
    }

    fn stat(&self) -> Option<&Stat> {
        Some(&self.stat)
    }
}

/// `WriteBufferedStream`: 64KB までためてから書く
pub struct WriteBufferedStream<'a> {
    pub inner: &'a mut dyn Stream,
    buf: Vec<u8>,
}

const WBUF_SIZE: usize = 64 * 1024;

impl<'a> WriteBufferedStream<'a> {
    pub fn new(inner: &'a mut dyn Stream) -> Self {
        WriteBufferedStream { inner, buf: Vec::new() }
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.buf.is_empty() {
            let b = std::mem::take(&mut self.buf);
            self.inner.write(&b)?;
        }
        Ok(())
    }
}

impl Drop for WriteBufferedStream<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            crate::log_error!("StreamException in dtor of WriteBufferedStream: {}", e);
        }
    }
}

// WriteBufferedStream は借用を持つので Send は中身次第だが、&mut dyn Stream は Send (Stream: Send)
impl Stream for WriteBufferedStream<'_> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        self.flush()?;
        self.inner.read(out)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > WBUF_SIZE {
            self.flush()?;
            self.inner.write(data)
        } else if self.buf.len() + data.len() > WBUF_SIZE {
            self.buf.extend_from_slice(data);
            self.flush()
        } else {
            self.buf.extend_from_slice(data);
            Ok(())
        }
    }

    fn close(&mut self) {
        let _ = self.flush();
        self.inner.close();
    }

    fn write_crlf(&self) -> bool {
        self.inner.write_crlf()
    }
}

/// 段階 1〜8 の解析器の `Reader` として `Stream` から読む。誤りは `error` に残る。
pub struct StreamReader<'a> {
    pub s: &'a mut dyn Stream,
    pub error: Option<Error>,
}

impl<'a> StreamReader<'a> {
    pub fn new(s: &'a mut dyn Stream) -> Self {
        StreamReader { s, error: None }
    }

    /// 解析器が `Abort` を返したときの、元の誤り
    pub fn take_error(&mut self) -> Error {
        self.error.take().unwrap_or_else(|| Error::stream("aborted"))
    }
}

impl crate::reader::Reader for StreamReader<'_> {
    fn read_char(&mut self) -> std::result::Result<u8, crate::reader::Abort> {
        self.s.read_char().map_err(|e| {
            self.error = Some(e);
            crate::reader::Abort
        })
    }

    fn read_exact(&mut self, n: usize) -> std::result::Result<Vec<u8>, crate::reader::Abort> {
        self.s.read_n(n).map_err(|e| {
            self.error = Some(e);
            crate::reader::Abort
        })
    }

    fn read_some(&mut self, n: usize) -> std::result::Result<Vec<u8>, crate::reader::Abort> {
        let mut v = vec![0u8; n];
        match self.s.read(&mut v) {
            Ok(r) => {
                v.truncate(r);
                Ok(v)
            }
            Err(e) => {
                self.error = Some(e);
                Err(crate::reader::Abort)
            }
        }
    }

    fn eof(&mut self) -> std::result::Result<bool, crate::reader::Abort> {
        self.s.eof().map_err(|e| {
            self.error = Some(e);
            crate::reader::Abort
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_stream() {
        let mut s = StringStream::from(b"ab\r\ncd\nlong line".to_vec());
        assert_eq!(s.read_line(100).unwrap(), b"ab");
        assert_eq!(s.read_line(100).unwrap(), b"cd");
        assert!(s.read_line(3).is_err());
        let mut s = StringStream::new();
        s.write_line("x").unwrap();
        assert_eq!(s.str(), b"x\r\n");
        s.seek_to(1).unwrap();
        s.write(b"yz").unwrap();
        assert_eq!(s.str(), b"xyz");
        let mut s = StringStream::from(b"abc".to_vec());
        assert_eq!(s.read_n(3).unwrap(), b"abc");
        assert!(s.read_n(1).is_err());
    }

    #[test]
    fn memory_stream() {
        let mut m = MemoryStream::with_len(4);
        m.write(b"abcd").unwrap();
        assert!(m.write(b"e").is_err());
        m.rewind().unwrap();
        let mut b = [0u8; 8];
        assert_eq!(m.read(&mut b).unwrap(), 0);
        assert_eq!(m.read(&mut b[..2]).unwrap(), 2);
        assert_eq!(&b[..2], b"ab");
    }

    #[test]
    fn line_buf_and_word() {
        let mut s = StringStream::from(b"abcdef\n".to_vec());
        assert_eq!(s.read_line_buf(4).unwrap(), b"abc");
        assert_eq!(s.read_line_buf(10).unwrap(), b"def");
        let mut s = StringStream::from(b"  hello world".to_vec());
        assert_eq!(s.read_word(64).unwrap(), b"hello");
        assert_eq!(s.read_word(64).unwrap(), b"world");
    }
}
