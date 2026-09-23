//! C++ の `Stream` から読むための抽象。
//!
//! 解析器は `Reader` から 1 バイトずつ (または決まった長さを) 読む。C++ からは、`Stream` を
//! 呼び出すコールバック (`ffi::CReader`) がこれを実装する。コールバックの中で C++ の例外が
//! 起きたら `Abort` を返し、解析器はそのまま処理をやめる。例外は C++ 側で投げ直す。
//!
//! 読み出しの細かい挙動 (例えば `MemoryStream` はデータが尽きても例外を投げず 0 を返す) は
//! C++ の `Stream` の実装のまま変わらない。

/// 読み出しが C++ の例外で中断された。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abort;

pub trait Reader {
    /// `Stream::readChar`
    fn read_char(&mut self) -> Result<u8, Abort>;
    /// `Stream::read(int)`: ちょうど `n` バイト読む (足りなければ C++ 側が例外を投げる)。
    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, Abort>;
    /// `Stream::read(void*, int)`: 最大 `n` バイト読み、読めた分を返す。
    fn read_some(&mut self, n: usize) -> Result<Vec<u8>, Abort>;
    /// `Stream::eof`
    fn eof(&mut self) -> Result<bool, Abort>;
}

/// テスト用: バイト列から読む。C++ の `MemoryStream` と同じく、データが尽きると
/// `read_char` と `read_some` は 0 (空) を返し、`read_exact` は中断する。
#[cfg(test)]
pub struct SliceReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

#[cfg(test)]
impl Reader for SliceReader<'_> {
    fn read_char(&mut self) -> Result<u8, Abort> {
        match self.data.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                Ok(c)
            }
            None => Ok(0),
        }
    }

    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        if n == 0 {
            return Ok(Vec::new());
        }
        if self.pos + n > self.data.len() {
            return Err(Abort);
        }
        let v = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }

    fn read_some(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        if self.pos + n > self.data.len() {
            return Ok(Vec::new());
        }
        let v = self.data[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }

    fn eof(&mut self) -> Result<bool, Abort> {
        Ok(self.pos >= self.data.len())
    }
}
