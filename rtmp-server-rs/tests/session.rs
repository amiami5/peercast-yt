//! セッション全体の結合テスト (メモリ上の疑似クライアントと疑似出力先)。
//! ネットワークは使わない。実ネットワークでの比較は tests/differential.py を参照。

use rtmpserver::session::Session;
use rtmpserver::{Error, Result};
use std::io::{self, Cursor, Read, Write};
use std::sync::{Arc, Mutex};

/// 読み出しは固定バイト列、書き込みは全部溜める疑似ソケット。
struct Duplex {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}
impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.input.read(buf)
    }
}
impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);
impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---- 入力の組み立て ----
fn amf_num(x: f64) -> Vec<u8> {
    let mut v = vec![0];
    v.extend_from_slice(&x.to_be_bytes());
    v
}
fn amf_str(s: &str) -> Vec<u8> {
    let mut v = vec![2];
    v.extend_from_slice(&(s.len() as u16).to_be_bytes());
    v.extend_from_slice(s.as_bytes());
    v
}
fn amf_null() -> Vec<u8> {
    vec![5]
}

fn chunked(cs: u8, ts: u32, ty: u8, sid: u32, payload: &[u8], chunk: usize) -> Vec<u8> {
    let mut out = vec![cs];
    out.extend_from_slice(&ts.to_be_bytes()[1..]);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
    out.push(ty);
    out.extend_from_slice(&sid.to_le_bytes());
    for (i, c) in payload.chunks(chunk).enumerate() {
        if i > 0 {
            out.push(0xc0 | cs);
        }
        out.extend_from_slice(c);
    }
    out
}

fn handshake_bytes() -> Vec<u8> {
    let mut v = vec![3];
    v.extend((0..1536).map(|i| (i % 251) as u8)); // C1
    v.extend(std::iter::repeat(0).take(1536)); // C2
    v
}

fn cmd(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

fn publish_prefix() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(chunked(3, 0, 0x14, 0, &cmd(&[amf_str("connect"), amf_num(1.0), amf_null()]), 128));
    v.extend(chunked(3, 0, 0x14, 0, &cmd(&[amf_str("FCPublish"), amf_num(2.0), amf_null(), amf_str("key")]), 128));
    v.extend(chunked(3, 0, 0x14, 0, &cmd(&[amf_str("createStream"), amf_num(3.0), amf_null()]), 128));
    v.extend(chunked(4, 0, 0x14, 1, &cmd(&[amf_str("publish"), amf_num(4.0), amf_null(), amf_str("key")]), 128));
    v
}

fn metadata(av: bool) -> Vec<u8> {
    let mut body = amf_str("@setDataFrame");
    body.extend(amf_str("onMetaData"));
    body.push(8); // ECMA array
    body.extend_from_slice(&(if av { 2u32 } else { 0 }).to_be_bytes());
    if av {
        for k in ["videocodecid", "audiocodecid"] {
            body.extend_from_slice(&(k.len() as u16).to_be_bytes());
            body.extend_from_slice(k.as_bytes());
            body.extend(amf_num(7.0));
        }
    }
    body.extend_from_slice(&[0, 0, 9]);
    chunked(4, 0, 0x12, 1, &body, 128)
}

fn run(input: Vec<u8>) -> (Result<()>, Vec<u8>, Vec<u8>) {
    let sink = Shared::default();
    let mut s = Session::new(Duplex { input: Cursor::new(input), output: Vec::new() }, sink.clone());
    let r = s.run();
    let (client, _) = s.into_parts();
    let flv = sink.0.lock().unwrap().clone();
    (r, client.output, flv)
}

#[test]
fn publishes_flv() {
    let mut input = handshake_bytes();
    input.extend(publish_prefix());
    input.extend(metadata(true));
    input.extend(chunked(2, 0, 0x01, 0, &4096u32.to_be_bytes(), 128)); // SetChunkSize
    let video = [vec![0x17, 1, 0, 0, 0], vec![0xaa; 1000]].concat(); // 128 を超える → 複数チャンク...
    input.extend(chunked(6, 40, 0x09, 1, &video, 4096));
    input.extend(chunked(5, 40, 0x08, 1, &[0xaf, 1, 9, 9], 4096));
    input.extend(chunked(3, 0, 0x14, 0, &cmd(&[amf_str("deleteStream"), amf_num(5.0), amf_null(), amf_num(1.0)]), 128));

    let (r, resp, flv) = run(input);
    r.unwrap();

    // FLV ヘッダー: 音声あり・映像ありのフラグ
    assert_eq!(&flv[..13], &[b'F', b'L', b'V', 1, 0x05, 0, 0, 0, 9, 0, 0, 0, 0]);
    // 動画タグと音声タグがこの順で 1 個ずつ入っている
    let vpos = flv.windows(5).position(|w| w == [0x17, 1, 0, 0, 0]).expect("video payload");
    assert_eq!(flv[vpos - 11], 9);
    assert!(flv.ends_with(&[0xaf, 1, 9, 9]));
    // サーバーの応答: S0(3) で始まり、S1+S2 の後に制御メッセージ (ServerBW=type 5)
    assert_eq!(resp[0], 3);
    assert_eq!(resp[1 + 1536 + 1536], 2); // fmt0, cs_id 2
    assert_eq!(resp[1 + 1536 + 1536 + 7], 0x05);
}

#[test]
fn video_before_metadata_is_an_error() {
    let mut input = handshake_bytes();
    input.extend(publish_prefix());
    input.extend(chunked(6, 0, 0x09, 1, &[0x17, 1, 0, 0, 0], 128));
    let (r, _, flv) = run(input);
    assert!(matches!(r, Err(Error::Protocol(_))));
    assert!(flv.is_empty());
}

#[test]
fn invalid_c0_is_rejected() {
    let mut input = handshake_bytes();
    input[0] = 0x40;
    let (r, resp, _) = run(input);
    assert!(matches!(r, Err(Error::Protocol(m)) if m.contains("C0")));
    assert!(resp.is_empty());
}

#[test]
fn oversized_control_message_is_rejected() {
    let mut input = handshake_bytes();
    // type 0x14 (コマンド) で 2 MiB を名乗る。本体は送らない。
    input.extend([3, 0, 0, 0, 0x20, 0, 0, 0x14, 0, 0, 0, 0]);
    let (r, _, _) = run(input);
    assert!(matches!(r, Err(Error::Protocol(m)) if m.contains("too large")));
}

#[test]
fn partial_messages_only_use_what_actually_arrived() {
    // 62 本のチャンクストリームで 16 MiB 近い音声メッセージを名乗るが、実際に送るのは各 128 バイトだけ。
    // 宣言された長さ分を先に確保する実装だと 1 GiB 近くなる。ここでは EOF で静かに終わる。
    let mut input = handshake_bytes();
    for cs in 2u8..=63 {
        input.extend([cs, 0, 0, 0, 0xff, 0xff, 0xff, 0x08, 1, 0, 0, 0]);
        input.extend(std::iter::repeat(0).take(128));
    }
    let (r, _, _) = run(input);
    assert!(matches!(r, Err(Error::Eof)));
}

#[test]
fn buffer_cap_is_enforced() {
    // チャンクサイズを 4 MiB にして、未完了のメッセージを別々のチャンクストリームで積み上げる。
    // 合計が上限 (64 MiB) を超えたところで打ち切られること。
    let mut input = handshake_bytes();
    input.extend(chunked(2, 0, 0x01, 0, &(4u32 * 1024 * 1024).to_be_bytes(), 128));
    for cs in 5u8..=30 {
        input.extend([cs, 0, 0, 0, 0xff, 0xff, 0xff, 0x08, 1, 0, 0, 0]);
        input.extend(std::iter::repeat(0).take(4 * 1024 * 1024));
    }
    let (r, _, _) = run(input);
    match r {
        Err(Error::Protocol(m)) => assert!(m.contains("too much buffered"), "{}", m),
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn set_chunk_size_validation() {
    for bad in [0u32, 0x100_0000, 0xffff_ffff] {
        let mut input = handshake_bytes();
        input.extend(chunked(2, 0, 0x01, 0, &bad.to_be_bytes(), 128));
        let (r, _, _) = run(input);
        assert!(matches!(r, Err(Error::Protocol(_))), "chunk size {:#x}", bad);
    }
}

#[test]
fn arbitrary_garbage_never_panics() {
    // 決まった種の擬似乱数で作ったゴミを大量に食わせる。panic しなければ良い。
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut base = handshake_bytes();
    base.extend(publish_prefix());
    base.extend(metadata(true));
    for _ in 0..300 {
        let mut d = base.clone();
        for _ in 0..(1 + next() % 8) {
            let i = (next() as usize) % d.len();
            d[i] = next() as u8;
        }
        let cut = (next() as usize) % (d.len() + 1);
        d.truncate(cut.max(1));
        let _ = run(d); // 結果は問わない
    }
}
