//! PCP の atom の読み書き (core/common/atom.h の `AtomStream`)。下の `Stream` の読み書きは
//! `AtomIo` で抽象化し、C++ の `MemoryStream` と同じ振る舞いの `MemStream` と、C++ の `Stream`
//! (ソケットなど) をコールバックで読む `StreamIo` がある。
//!
//! atom は「ID (4 バイト) + 長さ (int、最上位ビットが立っていれば子の数)」の見出しと、中身か子の
//! atom からなる。整数はリトルエンディアン (C++ 版の `CHECK_ENDIAN`)。

use super::Error;

/// ID4。C++ の `ID4("ok")` のように、4 バイトに満たない名前は 0 で埋める。
pub type Id4 = [u8; 4];

pub const fn id4(s: &[u8]) -> Id4 {
    let mut v = [0u8; 4];
    let mut i = 0;
    while i < s.len() && i < 4 {
        v[i] = s[i];
        i += 1;
    }
    v
}

/// ログに出す ID の文字列 (`ID4::getString()`)。4 バイトのうち、最初の NUL の手前まで。
pub fn id_str(id: &Id4) -> &[u8] {
    let n = id.iter().position(|&b| b == 0).unwrap_or(4);
    &id[..n]
}

/// `AtomStream` の下の `Stream` (`read(void*, int)`、`write`、`skip`)。
pub trait AtomIo {
    /// `Stream::read(void*, int)`。読めなかった分は 0 で埋める (C++ の `MemoryStream` は 0 で埋め、
    /// ソケットは全部読むか例外を投げる)。
    fn read(&mut self, l: i64) -> Result<Vec<u8>, Error>;
    fn write(&mut self, p: &[u8]) -> Result<(), Error>;

    /// `Stream::skip`: 4096 バイトずつ読んで捨てる
    fn skip(&mut self, len: i64) -> Result<(), Error> {
        if len < 0 {
            return Err(Error::Stream("Stream::skip: negative length"));
        }
        let mut len = len;
        while len != 0 {
            let rlen = CHUNK.min(len);
            self.read(rlen)?;
            len -= rlen;
        }
        Ok(())
    }

    /// 読み出しがもう 1 つも成功しない状態か (以後の読み出しはどれも 0 を返し、何も変わらない)。
    /// わからなければ false。
    fn stuck(&self) -> bool {
        false
    }

    fn read_i32(&mut self) -> Result<i32, Error> {
        let b = self.read(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i16(&mut self) -> Result<i16, Error> {
        let b = self.read(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    fn read_id4(&mut self) -> Result<Id4, Error> {
        let b = self.read(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    /// atom の見出しを読む。返り値は (ID、子の数、中身の長さ)。
    fn read_header(&mut self) -> Result<(Id4, i32, i32), Error> {
        let id = self.read_id4()?;
        let v = self.read_i32()? as u32;
        if v & 0x8000_0000 != 0 {
            Ok((id, (v & 0x7fff_ffff) as i32, 0))
        } else {
            Ok((id, 0, v as i32))
        }
    }
}

/// C++ の `MemoryStream` (固定長のバッファ) と `Stream` の基本の読み書き。
///
/// * 読み出しがバッファの終わりを越えるときは、読み出し先を 0 で埋め、位置は進めない (例外なし)。
/// * 書き込みがバッファの終わりを越えるときは `StreamException`。
/// * `skip` と `writeTo` は 4096 バイトずつ読む (越えたかどうかも 4096 バイトごとに決まる)。
pub struct MemStream<'a> {
    buf: &'a mut [u8],
    pub pos: usize,
}

pub(crate) const CHUNK: i64 = 4096;

impl<'a> MemStream<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        MemStream { buf, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 書き込んだ中身 (先頭から `n` バイト)
    pub fn data(&self, n: usize) -> &[u8] {
        &self.buf[..n]
    }

    /// `Stream::writeTo`: `len` バイトを 4096 バイトずつ読んで (読めなければ 0 で埋めて) `out` に書く。
    pub fn write_to(&mut self, out: &mut MemStream, len: i64) -> Result<(), Error> {
        let mut len = len;
        while len != 0 {
            // C++ 版は len が負だと rlen も負になり、read が例外を投げる
            let rlen = if CHUNK > len { len } else { CHUNK };
            let tmp = self.read(rlen)?;
            out.write(&tmp)?;
            len -= rlen;
        }
        Ok(())
    }
}

impl AtomIo for MemStream<'_> {
    /// `MemoryStream::read`。読めなければ 0 で埋めたものを返す。
    fn read(&mut self, l: i64) -> Result<Vec<u8>, Error> {
        if l < 0 {
            return Err(Error::Stream("MemoryStream::read: negative length"));
        }
        let l = l as usize;
        if self.pos + l <= self.buf.len() {
            let v = self.buf[self.pos..self.pos + l].to_vec();
            self.pos += l;
            Ok(v)
        } else {
            Ok(vec![0; l])
        }
    }

    /// ID の 4 バイトすら読めない状態。以後の読み出しはどれも 0 を返し、位置も変わらない。
    fn stuck(&self) -> bool {
        self.pos + 4 > self.buf.len()
    }

    fn write(&mut self, p: &[u8]) -> Result<(), Error> {
        if self.pos + p.len() > self.buf.len() {
            return Err(Error::Stream("Stream - premature end of write()"));
        }
        self.buf[self.pos..self.pos + p.len()].copy_from_slice(p);
        self.pos += p.len();
        Ok(())
    }

    /// `Stream::skip` と同じ結果を、読めない塊を試す回数を減らして求める
    fn skip(&mut self, len: i64) -> Result<(), Error> {
        if len < 0 {
            return Err(Error::Stream("Stream::skip: negative length"));
        }
        let mut len = len;
        while len != 0 {
            let rlen = CHUNK.min(len);
            if self.pos as i64 + rlen > self.buf.len() as i64 {
                // 読めない塊は位置を変えない。以後の塊も同じ長さか短い最後の 1 つなので、
                // 最後の塊だけ試せばよい (結果は同じ)
                let last = len % CHUNK;
                if last != 0 && last != rlen && self.pos as i64 + last <= self.buf.len() as i64 {
                    self.pos += last as usize;
                }
                return Ok(());
            }
            self.pos += rlen as usize;
            len -= rlen;
        }
        Ok(())
    }
}

/// アドレスの atom の値 (`AtomStream::readAddress`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ip {
    /// 4 バイトの atom。`IP(unsigned int)` に渡す値
    V4(u32),
    /// 16 バイトの atom。バイト順を逆にしたもの (`in6_addr` の `s6_addr`)
    V6([u8; 16]),
}

/// `AtomStream`。`num_data` は最後に読んだ見出しの中身の長さで、値を読むときに長さを確かめる。
pub struct AtomStream<IO> {
    pub io: IO,
    pub num_children: i32,
    pub num_data: i32,
}

/// `AtomStream::skip` のネストの上限 (C++ 版の `MAX_SKIP_DEPTH`)
pub const MAX_SKIP_DEPTH: i32 = 64;

impl<IO: AtomIo> AtomStream<IO> {
    pub fn new(io: IO) -> Self {
        AtomStream { io, num_children: 0, num_data: 0 }
    }

    fn check_data(&self, d: i32) -> Result<(), Error> {
        if self.num_data != d {
            return Err(Error::Stream("checkData: Bad atom data"));
        }
        Ok(())
    }

    pub fn read(&mut self) -> Result<(Id4, i32, i32), Error> {
        let (id, c, d) = self.io.read_header()?;
        self.num_children = c;
        self.num_data = d;
        Ok((id, c, d))
    }

    pub fn skip(&mut self, c: i32, d: i32) -> Result<(), Error> {
        self.skip_depth(c, d, 0)
    }

    fn skip_depth(&mut self, c: i32, d: i32, depth: i32) -> Result<(), Error> {
        if depth > MAX_SKIP_DEPTH {
            return Err(Error::Stream("skip: atom nesting too deep"));
        }
        if d != 0 {
            self.io.skip(d as i64)?;
        }
        for _ in 0..c {
            // バッファの終わりに着いたら、残りの子はどれも (0, 0, 0) の見出しになり何もしないので、
            // 1 つ読んで終える (C++ 版は子の数だけ、最大 2^31 回繰り返す)
            let stuck = self.io.stuck();
            let (_, numc, data) = self.read()?;
            self.skip_depth(numc, data, depth + 1)?;
            if stuck {
                break;
            }
        }
        Ok(())
    }

    pub fn read_int(&mut self) -> Result<i32, Error> {
        self.check_data(4)?;
        self.io.read_i32()
    }

    pub fn read_id4(&mut self) -> Result<Id4, Error> {
        self.check_data(4)?;
        self.io.read_id4()
    }

    /// `readShort` (short を int に広げた値)
    pub fn read_short(&mut self) -> Result<i32, Error> {
        self.check_data(2)?;
        Ok(self.io.read_i16()? as i32)
    }

    /// `readChar`。C++ 版の `char` は x86 では符号付き、ARM の Linux では符号なしで、値が変わって
    /// いた。Rust 版は CPU によらず x86 と同じ符号付き (-128〜127) にする。
    pub fn read_char(&mut self) -> Result<i32, Error> {
        self.check_data(1)?;
        Ok(self.io.read(1)?[0] as i8 as i32)
    }

    /// `readBytes(p, l)`
    pub fn read_bytes(&mut self, l: i32) -> Result<Vec<u8>, Error> {
        self.check_data(l)?;
        self.io.read(l as i64)
    }

    pub fn read_bytes16(&mut self) -> Result<[u8; 16], Error> {
        let v = self.read_bytes(16)?;
        let mut a = [0u8; 16];
        a.copy_from_slice(&v);
        Ok(a)
    }

    /// `readBytes(s, max, dlen)`: 読み出し先に書かれるバイト列 (`max` と `dlen` の小さいほうの長さ)。
    /// `dlen` が `max` より長いと、C++ 版は `checkData(max)` で例外になる (切り詰めない)。
    pub fn read_bytes_max(&mut self, max: i32, dlen: i32) -> Result<Vec<u8>, Error> {
        self.check_data(dlen)?;
        if max > dlen {
            self.read_bytes(dlen)
        } else {
            let v = self.read_bytes(max)?;
            self.io.skip(dlen as i64 - max as i64)?;
            Ok(v)
        }
    }

    /// `readString(s, max, dlen)`: 読み出し先に書かれるバイト列 (`s[max-1] = 0` は呼んだ側で)
    pub fn read_string(&mut self, max: i32, dlen: i32) -> Result<Vec<u8>, Error> {
        self.check_data(dlen)?;
        self.read_bytes_max(max, dlen)
    }

    pub fn read_address(&mut self) -> Result<Ip, Error> {
        if self.num_data == 4 {
            Ok(Ip::V4(self.read_int()? as u32))
        } else if self.num_data == 16 {
            let v = self.read_bytes_max(16, 16)?;
            let mut a = [0u8; 16];
            for (i, b) in v.iter().rev().enumerate() {
                a[i] = *b;
            }
            Ok(Ip::V6(a))
        } else {
            Err(Error::Stream("readAddress: Bad atom data"))
        }
    }

    pub fn write_parent(&mut self, id: Id4, nc: i32) -> Result<(), Error> {
        self.io.write(&id)?;
        self.io.write(&((nc as u32) | 0x8000_0000).to_le_bytes())
    }

    pub fn write_int(&mut self, id: Id4, d: i32) -> Result<(), Error> {
        self.io.write(&id)?;
        self.io.write(&4i32.to_le_bytes())?;
        self.io.write(&d.to_le_bytes())
    }

    pub fn write_short(&mut self, id: Id4, d: i16) -> Result<(), Error> {
        self.io.write(&id)?;
        self.io.write(&2i32.to_le_bytes())?;
        self.io.write(&d.to_le_bytes())
    }

    pub fn write_char(&mut self, id: Id4, d: u8) -> Result<(), Error> {
        self.io.write(&id)?;
        self.io.write(&1i32.to_le_bytes())?;
        self.io.write(&[d])
    }

    pub fn write_bytes(&mut self, id: Id4, p: &[u8]) -> Result<(), Error> {
        self.io.write(&id)?;
        self.io.write(&(p.len() as i32).to_le_bytes())?;
        self.io.write(p)
    }

}

impl AtomStream<MemStream<'_>> {
    /// `writeStream`: 見出しを書き、`input` から `l` バイト写す
    fn write_stream(&mut self, id: Id4, input: &mut MemStream, l: i32) -> Result<i32, Error> {
        self.io.write(&id)?;
        self.io.write(&l.to_le_bytes())?;
        input.write_to(&mut self.io, l as i64)?;
        Ok(8i32.wrapping_add(l))
    }

    /// `writeAtoms`: `input` から、見出しを読み終えた atom (`id`, `cnt`, `data`) を子も含めて写す。
    pub fn write_atoms(&mut self, id: Id4, input: &mut MemStream, cnt: i32, data: i32) -> Result<i32, Error> {
        let mut total = 0i32;
        if cnt != 0 {
            self.write_parent(id, cnt)?;
            total = total.wrapping_add(8);
            for _ in 0..cnt {
                // C++ 版は子ごとに別の AtomStream で読むので、self の num_data は変わらない
                let (cid, c, d) = input.read_header()?;
                total = total.wrapping_add(self.write_atoms(cid, input, c, d)?);
            }
        } else {
            total = total.wrapping_add(self.write_stream(id, input, data)?);
        }
        Ok(total)
    }
}
