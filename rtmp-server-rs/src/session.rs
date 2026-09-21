//! RTMP セッション (ハンドシェイク、チャンク処理、コマンド応答)。
//! C++ 版 (rtmp-server/session.h, message.h) の移植。
//!
//! 振る舞いは C++ 版に合わせてあり、違うのは次の点だけ:
//!  - 読み取りは常に範囲検査され、範囲外なら Error になる (panic / 未定義動作にならない)。
//!  - 未完了のメッセージが溜められる合計サイズに上限を設けた (MAX_BUFFERED_BYTES)。
//!  - 送信側で 1 チャンクに収まらないメッセージの 2 個目以降のチャンクが正しく作られる
//!    (C++ 版は先頭チャンクのヘッダーも fmt 3 になっていた。現在の応答は全て 1 チャンクに収まるので実害なし)。

use crate::amf0::{Reader, Value};
use crate::flv::FlvWriter;
use crate::{log, Error, Result};
use std::io::{Read, Write};

/// 音声・映像以外 (コマンド、メタデータ、制御メッセージ) の 1 メッセージの長さの上限。
pub const MAX_CONTROL_MESSAGE_LENGTH: usize = 1024 * 1024;
/// 組み立て途中のメッセージが全チャンクストリームで保持できる合計バイト数の上限。
/// 24 ビットの長さ (最大 16 MiB) を名乗るメッセージを 62 本のチャンクストリームで
/// 同時に始められると 1 GiB 近く確保させられるため。正常な配信では 1 本ずつしか未完了にならない。
pub const MAX_BUFFERED_BYTES: usize = 64 * 1024 * 1024;

struct ChunkStream {
    timestamp: u32,
    length: usize,
    type_id: u8,
    stream_id: u32,
    received: usize,
    data: Vec<u8>,
}

impl ChunkStream {
    fn new(timestamp: u32, length: usize, type_id: u8, stream_id: u32) -> ChunkStream {
        ChunkStream { timestamp, length, type_id, stream_id, received: 0, data: Vec::new() }
    }

    fn remaining(&self) -> usize {
        self.length - self.received
    }
}

pub struct Message {
    pub timestamp: u32,
    pub type_id: u8,
    pub data: Vec<u8>,
}

pub struct Session<C: Read + Write, W: Write> {
    client: C,
    flv: FlvWriter<W>,
    max_incoming_chunk_size: usize,
    max_outgoing_chunk_size: usize,
    quitting: bool,
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// ログに出す文字列を短く切る。
fn brief(s: String) -> String {
    const LIMIT: usize = 200;
    if s.len() <= LIMIT {
        s
    } else {
        let mut end = LIMIT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

impl<C: Read + Write, W: Write> Session<C, W> {
    pub fn new(client: C, out: W) -> Session<C, W> {
        Session {
            client,
            flv: FlvWriter::new(out),
            max_incoming_chunk_size: 128,
            max_outgoing_chunk_size: 128,
            quitting: false,
        }
    }

    pub fn into_parts(self) -> (C, W) {
        (self.client, self.flv.into_inner())
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.client.read_exact(&mut b)?;
        Ok(b)
    }

    fn read_into(&mut self, dst: &mut Vec<u8>, n: usize) -> Result<()> {
        // 宣言された長さ分を先に確保せず、実際に届いた分だけ伸ばす。
        let got = (&mut self.client).take(n as u64).read_to_end(dst)?;
        if got != n {
            return Err(Error::Eof);
        }
        Ok(())
    }

    fn be24(b: &[u8]) -> u32 {
        ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32)
    }

    pub fn run(&mut self) -> Result<()> {
        self.handshake()?;

        // チャンクストリームごとの最後のメッセージ。cs_id は 2..=63。
        let mut cstreams: Vec<Option<ChunkStream>> = (0..64).map(|_| None).collect();

        while !self.quitting {
            let basic = self.read_array::<1>()?[0];
            let fmt = basic >> 6;
            let cs_id = (basic & 0x3f) as usize;
            if cs_id < 2 {
                return Err(Error::protocol("cs_id out of range"));
            }

            let next = match fmt {
                0 => {
                    let h = self.read_array::<11>()?;
                    let timestamp = Self::be24(&h[0..3]);
                    let length = Self::be24(&h[3..6]) as usize;
                    let type_id = h[6];
                    let stream_id = u32::from_le_bytes([h[7], h[8], h[9], h[10]]);
                    if timestamp >= 0xff_ffff {
                        return Err(Error::protocol("extended timestamp not implemented"));
                    }
                    Some(ChunkStream::new(timestamp, length, type_id, stream_id))
                }
                1 => {
                    let (pts, psid) = match &cstreams[cs_id] {
                        Some(p) => (p.timestamp, p.stream_id),
                        None => return Err(Error::protocol("protocol error")),
                    };
                    let h = self.read_array::<7>()?;
                    let tdelta = Self::be24(&h[0..3]);
                    let length = Self::be24(&h[3..6]) as usize;
                    if tdelta >= 0xff_ffff {
                        return Err(Error::protocol("extended timestamp not implemented"));
                    }
                    Some(ChunkStream::new(pts.wrapping_add(tdelta), length, h[6], psid))
                }
                2 => {
                    let (pts, plen, ptype, psid) = match &cstreams[cs_id] {
                        Some(p) if p.remaining() == 0 => (p.timestamp, p.length, p.type_id, p.stream_id),
                        _ => return Err(Error::protocol("protocol error")),
                    };
                    let h = self.read_array::<3>()?;
                    Some(ChunkStream::new(pts.wrapping_add(Self::be24(&h)), plen, ptype, psid))
                }
                _ => {
                    // fmt 3: 直前のメッセージが完結していれば、同じヘッダーで次のメッセージが始まる。
                    // (C++ 版と同じく、タイムスタンプの差分は再適用しない。)
                    match &mut cstreams[cs_id] {
                        Some(p) => {
                            if p.remaining() == 0 {
                                p.received = 0;
                                p.data.clear();
                            }
                        }
                        None => return Err(Error::protocol("protocol error")),
                    }
                    None
                }
            };
            if let Some(cs) = next {
                cstreams[cs_id] = Some(cs);
            }

            let (is_av, length) = {
                let cs = cstreams[cs_id].as_ref().expect("chunk stream set above");
                (cs.type_id == 0x08 || cs.type_id == 0x09, cs.length)
            };
            // 音声・映像以外は小さいはずなので、大きなものは拒否してメモリ消費を抑える。
            if !is_av && length > MAX_CONTROL_MESSAGE_LENGTH {
                return Err(Error::protocol("message too large"));
            }

            let n = {
                let cs = cstreams[cs_id].as_ref().expect("chunk stream set above");
                cs.remaining().min(self.max_incoming_chunk_size)
            };
            let mut data = std::mem::take(&mut cstreams[cs_id].as_mut().expect("set").data);
            let res = self.read_into(&mut data, n);
            cstreams[cs_id].as_mut().expect("set").data = data;
            res?;
            cstreams[cs_id].as_mut().expect("set").received += n;

            let buffered: usize = cstreams.iter().flatten().map(|c| c.data.len()).sum();
            if buffered > MAX_BUFFERED_BYTES {
                return Err(Error::protocol("too much buffered data"));
            }

            let complete = cstreams[cs_id].as_ref().expect("set").remaining() == 0;
            if complete {
                let cs = cstreams[cs_id].as_mut().expect("set");
                let msg = Message {
                    timestamp: cs.timestamp,
                    type_id: cs.type_id,
                    data: std::mem::take(&mut cs.data), // 完結したら本体は手放す
                };
                self.on_message(&msg)?;
            }
        }
        Ok(())
    }

    // ---- 送信 ----

    /// 1 メッセージを (必要ならチャンクに分けて) 送る。
    fn send(&mut self, type_id: u8, stream_id: u32, cs_id: u8, data: &[u8]) -> Result<()> {
        if data.len() > 0xff_ffff {
            return Err(Error::protocol("outgoing message too large"));
        }
        let mut out = Vec::with_capacity(data.len() + 16);
        out.push(cs_id & 0x3f); // fmt 0
        out.extend_from_slice(&[0, 0, 0]); // timestamp
        out.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        out.push(type_id);
        out.extend_from_slice(&stream_id.to_le_bytes());
        for (i, chunk) in data.chunks(self.max_outgoing_chunk_size).enumerate() {
            if i > 0 {
                out.push(0xc0 | (cs_id & 0x3f)); // fmt 3
            }
            out.extend_from_slice(chunk);
        }
        self.client.write_all(&out)?;
        Ok(())
    }

    fn send_command(&mut self, stream_id: u32, cs_id: u8, values: &[&Value]) -> Result<()> {
        let mut data = Vec::new();
        for v in values {
            data.extend_from_slice(&v.serialize()?);
        }
        self.send(0x14, stream_id, cs_id, &data)
    }

    // ---- コマンド ----

    fn on_connect(&mut self, tid: &Value) -> Result<()> {
        self.send(0x05, 0, 2, &be32(5_000_000))?; // Window Acknowledgement Size
        let mut bw = be32(5_000_000).to_vec();
        bw.push(1); // 0: Hard, 1: Soft, 2: Dynamic
        self.send(0x06, 0, 2, &bw)?; // Set Peer Bandwidth
        self.send(0x01, 0, 2, &be32(4096))?; // Set Chunk Size
        self.max_outgoing_chunk_size = 4096;

        let props = Value::object(&[
            ("fmsVer", Value::string("FMS/3,0,1,123")),
            ("capabilities", Value::Number(31.0)),
        ]);
        let info = Value::object(&[
            ("level", Value::string("status")),
            ("code", Value::string("NetConnection.Connect.Success")),
            ("description", Value::string("Connection succeeded.")),
            ("objectEncoding", Value::Number(0.0)),
        ]);
        self.send_command(0, 3, &[&Value::string("_result"), tid, &props, &info])
    }

    fn on_fcpublish(&mut self, tid: &Value, params: &[Value]) -> Result<()> {
        if params.len() < 2 {
            return Err(Error::protocol("FCPublish: missing parameter"));
        }
        self.send_command(0, 3, &[&Value::string("_result"), tid, &params[1]])
    }

    fn on_create_stream(&mut self, tid: &Value) -> Result<()> {
        self.send_command(0, 3, &[&Value::string("_result"), tid, &Value::Null, &Value::Number(1.0)])
    }

    fn on_publish(&mut self) -> Result<()> {
        let info = Value::object(&[
            ("level", Value::string("status")),
            ("code", Value::string("NetStream.Publish.Start")),
            ("description", Value::string("Start publishing")),
        ]);
        self.send_command(1, 8, &[&Value::string("onStatus"), &Value::Number(0.0), &Value::Null, &info])
    }

    fn on_command(&mut self, msg: &Message) -> Result<()> {
        let mut r = Reader::new(&msg.data);
        let command = r.read_value()?;
        log!("command = {}", brief(command.inspect()));
        let tid = r.read_value()?;
        log!("transaction_id = {}", brief(tid.inspect()));
        let mut params = Vec::new();
        while !r.eof() {
            let p = r.read_value()?;
            log!("param{} = {}", params.len() + 1, brief(p.inspect()));
            params.push(p);
        }

        match command.as_string()? {
            b"connect" => self.on_connect(&tid),
            b"FCPublish" => self.on_fcpublish(&tid, &params),
            b"createStream" => self.on_create_stream(&tid),
            b"publish" => self.on_publish(),
            b"deleteStream" => {
                self.quitting = true;
                Ok(())
            }
            other => {
                log!("ignoring command {}", brief(String::from_utf8_lossy(other).into_owned()));
                Ok(())
            }
        }
    }

    fn on_metadata(&mut self, msg: &Message) -> Result<()> {
        // 3 つの AMF0 値を期待している。"@setDataFrame", "onMetaData", { ... }
        log!("Got metadata");
        let mut r = Reader::new(&msg.data);
        let first = r.read_value()?;
        log!("{}", brief(first.inspect()));

        let pos = r.position();
        let mut has_video = false;
        let mut has_audio = false;
        let flv = &mut self.flv;
        // C++ 版は std::runtime_error だけを捕まえて続行する (EOF や I/O エラーは通さない)。
        let attempt = (|| -> Result<()> {
            if r.read_value()?.as_string()? == b"onMetaData" {
                let v = r.read_value()?;
                log!("{}", brief(v.inspect()));
                has_video = v.get("videocodecid")?.is_some();
                has_audio = v.get("audiocodecid")?.is_some();
            }
            flv.write_file_header(has_audio, has_video)
        })();
        match attempt {
            Ok(()) => {}
            Err(Error::Protocol(m)) => log!("Error: {}", m),
            Err(e) => return Err(e),
        }
        self.flv.write_script_tag(msg.timestamp, &msg.data[pos..])
    }

    fn on_set_chunk_size(&mut self, msg: &Message) -> Result<()> {
        if msg.data.len() < 4 {
            return Err(Error::protocol("SetChunkSize: message too short"));
        }
        let v = u32::from_be_bytes([msg.data[0], msg.data[1], msg.data[2], msg.data[3]]);
        // 仕様上 1 〜 0x7FFFFFFF (通常は 16777215 まで)。0 は不正。
        if !(1..=0xff_ffff).contains(&v) {
            return Err(Error::protocol("SetChunkSize: invalid chunk size"));
        }
        log!(
            "max incoming chunk size is now {} (was {})",
            v,
            self.max_incoming_chunk_size
        );
        self.max_incoming_chunk_size = v as usize;
        Ok(())
    }

    fn on_message(&mut self, msg: &Message) -> Result<()> {
        match msg.type_id {
            0x14 => self.on_command(msg),
            0x12 => self.on_metadata(msg),
            0x01 => self.on_set_chunk_size(msg),
            0x08 => self.flv.write_audio_tag(msg.timestamp, &msg.data),
            0x09 => self.flv.write_video_tag(msg.timestamp, &msg.data),
            0x05 => {
                let v = msg.data.iter().fold(0i32, |a, &b| a.wrapping_mul(256).wrapping_add(b as i32));
                log!("Window Acknowledgement Size: {}", v);
                Ok(())
            }
            t => {
                log!("unknown message type id {}", t);
                Ok(())
            }
        }
    }

    // ---- ハンドシェイク (単純ハンドシェイク) ----

    /// 毎回同じ擬似乱数列 (xorshift)。ハンドシェイクに暗号学的な乱数は要らない。
    fn pseudo_random_bytes(n: usize) -> Vec<u8> {
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 8) as u8
            })
            .collect()
    }

    fn handshake(&mut self) -> Result<()> {
        // C0
        let c0 = self.read_array::<1>()?[0];
        if c0 >= 32 {
            return Err(Error::protocol("invalid C0"));
        }

        // S0 + S1
        let mut s0s1 = Vec::with_capacity(1 + 1536);
        s0s1.push(3);
        s0s1.extend_from_slice(&be32(0));
        s0s1.extend_from_slice(&be32(0x0d0e_0a0d));
        s0s1.extend_from_slice(&Self::pseudo_random_bytes(1528));
        self.client.write_all(&s0s1)?;

        // C1 → S2 (C1 の時刻 + 0 + C1 のランダム部をそのまま返す)
        let c1 = self.read_array::<1536>()?;
        let mut s2 = Vec::with_capacity(1536);
        s2.extend_from_slice(&c1[0..4]);
        s2.extend_from_slice(&[0, 0, 0, 0]);
        s2.extend_from_slice(&c1[8..]);
        self.client.write_all(&s2)?;

        // C2 (内容は見ない)
        let _c2 = self.read_array::<1536>()?;
        Ok(())
    }
}
