//! HTTP の chunked 転送の 1 チャンクを読む (core/common/dechunker.cpp の `Dechunker::getNextChunk`)。
//!
//! 読んだデータを溜めて少しずつ返す部分 (`Dechunker::read`) は C++ に残る。

use crate::reader::{Abort, Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// 読み出しが C++ の例外で中断された
    Abort,
    /// "Protocol error"
    Protocol,
    /// "Chunk size too large"
    TooLarge,
    /// 大きさ 0 の最後のチャンクを読んだ ("Closed on read"。ストリームは EOF になる)
    Closed,
    /// "Premature end"
    Premature,
}

impl From<Abort> for Error {
    fn from(_: Abort) -> Self {
        Error::Abort
    }
}

fn hex_value(c: u8) -> Option<usize> {
    (c as char).to_digit(16).map(|v| v as usize)
}

/// 1 チャンクを読む。
///
/// 返り値の 1 つ目は、読めたチャンクの中身 (C++ 版はチャンクの後ろの CRLF を確かめる前に
/// 中身をバッファに入れていたので、CRLF が不正でも中身は返す)。2 つ目は、その後に起きた
/// エラー (なければ `None`)。
pub fn next_chunk(r: &mut dyn Reader, max_chunk_size: usize) -> (Vec<u8>, Option<Error>) {
    match read_chunk(r, max_chunk_size) {
        Ok((data, trailer)) => (data, trailer.err()),
        Err(e) => (Vec::new(), Some(e)),
    }
}

type Chunk = (Vec<u8>, Result<(), Error>);

fn read_chunk(r: &mut dyn Reader, max_chunk_size: usize) -> Result<Chunk, Error> {
    let mut size: usize = 0;
    let mut digits = 0;

    // チャンクサイズを読み込む。
    loop {
        let c = r.read_char()?;
        if c == b'\r' {
            if r.read_char()? != b'\n' {
                return Err(Error::Protocol);
            }
            break;
        }
        let v = hex_value(c).ok_or(Error::Protocol)?;
        // 桁あふれを防ぐ。16 進 8 桁 (32 ビット) を超える大きさは不正。
        digits += 1;
        if digits > 8 {
            return Err(Error::TooLarge);
        }
        size = size * 0x10 + v;
    }

    // 巨大なメモリ確保を防ぐ。
    if size > max_chunk_size {
        return Err(Error::TooLarge);
    }

    if size == 0 {
        if r.read_char()? != b'\r' || r.read_char()? != b'\n' {
            return Err(Error::Protocol);
        }
        return Err(Error::Closed);
    }

    let data = r.read_some(size)?;
    if data.len() != size {
        return Err(Error::Premature);
    }

    let trailer = (|| -> Result<(), Error> {
        if r.read_char()? != b'\r' || r.read_char()? != b'\n' {
            return Err(Error::Protocol);
        }
        Ok(())
    })();
    Ok((data, trailer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::SliceReader;

    fn chunks(data: &[u8]) -> Vec<(Vec<u8>, Option<Error>)> {
        let mut r = SliceReader { data, pos: 0 };
        let mut out = Vec::new();
        loop {
            let (d, e) = next_chunk(&mut r, 16 * 1024 * 1024);
            let stop = e.is_some();
            out.push((d, e));
            if stop {
                return out;
            }
        }
    }

    #[test]
    fn normal() {
        let c = chunks(b"3\r\nabc\r\nA\r\n0123456789\r\n0\r\n\r\n");
        assert_eq!(c[0], (b"abc".to_vec(), None));
        assert_eq!(c[1], (b"0123456789".to_vec(), None));
        assert_eq!(c[2], (vec![], Some(Error::Closed)));
    }

    #[test]
    fn errors() {
        assert_eq!(chunks(b"x\r\n")[0].1, Some(Error::Protocol));
        assert_eq!(chunks(b"3\rx")[0].1, Some(Error::Protocol));
        assert_eq!(chunks(b"123456789\r\n")[0].1, Some(Error::TooLarge));
        assert_eq!(chunks(b"FFFFFFF\r\n")[0].1, Some(Error::TooLarge));
        assert_eq!(chunks(b"5\r\nab")[0].1, Some(Error::Premature));
        // 中身は読めたが、後ろの CRLF が不正
        assert_eq!(chunks(b"2\r\nabXY")[0], (b"ab".to_vec(), Some(Error::Protocol)));
        assert_eq!(chunks(b"0\r\nX")[0].1, Some(Error::Protocol));
    }
}
