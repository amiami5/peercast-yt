use super::*;
use crate::reader::Abort;

/// テスト用の atom の木
pub enum A {
    Leaf(&'static [u8], Vec<u8>),
    Parent(&'static [u8], Vec<A>),
}

pub fn ser(a: &A, out: &mut Vec<u8>) {
    match a {
        A::Leaf(id, data) => {
            out.extend_from_slice(&id4(id));
            out.extend_from_slice(&(data.len() as i32).to_le_bytes());
            out.extend_from_slice(data);
        }
        A::Parent(id, ch) => {
            out.extend_from_slice(&id4(id));
            out.extend_from_slice(&(ch.len() as u32 | 0x8000_0000).to_le_bytes());
            for c in ch {
                ser(c, out);
            }
        }
    }
}

fn int(id: &'static [u8], v: i32) -> A {
    A::Leaf(id, v.to_le_bytes().to_vec())
}

fn chr(id: &'static [u8], v: u8) -> A {
    A::Leaf(id, vec![v])
}

fn packet(a: &A) -> Vec<u8> {
    let mut v = Vec::new();
    ser(a, &mut v);
    v.resize(MAX_DATALEN, 0);
    v
}

/// 呼ばれたコールバックを文字列で記録する
#[derive(Default)]
struct TestHost {
    events: Vec<String>,
    root: bool,
    sid: [u8; 16],
    has_ch: bool,
}

fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Host for TestHost {
    fn session_id(&mut self) -> [u8; 16] {
        self.sid
    }
    fn is_root(&mut self) -> bool {
        self.root
    }
    fn time(&mut self) -> u32 {
        1000
    }
    fn log(&mut self, level: Level, msg: &[u8]) {
        self.events.push(format!("log{:?} {}", level, show(msg)));
    }
    fn route_add(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("route {}", id[0]));
        Ok(())
    }
    fn set_update_interval(&mut self, si: i32) -> std::result::Result<(), Abort> {
        self.events.push(format!("updint {}", si));
        Ok(())
    }
    fn upgrade(&mut self, url: &[u8]) -> std::result::Result<(), Abort> {
        self.events.push(format!("upgrade {}", show(url)));
        Ok(())
    }
    fn tracker_update(&mut self) -> std::result::Result<(), Abort> {
        self.events.push("tracker".into());
        Ok(())
    }
    fn root_message(&mut self, msg: &[u8]) -> std::result::Result<(), Abort> {
        self.events.push(format!("rootmsg {}", show(msg)));
        Ok(())
    }
    fn hit(&mut self, hit: &Hit, add: bool) -> std::result::Result<(), Abort> {
        self.events.push(format!("hit {} {:?}", add, hit));
        Ok(())
    }
    fn push(&mut self, ip: Option<Ip>, port: Option<i32>, chan_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("push {:?} {:?} {}", ip, port, chan_id[0]));
        Ok(())
    }
    fn chan_begin(&mut self, chan_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("chan_begin {}", chan_id[0]));
        Ok(())
    }
    fn chan_has_channel(&mut self) -> bool {
        self.has_ch
    }
    fn chan_packet(&mut self, pkt: &Packet) -> std::result::Result<(), Abort> {
        self.events.push(format!("pkt {} {} {} {}", pkt.kind, pkt.pos, pkt.cont, show(&pkt.data)));
        Ok(())
    }
    fn chan_info_string(&mut self, field: InfoField, bytes: &[u8]) -> std::result::Result<(), Abort> {
        self.events.push(format!("info {:?} {:?}", field, bytes));
        Ok(())
    }
    fn chan_info_bitrate(&mut self, bitrate: i32) -> std::result::Result<(), Abort> {
        self.events.push(format!("bitrate {}", bitrate));
        Ok(())
    }
    fn chan_bcid(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("bcid {} {}", id[0], id[1]));
        Ok(())
    }
    fn chan_id(&mut self, id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("chan_id {}", id[0]));
        Ok(())
    }
    fn chan_end(&mut self) -> std::result::Result<(), Abort> {
        self.events.push("chan_end".into());
        Ok(())
    }
    fn broadcast(&mut self, target: Target, pack: &[u8], chan_id: &[u8; 16], dest_id: &[u8; 16]) -> std::result::Result<(), Abort> {
        self.events.push(format!("bcast {:?} {} {} {}", target, pack.len(), chan_id[0], dest_id[0]));
        Ok(())
    }
}

fn run(h: &mut TestHost, a: &A) -> (Result<i32>, State) {
    let mut buf = packet(a);
    let mut st = State::default();
    let r = proc_packet(h, &mut buf, &mut st);
    (r, st)
}

#[test]
fn ok_and_quit() {
    let mut h = TestHost::default();
    assert_eq!(run(&mut h, &int(b"ok", 5)).0, Ok(0));
    assert_eq!(run(&mut h, &int(b"quit", 0)).0, Ok(PCP_ERROR_QUIT));
    assert_eq!(run(&mut h, &int(b"quit", 1003)).0, Ok(1003));
    // 長さが合わない
    assert_eq!(run(&mut h, &chr(b"ok", 5)).0, Err(Error::Stream("checkData: Bad atom data")));
    assert_eq!(run(&mut h, &int(b"xxxx", 5)).0, Err(Error::Stream("Protocol error")));
    assert_eq!(h.events.last().unwrap(), "logError PCP unknown or misplaced atom: xxxx");
}

#[test]
fn atom_container_returns_last_nonzero() {
    let mut h = TestHost::default();
    let a = A::Parent(b"atom", vec![int(b"quit", 7), int(b"ok", 0), A::Leaf(b"mesg", b"hi\0".to_vec())]);
    assert_eq!(run(&mut h, &a).0, Ok(7));
    assert_eq!(h.events, vec!["logDebug PCP got text: hi"]);
}

#[test]
fn nesting_limit() {
    let mut a = int(b"ok", 0);
    for _ in 0..(MAX_PROC_DEPTH + 1) {
        a = A::Parent(b"atom", vec![a]);
    }
    let mut h = TestHost::default();
    assert_eq!(run(&mut h, &a).0, Err(Error::Stream("PCP: atom nesting too deep")));
    let mut a = int(b"ok", 0);
    for _ in 0..MAX_PROC_DEPTH {
        a = A::Parent(b"atom", vec![a]);
    }
    assert_eq!(run(&mut h, &a).0, Ok(0));
}

#[test]
fn host_atoms() {
    let mut h = TestHost::default();
    let a = A::Parent(
        b"host",
        vec![
            int(b"ip", 0x0100007f),
            A::Leaf(b"port", 7144i16.to_le_bytes().to_vec()),
            A::Leaf(b"ip", (0u8..16).collect()),
            A::Leaf(b"port", (-1i16).to_le_bytes().to_vec()),
            A::Leaf(b"port", 80i16.to_le_bytes().to_vec()),
            int(b"numl", 3),
            chr(b"flg1", 0x02),
            A::Leaf(b"vexp", b"YT".to_vec()),
            int(b"zzzz", 1),
        ],
    );
    run(&mut h, &a).0.unwrap();
    assert_eq!(h.events[0], "logDebug PCP skip: zzzz, 0, 4");
    let hit = &h.events[1];
    assert!(hit.starts_with("hit false "), "{}", hit);
    assert!(hit.contains("rhost_ip: [Some(V4(16777343)), Some(V6([15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]))]"), "{}", hit);
    assert!(hit.contains("rhost_port: [Some(7144), Some(80)]"), "{}", hit);
    assert!(hit.contains("version_ex_prefix: Some([89, 84])"), "{}", hit);

    // flg1 がなければ recv (ChanHit::init() の値) なので addHit
    let mut h = TestHost::default();
    run(&mut h, &A::Parent(b"host", vec![int(b"numr", 1)])).0.unwrap();
    assert!(h.events[0].starts_with("hit true "));
}

#[test]
fn chan_atoms() {
    let mut h = TestHost { has_ch: true, ..Default::default() };
    let a = A::Parent(
        b"chan",
        vec![
            A::Leaf(b"id", vec![9; 16]),
            A::Parent(b"info", vec![A::Leaf(b"name", b"abc".to_vec()), int(b"bitr", 500), int(b"zz", 1)]),
            A::Parent(b"trck", vec![A::Leaf(b"titl", b"t\0".to_vec())]),
            A::Leaf(b"key", vec![7; 16]),
            A::Parent(
                b"pkt",
                vec![int(b"type", i32::from_le_bytes(*b"data")), int(b"pos", 100), chr(b"cont", 1), A::Leaf(b"data", b"xyz".to_vec())],
            ),
        ],
    );
    let (r, st) = run(&mut h, &a);
    r.unwrap();
    assert_eq!(
        h.events,
        vec![
            "chan_begin 0",
            "chan_id 9",
            "info Name [97, 98, 99]",
            "bitrate 500",
            "info TrackTitle [116, 0]",
            "bcid 0 7",
            "pkt 2 100 true xyz",
            "chan_end",
        ]
    );
    assert_eq!(st.bcs.stream_pos, 100);

    // チャンネルがなければ pkt は「知らない atom」
    let mut h = TestHost::default();
    let a = A::Parent(b"chan", vec![A::Parent(b"pkt", vec![])]);
    assert_eq!(run(&mut h, &a).0, Err(Error::Stream("Protocol error")));
    assert_eq!(h.events, vec!["chan_begin 0", "logError PCP unknown or misplaced atom: pkt, 0, 0"]);
}

#[test]
fn long_string_is_error() {
    let mut h = TestHost::default();
    let ok = A::Parent(b"chan", vec![A::Parent(b"info", vec![A::Leaf(b"name", vec![b'a'; 256])])]);
    run(&mut h, &ok).0.unwrap();
    let ng = A::Parent(b"chan", vec![A::Parent(b"info", vec![A::Leaf(b"name", vec![b'a'; 257])])]);
    assert_eq!(run(&mut h, &ng).0, Err(Error::Stream("checkData: Bad atom data")));
}

/// pcpstream_unittest.cpp の smallDataPacket / bigDataPacket
#[test]
fn pkt_data_size() {
    let mut h = TestHost { has_ch: true, ..Default::default() };
    let small = A::Parent(b"chan", vec![A::Parent(b"pkt", vec![A::Leaf(b"data", b"AA".to_vec())])]);
    run(&mut h, &small).0.unwrap();
    // 16384 バイトを超える data はパケットに入らないので、見出しだけ作る
    let mut buf = Vec::new();
    ser(&A::Parent(b"chan", vec![]), &mut buf);
    buf[4] = 1;
    ser(&A::Parent(b"pkt", vec![]), &mut buf);
    let n = buf.len();
    buf[n - 4] = 1;
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(10 * 65536i32).to_le_bytes());
    buf.resize(MAX_DATALEN, b'A');
    let mut st = State::default();
    assert_eq!(proc_packet(&mut h, &mut buf, &mut st), Err(Error::Stream("Data size too large")));
}

#[test]
fn root_atoms() {
    let mut h = TestHost::default();
    let a = A::Parent(
        b"root",
        vec![
            int(b"uint", 120),
            A::Leaf(b"url", b"download\0".to_vec()),
            int(b"chkv", 1219),
            int(b"next", 60),
            A::Leaf(b"mesg", b"hello".to_vec()),
            A::Parent(b"upd", vec![int(b"x", 1)]),
        ],
    );
    let (r, st) = run(&mut h, &a);
    r.unwrap();
    assert_eq!(
        h.events,
        vec![
            "updint 120",
            "logDebug PCP got new host update interval: 120s",
            "upgrade http://www.peercast.org/download",
            "logDebug PCP got version check: 1219 / 1218",
            "logDebug PCP expecting next root packet in 60s",
            "rootmsg hello",
            "tracker",
        ]
    );
    assert_eq!(st.next_root_packet, 1060);

    let mut h = TestHost { root: true, ..Default::default() };
    assert_eq!(run(&mut h, &a).0, Err(Error::Stream("Unauthorized root message")));
}

#[test]
fn helo_writes_oleh_into_packet() {
    let mut h = TestHost { sid: [0xab; 16], ..Default::default() };
    // helo のあとの atom の先頭 32 バイトは oleh で上書きされ、その続き (mesg の中身の途中) を
    // 次の atom として読む
    let a = A::Parent(b"atom", vec![A::Parent(b"helo", vec![int(b"port", 1)]), A::Leaf(b"mesg", vec![b'x'; 40])]);
    let mut buf = packet(&a);
    let mut st = State::default();
    assert_eq!(proc_packet(&mut h, &mut buf, &mut st), Err(Error::Stream("Protocol error")));
    assert_eq!(h.events, vec!["logError PCP unknown or misplaced atom: xxxx"]);
    let p = 8 + 8 + 12;
    assert_eq!(&buf[p..p + 8], b"oleh\x01\x00\x00\x80");
    assert_eq!(&buf[p + 8..p + 16], b"sid\x00\x10\x00\x00\x00");
    assert_eq!(&buf[p + 16..p + 32], &[0xab; 16]);
}

#[test]
fn broadcast() {
    let mut h = TestHost { sid: [1; 16], ..Default::default() };
    let a = A::Parent(
        b"bcst",
        vec![
            chr(b"ttl", 7),
            chr(b"hops", 2),
            A::Leaf(b"from", vec![5; 16]),
            chr(b"grp", PCP_BCST_GROUP_TRACKERS as u8),
            A::Leaf(b"cid", vec![3; 16]),
            int(b"vers", 1218),
            A::Leaf(b"vexp", b"YT".to_vec()),
            A::Leaf(b"vexn", 50i16.to_le_bytes().to_vec()),
            A::Leaf(b"mesg", b"m\0".to_vec()),
        ],
    );
    let (r, st) = run(&mut h, &a);
    assert_eq!(r, Ok(0));
    assert_eq!(st.bcs.num_hops, 3);
    assert_eq!(st.bcs.group, 2);
    assert_eq!(
        h.events,
        vec![
            "route 5",
            "logDebug PCP bcst version 1218",
            "logDebug PCP bcst ex version YT50",
            "logDebug PCP got text: m",
            "logDebug PCP bcst: group=2, hops=3, ver=1218, from=05050505050505050505050505050505, dest=00000000000000000000000000000000",
            "bcast Up 125 3 0",
            "bcast Cout 125 3 0",
            "bcast Cin 125 3 0",
        ]
    );

    // 自分が出したもの
    let mut h = TestHost { sid: [5; 16], ..Default::default() };
    assert_eq!(run(&mut h, &a).0, Ok(PCP_ERROR_BCST + PCP_ERROR_LOOPBACK));
    assert_eq!(h.events.last().unwrap(), "logError BCST loopback");
}

#[test]
fn char_is_signed() {
    let mut h = TestHost::default();
    let a = A::Parent(b"bcst", vec![chr(b"ttl", 0xff), chr(b"hops", 0x7f), chr(b"grp", 0xff)]);
    let (r, st) = run(&mut h, &a);
    r.unwrap();
    assert_eq!(st.bcs.group, -1);
    assert_eq!(st.bcs.num_hops, 128);
    // ttl は -2 なので中継しない
    assert!(!h.events.iter().any(|e| e.starts_with("bcast")));
}

#[test]
fn skip_loop_stops_at_end_of_buffer() {
    // helo の oleh で上書きしたあとに、子の数の大きい host の見出しを置く
    let mut junk = vec![0u8; 24];
    junk.extend_from_slice(b"host");
    junk.extend_from_slice(&(0xffff_ffffu32).to_le_bytes());
    let a = A::Parent(b"atom", vec![A::Leaf(b"helo", vec![]), A::Leaf(b"xxxx", junk)]);
    let mut h = TestHost::default();
    let (r, _) = run(&mut h, &a);
    assert_eq!(r, Ok(0));
    assert!(h.events.last().unwrap().starts_with("hit true "));
    assert!(h.events.len() < 3000, "{}", h.events.len());
}

#[test]
fn mem_stream_semantics() {
    let mut b = [1u8, 2, 3, 4, 5];
    let mut m = MemStream::new(&mut b);
    assert_eq!(m.read(3).unwrap(), vec![1, 2, 3]);
    assert_eq!(m.read(3).unwrap(), vec![0, 0, 0]);
    assert_eq!(m.pos, 3);
    m.skip(2).unwrap();
    assert_eq!(m.pos, 5);
    assert!(m.write(&[1]).is_err());
    assert_eq!(m.skip(-1), Err(Error::Stream("Stream::skip: negative length")));

    // 4096 バイトずつ読むので、読めない塊のあとの短い塊は読める
    let mut b = vec![0u8; 5000];
    let mut m = MemStream::new(&mut b);
    m.pos = 4000;
    m.skip(4096 + 100).unwrap();
    assert_eq!(m.pos, 4100);
}

/// 変異させたパケットでパニックしないこと
#[test]
fn fuzz_no_panic() {
    let seeds = [
        A::Parent(b"atom", vec![A::Parent(b"bcst", vec![chr(b"ttl", 3), A::Parent(b"chan", vec![A::Parent(b"info", vec![A::Leaf(b"name", b"n".to_vec())])])])]),
        A::Parent(b"host", vec![int(b"ip", 1), A::Leaf(b"port", vec![1, 0])]),
        A::Parent(b"root", vec![int(b"next", 1), A::Leaf(b"mesg", b"q".to_vec())]),
    ];
    let mut rng = 12345u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for i in 0..20000 {
        let mut buf = packet(&seeds[i % seeds.len()]);
        for _ in 0..(next() % 6) {
            let p = (next() % 80) as usize;
            buf[p] = next() as u8;
        }
        let mut h = TestHost { has_ch: i % 2 == 0, ..Default::default() };
        let mut st = State::default();
        let _ = proc_packet(&mut h, &mut buf, &mut st);
    }
}
