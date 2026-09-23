//! 単体テスト用の `Host`。入力は C++ の `StringStream` と同じく、途中までの読み出しはでき、
//! 末尾で読もうとすると中断する (例外)。チャンネルへの操作は文字列にして記録する。

use super::{HeadKind, Host, LogLevel, Track};
use crate::reader::{Abort, Reader};

#[derive(Default)]
pub struct TestHost {
    pub input: Vec<u8>,
    pub pos: usize,
    pub events: Vec<String>,
    /// 送ったパケットの中身 (ヘッダーを含む)
    pub packets: Vec<Vec<u8>>,
    pub stream_pos: u32,
    pub stream_index: u32,
    pub head: Vec<u8>,
    pub bitrate: i32,
    pub ogm: bool,
    pub track: Option<Track>,
    pub icy: i32,
    pub read_delay: bool,
    pub now: f64,
    pub measured: i32,
    pub metadata: Vec<Vec<u8>>,
}

impl TestHost {
    pub fn new(input: &[u8]) -> TestHost {
        TestHost { input: input.to_vec(), ..Default::default() }
    }
}

impl Reader for TestHost {
    fn read_char(&mut self) -> Result<u8, Abort> {
        let c = *self.input.get(self.pos).ok_or(Abort)?;
        self.pos += 1;
        Ok(c)
    }

    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        let mut v = Vec::new();
        let mut remaining = n;
        while remaining > 0 {
            let chunk = self.read_some(remaining.min(4096))?;
            remaining -= chunk.len();
            v.extend(chunk);
        }
        Ok(v)
    }

    fn read_some(&mut self, n: usize) -> Result<Vec<u8>, Abort> {
        if self.pos == self.input.len() {
            return Err(Abort);
        }
        let end = (self.pos + n).min(self.input.len());
        let v = self.input[self.pos..end].to_vec();
        self.pos = end;
        Ok(v)
    }

    fn eof(&mut self) -> Result<bool, Abort> {
        Ok(self.pos == self.input.len())
    }
}

impl Host for TestHost {
    fn ready(&mut self) -> Result<bool, Abort> {
        Ok(self.pos < self.input.len())
    }

    fn raise_bitrate(&mut self) -> Result<(), Abort> {
        if self.measured > self.bitrate {
            self.bitrate = self.measured;
            self.events.push(format!("bitrate {}", self.bitrate));
        }
        Ok(())
    }

    fn packet(&mut self, data: &[u8], cont: bool, read_delay: bool) -> Result<(), Abort> {
        self.events.push(format!("data {} {} cont={} delay={}", self.stream_pos, data.len(), cont, read_delay));
        self.packets.push(data.to_vec());
        self.stream_pos += data.len() as u32;
        Ok(())
    }

    fn head(&mut self, kind: HeadKind, data: &[u8]) -> Result<(), Abort> {
        match kind {
            HeadKind::NewStream | HeadKind::Mkv => {
                self.stream_index += 1;
                self.head = data.to_vec();
                self.stream_pos = 0;
            }
            HeadKind::Ogg => {}
        }
        self.events.push(format!("head {:?} {} {}", kind, self.stream_pos, self.head.len()));
        self.packets.push(self.head.clone());
        self.stream_pos += self.head.len() as u32;
        Ok(())
    }

    fn head_len(&mut self) -> u32 {
        self.head.len() as u32
    }

    fn head_clear(&mut self) {
        self.head.clear();
    }

    fn head_append(&mut self, data: &[u8]) -> Result<(), Abort> {
        assert!(self.head.len() + data.len() < super::MAX_DATALEN);
        self.head.extend_from_slice(data);
        Ok(())
    }

    fn set_bitrate(&mut self, bitrate: i32) -> Result<(), Abort> {
        self.bitrate = bitrate;
        self.events.push(format!("bitrate {}", bitrate));
        Ok(())
    }

    fn ogg_set_info(&mut self, bitrate: i32, ogm: bool) {
        self.bitrate = bitrate;
        self.ogm |= ogm;
        self.events.push(format!("ogginfo {} {}", bitrate, ogm));
    }

    fn set_track(&mut self, track: &Track) -> Result<(), Abort> {
        self.track = Some(track.clone());
        Ok(())
    }

    fn mp3_metadata(&mut self, buf: &[u8]) -> Result<(), Abort> {
        self.metadata.push(buf.to_vec());
        Ok(())
    }

    fn icy_meta_interval(&mut self) -> i32 {
        self.icy
    }

    fn read_delay(&mut self) -> bool {
        self.read_delay
    }

    fn dtime(&mut self) -> f64 {
        self.now
    }

    fn time(&mut self) -> u32 {
        self.now as u32
    }

    fn sleep(&mut self, ms: i32) {
        self.events.push(format!("sleep {}", ms));
    }

    fn sleep_until(&mut self, t: f64) {
        self.events.push(format!("until {}", t));
    }

    fn log(&mut self, _level: LogLevel, _msg: &str) {}
}

/// 変異ファズ用の簡単な擬似乱数 (xorshift64)
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// 1 バイト書き換え、挿入、削除、切り詰めのどれかを数回
    pub fn mutate(&mut self, data: &[u8]) -> Vec<u8> {
        let mut v = data.to_vec();
        for _ in 0..1 + self.below(4) {
            let i = self.below(v.len() + 1);
            match self.below(5) {
                0 | 1 if i < v.len() => v[i] = self.next() as u8,
                2 => v.insert(i, self.next() as u8),
                3 if i < v.len() => {
                    v.remove(i);
                }
                4 => v.truncate(i),
                _ => {}
            }
        }
        v
    }
}

/// 解析器を入力が尽きるまで (または `limit` 回) 動かす。panic しないこと、止まることを確かめる。
pub fn run(kind: super::Kind, host: &mut TestHost, limit: usize) -> super::Result<()> {
    let mut p = super::Parser::new(kind);
    p.read_header(host)?;
    for _ in 0..limit {
        p.read_packet(host)?;
    }
    Ok(())
}
