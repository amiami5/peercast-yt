//! チャンネルの配信元 (core/common/channel.cpp の `PeercastSource`、url.cpp の `URLSource`、icy.cpp の
//! `ICYSource`、httppush.cpp の `HTTPPushSource`) と、入力をパケットに切り分けるもの (`ChannelStream` の
//! 派生クラス: `PCPStream`、各メディアの解析器 (段階 4 の `crate::media`)、`RawStream`)。

use std::sync::Arc;

use super::chaninfo::{self as ci, ChanInfo};
use super::channel::{self as chn, Channel};
use super::error::{Error, Result};
use super::host::Host;
use super::http::{Http, PCX_AGENT};
use super::packetbuf::{self as pb, ChanPacket};
use super::pcpconst::*;
use super::pcpstream::PcpStream;
pub use super::pcpstream::PcpShared;
use super::pcstr::StrType;
use super::peercast::Peercast;
use super::socket::ClientSocket;
use super::stream::{Stat, Stream, StreamExt};
use super::sys;
use crate::media;
use crate::pcp::write::AtomBuf;
use crate::reader::{Abort, Reader};

/// 配信元の種類
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// ほかのノードから中継する
    Peercast,
    /// URL から取る (`URLSource`)
    Url(Vec<u8>),
    /// ShoutCast / Icecast から受ける (`ICYSource`)
    Icy,
    /// HTTP の POST で受ける (`HTTPPushSource`)。chunked かどうか
    HttpPush(bool),
}

/// `ChannelStream` の状態のうち、`updateStatus` で使うもの
#[derive(Default)]
struct StatusState {
    num_relays: i32,
    num_listeners: i32,
    is_playing: bool,
    fw_state: i32,
    last_update: u32,
}

enum Kind {
    Pcp(Box<PcpStream>),
    Media(media::Parser),
    Raw,
}

/// `ChannelStream`
pub struct SourceStream {
    kind: Kind,
    status: StatusState,
}

impl SourceStream {
    /// `Channel::createSource`
    pub fn create(ch: &Channel) -> SourceStream {
        let (proto, ct, remote) = {
            let st = ch.st();
            (st.info.src_protocol, st.info.content_type.data.clone(), st.remote_id)
        };
        let kind = if proto == ci::SP_PCP {
            crate::log_info!("Channel is PCP");
            Kind::Pcp(Box::new(PcpStream::new(remote)))
        } else if ct == ci::T_MP3 {
            crate::log_info!("Channel is MP3 - meta: {}", ch.st().icy_meta_interval);
            Kind::Media(media::Parser::new(media::Kind::Mp3))
        } else if ct == ci::T_FLV {
            crate::log_info!("Channel is FLV");
            Kind::Media(media::Parser::new(media::Kind::Flv))
        } else if ct == ci::T_OGG || ct == ci::T_OGM {
            crate::log_info!("Channel is OGG");
            Kind::Media(media::Parser::new(media::Kind::Ogg))
        } else if ct == ci::T_MKV {
            crate::log_info!("Channel is MKV");
            Kind::Media(media::Parser::new(media::Kind::Mkv))
        } else if ct == ci::T_WEBM {
            crate::log_info!("Channel is WebM");
            Kind::Media(media::Parser::new(media::Kind::Mkv))
        } else if ct == ci::T_MP4 {
            crate::log_info!("Channel is MP4");
            Kind::Media(media::Parser::new(media::Kind::Mp4))
        } else {
            crate::log_info!("Channel is Raw");
            Kind::Raw
        };
        SourceStream { kind, status: StatusState::default() }
    }

    /// PCP の配信元なら、ほかのスレッドから送るためのもの
    pub fn pcp_shared(&self) -> Option<Arc<PcpShared>> {
        match &self.kind {
            Kind::Pcp(p) => Some(p.shared.clone()),
            _ => None,
        }
    }

    /// `readHeader`
    pub fn read_header(&mut self, pc: &Arc<Peercast>, ch: &Arc<Channel>, input: &mut dyn Stream) -> Result<()> {
        match &mut self.kind {
            Kind::Media(p) => {
                let mut h = MediaCtx::new(pc, ch, input);
                let r = p.read_header(&mut h);
                h.check(r)
            }
            _ => Ok(()),
        }
    }

    /// `readPacket`: 誤りなら 0 以外 (PCP の番号)
    pub fn read_packet(&mut self, pc: &Arc<Peercast>, ch: &Arc<Channel>, input: &mut dyn Stream) -> Result<i32> {
        match &mut self.kind {
            Kind::Pcp(p) => {
                let mut bcs = crate::pcp::BroadcastState::default();
                Ok(p.read_packet(pc, input, &mut bcs))
            }
            Kind::Media(p) => {
                let mut h = MediaCtx::new(pc, ch, input);
                let r = p.read_packet(&mut h);
                h.check(r)?;
                Ok(0)
            }
            Kind::Raw => {
                read_raw(ch, input)?;
                Ok(0)
            }
        }
    }

    /// `flush`: PCP なら送り残しを書く
    pub fn flush(&mut self, out: &mut dyn Stream) -> Result<()> {
        match &self.kind {
            Kind::Pcp(p) => p.flush(out),
            _ => Ok(()),
        }
    }

    /// `ChannelStream::getStatus`: 状態が変わっていれば、上流に送るパケット
    fn get_status(&mut self, pc: &Peercast, ch: &Channel) -> Option<ChanPacket> {
        let ctime = sys::get_time();
        let id = ch.id();
        if !pc.chanmgr.has_hitlist_by_id(&id) {
            return None;
        }
        let new_l = ch.local_listeners(pc, true);
        let new_r = ch.local_relays(pc, true);
        let playing = ch.is_playing();
        let ipv = ch.st().ip_version;
        let fw = pc.servmgr.get_firewall(ipv);
        let s = &mut self.status;
        let changed = s.num_listeners != new_l || s.num_relays != new_r || playing != s.is_playing || fw != s.fw_state || ctime.wrapping_sub(s.last_update) > 120;
        if !(changed && ctime.wrapping_sub(s.last_update) > 10) {
            return None;
        }
        s.num_listeners = new_l;
        s.num_relays = new_r;
        s.is_playing = playing;
        s.fw_state = fw;
        s.last_update = ctime;
        let hit = ch.local_hit(pc, false);
        let mut out = AtomBuf::default();
        out.parent(PCP_BCST, 10);
        out.char(PCP_BCST_GROUP, PCP_BCST_GROUP_TRACKERS as u8);
        out.char(PCP_BCST_HOPS, 0);
        out.char(PCP_BCST_TTL, 11);
        out.bytes(PCP_BCST_FROM, &pc.servmgr.session_id);
        out.int(PCP_BCST_VERSION, PCP_CLIENT_VERSION as i32);
        out.int(PCP_BCST_VERSION_VP, PCP_CLIENT_VERSION_VP as i32);
        out.bytes(PCP_BCST_VERSION_EX_PREFIX, PCP_CLIENT_VERSION_EX_PREFIX);
        out.short(PCP_BCST_VERSION_EX_NUMBER, PCP_CLIENT_VERSION_EX_NUMBER as i16);
        out.bytes(PCP_BCST_CHANID, &id);
        hit.write_atoms(&mut out, &[0; 16]);
        ChanPacket::new(pb::T_PCP, &out.0, 0).ok()
    }

    /// `ChannelStream::updateStatus`
    pub fn update_status(&mut self, pc: &Peercast, ch: &Channel) {
        if let Some(pack) = self.get_status(pc, ch) {
            if !ch.is_broadcasting() {
                let cnt = pc.chanmgr.broadcast_packet_up(&pack, &ch.id(), &pc.servmgr.session_id, &[0; 16]);
                crate::log_info!("Sent channel status update to {} clients", cnt);
            }
        }
    }
}

/// `ChannelStream::readRaw`: 8192 バイトずつ読んでパケットにする
fn read_raw(ch: &Channel, input: &mut dyn Stream) -> Result<()> {
    let mut data = vec![0u8; 8192];
    input.read(&mut data)?;
    let pos = ch.st().stream_pos;
    let mut pack = ChanPacket::new(pb::T_DATA, &data, pos)?;
    ch.new_packet(&mut pack);
    ch.check_read_delay(pack.len());
    let mut st = ch.st();
    st.stream_pos = st.stream_pos.wrapping_add(pack.len());
    Ok(())
}

/// 解析器から見たチャンネルと入力 (C++ の `rustbridge::MediaHost`)
struct MediaCtx<'a> {
    pc: &'a Arc<Peercast>,
    ch: &'a Arc<Channel>,
    input: &'a mut dyn Stream,
    error: Option<Error>,
}

impl<'a> MediaCtx<'a> {
    fn new(pc: &'a Arc<Peercast>, ch: &'a Arc<Channel>, input: &'a mut dyn Stream) -> MediaCtx<'a> {
        MediaCtx { pc, ch, input, error: None }
    }

    fn guard<T>(&mut self, r: Result<T>) -> std::result::Result<T, Abort> {
        r.map_err(|e| {
            self.error = Some(e);
            Abort
        })
    }

    /// 解析器の結果を、C++ 版と同じ例外にする
    fn check(&mut self, r: media::Result<()>) -> Result<()> {
        match r {
            Ok(()) => Ok(()),
            Err(media::Error::Abort) => Err(self.error.take().unwrap_or_else(|| Error::stream("media: aborted"))),
            Err(media::Error::Stream(m)) => Err(Error::stream(m)),
        }
    }

    fn checked(len: usize) -> Result<()> {
        if len > pb::MAX_DATALEN {
            return Err(Error::stream("Packet data too large"));
        }
        Ok(())
    }
}

impl Reader for MediaCtx<'_> {
    fn read_char(&mut self) -> std::result::Result<u8, Abort> {
        let r = self.input.read_char();
        self.guard(r)
    }

    fn read_exact(&mut self, n: usize) -> std::result::Result<Vec<u8>, Abort> {
        let r = self.input.read_n(n);
        self.guard(r)
    }

    fn read_some(&mut self, n: usize) -> std::result::Result<Vec<u8>, Abort> {
        let mut v = vec![0u8; n];
        let r = self.input.read(&mut v).map(|k| {
            v.truncate(k);
            v
        });
        self.guard(r)
    }

    fn eof(&mut self) -> std::result::Result<bool, Abort> {
        let r = self.input.eof();
        self.guard(r)
    }
}

impl media::Host for MediaCtx<'_> {
    fn ready(&mut self) -> std::result::Result<bool, Abort> {
        Ok(self.input.read_ready(0))
    }

    fn raise_bitrate(&mut self) -> std::result::Result<(), Abort> {
        let avg = self.input.stat().map_or(0, |s| s.bytes_in_per_sec_avg());
        let mut info = self.ch.info();
        let nb = (avg / 1000 * 8) as i32;
        if nb > info.bitrate {
            info.bitrate = nb;
            self.ch.update_info(self.pc, &info);
        }
        Ok(())
    }

    fn packet(&mut self, data: &[u8], cont: bool, read_delay: bool) -> std::result::Result<(), Abort> {
        let r = (|| -> Result<()> {
            Self::checked(data.len())?;
            let pos = self.ch.st().stream_pos;
            let mut pack = ChanPacket { ty: pb::T_DATA, pos, sync: 0, cont, data: data.to_vec() };
            self.ch.new_packet(&mut pack);
            if read_delay {
                self.ch.check_read_delay(pack.len());
            }
            let mut st = self.ch.st();
            st.stream_pos = st.stream_pos.wrapping_add(pack.len());
            Ok(())
        })();
        self.guard(r)
    }

    fn head(&mut self, kind: media::HeadKind, data: &[u8]) -> std::result::Result<(), Abort> {
        let r = (|| -> Result<()> {
            let ch = self.ch;
            match kind {
                media::HeadKind::NewStream => {
                    Self::checked(data.len())?;
                    ch.raw_data.init();
                    let mut pack = {
                        let mut st = ch.st();
                        st.stream_index = st.stream_index.wrapping_add(1);
                        st.head_pack = ChanPacket { ty: pb::T_HEAD, pos: 0, sync: 0, cont: st.head_pack.cont, data: data.to_vec() };
                        st.head_pack.clone()
                    };
                    ch.new_packet(&mut pack);
                    let mut st = ch.st();
                    st.head_pack.sync = pack.sync;
                    st.stream_pos = pack.len();
                }
                media::HeadKind::Mkv => {
                    Self::checked(data.len())?;
                    let mut pack = {
                        let mut st = ch.st();
                        st.stream_index = st.stream_index.wrapping_add(1);
                        st.stream_pos = 0;
                        ChanPacket { ty: pb::T_HEAD, pos: 0, sync: 0, cont: false, data: data.to_vec() }
                    };
                    ch.raw_data.init();
                    ch.st().head_pack = pack.clone();
                    ch.new_packet(&mut pack);
                    let mut st = ch.st();
                    st.stream_pos = st.stream_pos.wrapping_add(pack.len());
                }
                media::HeadKind::Ogg => {
                    let mut pack = {
                        let mut st = ch.st();
                        st.head_pack.ty = pb::T_HEAD;
                        st.head_pack.pos = st.stream_pos;
                        st.start_time = sys::get_dtime();
                        st.stream_pos = st.stream_pos.wrapping_add(st.head_pack.len());
                        st.head_pack.clone()
                    };
                    ch.new_packet(&mut pack);
                }
            }
            Ok(())
        })();
        self.guard(r)
    }

    fn head_len(&mut self) -> u32 {
        self.ch.st().head_pack.len()
    }

    fn head_clear(&mut self) {
        self.ch.st().head_pack.data.clear();
    }

    fn head_append(&mut self, data: &[u8]) -> std::result::Result<(), Abort> {
        let r = (|| -> Result<()> {
            let mut st = self.ch.st();
            if st.head_pack.data.len() + data.len() > pb::MAX_DATALEN {
                return Err(Error::stream("OGG packet too big for headPack"));
            }
            st.head_pack.data.extend_from_slice(data);
            Ok(())
        })();
        self.guard(r)
    }

    fn set_bitrate(&mut self, bitrate: i32) -> std::result::Result<(), Abort> {
        let mut info = self.ch.info();
        info.bitrate = bitrate;
        self.ch.update_info(self.pc, &info);
        Ok(())
    }

    fn ogg_set_info(&mut self, bitrate: i32, ogm: bool) {
        let mut st = self.ch.st();
        st.info.bitrate = bitrate;
        if ogm {
            st.info.content_type.assign(ci::T_OGM);
        }
    }

    fn set_track(&mut self, t: &media::Track) -> std::result::Result<(), Abort> {
        let mut info = self.ch.info();
        info.track.clear();
        let set = |dst: &mut super::pcstr::PcString, v: &Option<Vec<u8>>| {
            if let Some(v) = v {
                dst.set(v, StrType::Ascii);
                dst.convert_to(StrType::Unicode);
            }
        };
        set(&mut info.track.artist, &t.artist);
        set(&mut info.track.title, &t.title);
        set(&mut info.track.genre, &t.genre);
        set(&mut info.track.contact, &t.contact);
        set(&mut info.track.album, &t.album);
        self.ch.update_info(self.pc, &info);
        Ok(())
    }

    fn mp3_metadata(&mut self, buf: &[u8]) -> std::result::Result<(), Abort> {
        let b = &buf[..buf.len().min(1024)];
        let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
        self.ch.process_mp3_metadata(self.pc, b);
        Ok(())
    }

    fn icy_meta_interval(&mut self) -> i32 {
        self.ch.st().icy_meta_interval
    }

    fn read_delay(&mut self) -> bool {
        self.ch.st().read_delay
    }

    fn dtime(&mut self) -> f64 {
        sys::get_dtime()
    }

    fn time(&mut self) -> u32 {
        sys::get_time()
    }

    fn sleep(&mut self, ms: i32) {
        sys::sleep(ms.max(0) as u32);
    }

    fn sleep_until(&mut self, t: f64) {
        self.ch.sleep_until(t);
    }

    fn log(&mut self, level: media::LogLevel, msg: &str) {
        let l = match level {
            media::LogLevel::Trace => super::log::Level::Trace,
            media::LogLevel::Debug => super::log::Level::Debug,
            media::LogLevel::Info => super::log::Level::Info,
            media::LogLevel::Warn => super::log::Level::Warn,
            media::LogLevel::Error => super::log::Level::Error,
        };
        super::log::add_log(l, msg.as_bytes());
    }
}

/// `Dechunker`: chunked 転送の中身を読む
pub struct Dechunker<'a> {
    input: &'a mut dyn Stream,
    buf: std::collections::VecDeque<u8>,
    eof: bool,
}

/// 1 チャンクの大きさの上限
const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024;

impl<'a> Dechunker<'a> {
    pub fn new(input: &'a mut dyn Stream) -> Dechunker<'a> {
        Dechunker { input, buf: Default::default(), eof: false }
    }

    fn next_chunk(&mut self) -> Result<()> {
        let mut r = super::stream::StreamReader::new(&mut *self.input);
        let (data, err) = crate::dechunk::next_chunk(&mut r, MAX_CHUNK_SIZE);
        self.buf.extend(data);
        match err {
            None => Ok(()),
            Some(crate::dechunk::Error::Abort) => Err(r.take_error()),
            Some(crate::dechunk::Error::Protocol) => Err(Error::stream("Protocol error")),
            Some(crate::dechunk::Error::TooLarge) => Err(Error::stream("Chunk size too large")),
            Some(crate::dechunk::Error::Closed) => {
                self.eof = true;
                Err(Error::stream("Closed on read"))
            }
            Some(crate::dechunk::Error::Premature) => Err(Error::stream("Premature end")),
        }
    }
}

impl Stream for Dechunker<'_> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.eof {
            return Err(Error::stream("Closed on read"));
        }
        while self.buf.len() < out.len() {
            self.next_chunk()?;
        }
        for b in out.iter_mut() {
            *b = self.buf.pop_front().unwrap_or(0);
        }
        Ok(out.len())
    }

    fn write(&mut self, _data: &[u8]) -> Result<()> {
        Err(Error::stream("Stream can`t write"))
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.eof)
    }

    fn stat(&self) -> Option<&Stat> {
        self.input.stat()
    }
}

/// 配信元の本体 (`ChannelSource::stream`)
pub fn stream(pc: &Arc<Peercast>, ch: &Arc<Channel>, src: &Source) {
    match src {
        Source::Peercast => peercast_stream(pc, ch),
        Source::Url(u) => url_stream(pc, ch, u),
        Source::Icy => icy_stream(pc, ch),
        Source::HttpPush(chunked) => http_push_stream(pc, ch, *chunked),
    }
}

fn set_src_stat(ch: &Channel, s: Option<Arc<Stat>>) {
    *ch.src_stat.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// ソケットから読んでチャンネルに流す (`readStream` と、PCP の配信元の登録)
fn run_read_stream(pc: &Arc<Peercast>, ch: &Arc<Channel>, input: &mut dyn Stream, source: &mut SourceStream) -> i32 {
    *ch.source_stream.lock().unwrap_or_else(|e| e.into_inner()) = source.pcp_shared();
    let r = ch.read_stream(pc, ch, input, source);
    r
}

/// `ICYSource::stream`
fn icy_stream(pc: &Arc<Peercast>, ch: &Arc<Channel>) {
    let sock = ch.sock.lock().unwrap_or_else(|e| e.into_inner()).take();
    match sock {
        None => crate::log_error!("Channel aborted: ICY channel has no socket"),
        Some(mut sock) => {
            ch.st().src_sock = Some(sock.host);
            ch.reset_play_time();
            ch.set_status(pc, chn::S_BROADCASTING);
            let mut source = SourceStream::create(ch);
            run_read_stream(pc, ch, &mut sock, &mut source);
            sock.close();
            ch.st().src_sock = None;
        }
    }
    ch.set_status(pc, chn::S_CLOSING);
}

/// `HTTPPushSource::stream`
fn http_push_stream(pc: &Arc<Peercast>, ch: &Arc<Channel>, chunked: bool) {
    let sock = ch.sock.lock().unwrap_or_else(|e| e.into_inner()).take();
    match sock {
        None => crate::log_error!("Channel aborted: HTTP Push channel has no socket"),
        Some(mut sock) => {
            set_src_stat(ch, sock.stat().map(|_| sock.shared_stat()));
            ch.st().src_sock = Some(sock.host);
            ch.reset_play_time();
            ch.set_status(pc, chn::S_BROADCASTING);
            let mut source = SourceStream::create(ch);
            if chunked {
                let mut d = Dechunker::new(&mut sock);
                run_read_stream(pc, ch, &mut d, &mut source);
            } else {
                run_read_stream(pc, ch, &mut sock, &mut source);
            }
            set_src_stat(ch, None);
            sock.close();
            ch.st().src_sock = None;
        }
    }
    ch.set_status(pc, chn::S_CLOSING);
}

/// 自分のグローバル IP か
fn is_own_global_ip(pc: &Peercast, ip: &super::host::Ip) -> bool {
    let sh = pc.servmgr.settings().server_host;
    sh.ip.is_global() && sh.ip == *ip
}

/// `Channel::connectFetch`
fn connect_fetch(ch: &Channel) -> Result<ClientSocket> {
    let (sh, longer) = {
        let st = ch.st();
        (st.source_host.host, st.source_host.tracker || st.source_host.yp)
    };
    let mut sock = ClientSocket::new();
    if longer {
        sock.set_read_timeout(30000);
        sock.set_write_timeout(30000);
        crate::log_info!("Channel using longer timeouts");
    }
    if !sh.ip.is_ipv4_mapped() {
        ch.st().ip_version = chn::IP_V6;
    }
    sock.connect(sh)?;
    Ok(sock)
}

/// `Channel::handshakeFetch`: 0 なら続ける。そうでなければ HTTP の状態の番号
fn handshake_fetch(pc: &Arc<Peercast>, ch: &Channel, sock: &mut ClientSocket) -> Result<i32> {
    let (id, pos, ipv) = {
        let st = ch.st();
        (st.info.id, st.stream_pos, st.ip_version)
    };
    sock.write_line(format!("GET /channel/{} HTTP/1.0", ci::id_str(&id)))?;
    sock.write_line(format!("{} {}", PCX_HS_POS, pos))?;
    sock.write_line(format!("{} {}", PCX_HS_PCP, if ipv == chn::IP_V4 { 1 } else { 100 }))?;
    sock.write_line("")?;
    let r;
    {
        let mut http = Http::new(sock);
        r = http.read_response()?;
        crate::log_info!("Got response: {}", r);
        while http.next_header()? {
            let arg = match http.arg_str() {
                Some(a) => a.to_vec(),
                None => continue,
            };
            if http.is_header(PCX_HS_POS) {
                ch.st().stream_pos = crate::http::atoi(&arg) as u32;
            } else {
                let mut st = ch.st();
                super::servent::read_icy_header(&http, &mut st.info, None);
            }
            crate::log_info!("Channel fetch: {}", String::from_utf8_lossy(&http.cmd_line));
        }
    }
    if r != 200 && r != 503 {
        return Ok(r);
    }
    if ch.raw_data.latest_pos() > ch.st().stream_pos {
        ch.raw_data.init();
    }
    let (proto, trusted) = {
        let st = ch.st();
        (st.info.src_protocol, st.source_host.yp || st.source_host.tracker)
    };
    if proto == ci::SP_PCP {
        let rhost = sock.host;
        let (rid, _agent) = super::servent::handshake_outgoing_pcp(pc, sock, rhost, trusted)?;
        ch.st().remote_id = rid;
    }
    Ok(0)
}

/// 入力元がすぐに終わったときに、つなぎ直すまで待つ時間を決める (Rust 版で足した)。
///
/// C++ 版は、入力元がエラーなしですぐに終わると待たずにつなぎ直した。このため、すぐに終わる
/// 入力元 (中身のない応答、自分自身へのリダイレクト、すぐに終わる外部のプログラム、つないで
/// すぐに切るノードなど) で、接続やプログラムの起動を休みなく繰り返した。
/// すぐに (`QUICK_MS` 未満で) 終わるのが続くと、2 回目から 1 秒、2 秒、4 秒と延ばす (最大 30 秒)。
/// 1 回目は待たないので、1 回のリダイレクトなどは今までと同じ。長く続いたあとに終わったなら戻す。
#[derive(Debug, Default)]
struct RetryDelay {
    quick_ends: u32,
}

impl RetryDelay {
    const QUICK_MS: u128 = 10_000;
    const MAX_MS: u32 = 30_000;

    /// 入力が `elapsed_ms` のあいだ続いて終わったとき、次につなぐまで待つ時間 (ms)
    fn next(&mut self, elapsed_ms: u128) -> u32 {
        if elapsed_ms >= Self::QUICK_MS {
            self.quick_ends = 0;
            return 0;
        }
        self.quick_ends = self.quick_ends.saturating_add(1);
        match self.quick_ends {
            1 => 0,
            n => (1000u32 << (n - 2).min(5)).min(Self::MAX_MS),
        }
    }

    /// `start` から続いた入力が終わったあと、決めた時間だけ待つ。`stop` が true になったらやめる
    fn wait(&mut self, start: std::time::Instant, stop: impl Fn() -> bool) {
        let ms = self.next(start.elapsed().as_millis());
        if ms == 0 {
            return;
        }
        crate::log_info!("Channel source ended quickly; retrying in {} sec", ms / 1000);
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
        while !stop() {
            let now = std::time::Instant::now();
            if now >= end {
                break;
            }
            sys::sleep((end - now).as_millis().min(200) as u32);
        }
    }
}

/// `PeercastSource::stream`: ほかのノードから中継する
fn peercast_stream(pc: &Arc<Peercast>, ch: &Arc<Channel>) {
    if !pc.servmgr.settings().server_host.ip.is_global() {
        crate::log_info!("Checking own global IP ...");
        if let Err(e) = pc.servmgr.check_firewall(pc) {
            crate::log_error!("checkFirewall: {}", e);
        }
        if !pc.servmgr.settings().server_host.ip.is_global() {
            crate::log_error!("Could not determine own global IP. LAN relaying may not work.");
        }
    }

    let mut num_yp_tries = 0;
    let mut retry = RetryDelay::default();
    while ch.thread.active() {
        ch.st().source_host = super::chanhit::ChanHit::new();
        ch.set_status(pc, chn::S_SEARCHING);
        crate::log_info!("Channel searching for hit..");
        let mut sock: Option<ClientSocket> = None;
        loop {
            if let Some(ps) = ch.push_sock.lock().unwrap_or_else(|e| e.into_inner()).take() {
                ch.st().source_host.host = ps.host;
                sock = Some(ps);
                break;
            }
            {
                let mut st = ch.st();
                if st.designated_host.host.ip.is_set() {
                    st.source_host = st.designated_host.clone();
                    st.designated_host = super::chanhit::ChanHit::new();
                    break;
                }
            }
            let old = ch.st().source_host.clone();
            let picked = super::channel::pick_from_hit_list(pc, ch, &old);
            ch.st().source_host = picked;

            // チャンネルの一覧に尋ねる
            if !ch.st().source_host.host.ip.is_set() {
                let info = ch.info();
                let ent = pc.servmgr.channel_directory.find_entry(&info.id);
                crate::log_debug!("consulting chandir for {}", super::chanmgr::ch_name(&info));
                if let Some(ent) = &ent {
                    let rh = pc.yplist.root_host_of(&ent.feed_url);
                    crate::log_debug!(
                        "feedUrlToRooHost('{}') = '{}'",
                        String::from_utf8_lossy(&ent.feed_url),
                        String::from_utf8_lossy(&rh)
                    );
                    if !rh.is_empty() {
                        crate::log_info!("Root host for channel {} set to '{}'", super::chanmgr::ch_name(&info), String::from_utf8_lossy(&rh));
                        ch.st().root_host = rh;
                    }
                }
                if let Some(ent) = ent.filter(|e| !e.tip.is_empty()) {
                    crate::log_debug!("チャンネルフィードで {} のトラッカーが見付かりました。", super::chanmgr::ch_name(&info));
                    let host = Host::from_string(&ent.tip, super::servmgr::DEFAULT_PORT);
                    if host.port == 0 {
                        crate::log_debug!("ポート0のトラッカーIPはホストキャッシュに登録しない。(チャンネルフィードから)");
                    } else if is_own_global_ip(pc, &host.ip) {
                        crate::log_debug!(
                            "{}: Tracker has the same global IP {} as local host. Not connecting.",
                            super::chanmgr::ch_name(&info),
                            host.ip.str()
                        );
                    } else {
                        let sh = {
                            let mut st = ch.st();
                            st.source_host.host = host;
                            st.source_host.rhost[0] = host;
                            st.source_host.tracker = true;
                            st.source_host.clone()
                        };
                        let sid = pc.servmgr.session_id;
                        pc.chanmgr.with_hitlist(&info, |chl| chl.add_hit(&sh, &sid));
                        break;
                    }
                }
            }

            // トラッカーがなければ YP に尋ねる
            if !ch.st().source_host.host.ip.is_set() {
                let rh = ch.st().root_host.clone();
                if rh.is_empty() || num_yp_tries >= 3 {
                    break;
                }
                let ctime = sys::get_time();
                let mut s = pc.chanmgr.settings();
                if ctime.wrapping_sub(s.last_yp_connect) > super::servmgr::MIN_YP_RETRY {
                    drop(s);
                    let h = Host::from_str_name(&rh, super::servmgr::DEFAULT_PORT);
                    {
                        let mut st = ch.st();
                        st.source_host.host = h;
                        st.source_host.yp = true;
                    }
                    s = pc.chanmgr.settings();
                    s.last_yp_connect = ctime;
                }
            }
            sys::sleep_idle();
            if ch.st().source_host.host.ip.is_set() || !ch.thread.active() {
                break;
            }
        }

        let sh = ch.st().source_host.clone();
        if !sh.host.ip.is_set() {
            crate::log_error!("Channel giving up");
            break;
        }
        if sh.yp {
            num_yp_tries += 1;
            crate::log_info!("Channel contacting YP, try {}", num_yp_tries);
        } else {
            crate::log_info!("Channel found hit");
            num_yp_tries = 0;
        }

        let ipstr = sh.host.str();
        let ty = if sh.tracker {
            "(tracker)"
        } else if sh.yp {
            "(YP)"
        } else {
            ""
        };
        let mut error = -1;
        let mut source: Option<SourceStream> = None;
        let started = std::time::Instant::now();
        let r: Result<()> = (|| {
            ch.set_status(pc, chn::S_CONNECTING);
            if sock.is_none() {
                crate::log_info!("Channel connecting to {} {}", ipstr, ty);
                sock = Some(connect_fetch(ch)?);
            }
            let s = sock.as_mut().ok_or_else(|| Error::stream("no socket"))?;
            set_src_stat(ch, Some(s.shared_stat()));
            ch.st().src_sock = Some(s.host);
            error = handshake_fetch(pc, ch, s)?;
            if error != 0 {
                return Err(Error::stream("Handshake error"));
            }
            let mut src = SourceStream::create(ch);
            error = run_read_stream(pc, ch, s, &mut src);
            source = Some(src);
            if error != 0 {
                return Err(Error::stream("Stream error"));
            }
            error = 0;
            ch.set_status(pc, chn::S_CLOSING);
            crate::log_info!("Channel closed normally");
            Ok(())
        })();
        // 同じ相手にまたつなぐか (dead_hit にしなかった)。そうでなければ次の候補を待たずに探す
        let mut retry_same = true;
        if let Err(e) = &r {
            ch.set_status(pc, chn::S_ERROR);
            crate::log_error!("Channel to {} {} : {}", ipstr, ty, e);
            if !sh.tracker || (error != 503 && sh.tracker) {
                pc.chanmgr.dead_hit(&sh);
                retry_same = false;
            }
        }

        // 下流につながっているものに終わりを知らせる
        {
            let mut out = AtomBuf::default();
            out.int(PCP_QUIT, PCP_ERROR_QUIT + PCP_ERROR_OFFAIR);
            if let Ok(mut pack) = ChanPacket::new(pb::T_PCP, &out.0, 0) {
                let (id, rid) = {
                    let st = ch.st();
                    (st.info.id, st.remote_id)
                };
                pc.servmgr.broadcast_packet(&mut pack, &id, &rid, &[0; 16], super::servent::T_RELAY);
            }
        }

        if let Some(mut src) = source.take() {
            if error == 0 {
                src.update_status(pc, ch);
                if let Some(s) = sock.as_mut() {
                    let _ = src.flush(s);
                }
            }
            *ch.source_stream.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        if let Some(mut s) = sock.take() {
            s.close();
        }
        set_src_stat(ch, None);
        ch.st().src_sock = None;

        if error == 404 {
            crate::log_error!("Channel not found");
            return;
        }

        ch.st().last_idle_time = sys::get_time();
        ch.set_status(pc, chn::S_IDLE);
        while ch.check_idle(pc) && ch.thread.active() {
            sys::sleep(200);
        }
        sys::sleep_idle();
        if retry_same {
            // 再接続 (bump) やプッシュの接続が来たら、すぐにつなぐ
            retry.wait(started, || {
                !ch.thread.active()
                    || ch.push_sock.lock().unwrap_or_else(|e| e.into_inner()).is_some()
                    || ch.st().designated_host.host.ip.is_set()
            });
        }
    }
}

/// `URLSource::getSourceProtocol`: 先頭の scheme を見て、残りを返す
fn source_protocol(url: &[u8]) -> (i32, &[u8]) {
    let (p, n) = crate::url::source_protocol(url);
    let proto = match p {
        crate::url::SourceProtocol::Http => ci::SP_HTTP,
        crate::url::SourceProtocol::Pcp => ci::SP_PCP,
        crate::url::SourceProtocol::File => ci::SP_FILE,
        crate::url::SourceProtocol::Rtmp => ci::SP_RTMP,
        crate::url::SourceProtocol::Pipe => ci::SP_PIPE,
    };
    (proto, &url[n..])
}

/// `URLSource::stream`
fn url_stream(pc: &Arc<Peercast>, ch: &Arc<Channel>, base: &[u8]) {
    let mut url: Vec<u8> = Vec::new();
    let mut retry = RetryDelay::default();
    while ch.thread.active() && !pc.is_quitting() {
        // 管理者が入力した URL だけを信じる。返ってきたのはリダイレクト先 (中継元が書いたもの)
        let trusted = url.is_empty();
        if trusted {
            url = base.to_vec();
        }
        let started = std::time::Instant::now();
        url = stream_url(pc, ch, &url, 0, trusted);
        retry.wait(started, || !ch.thread.active() || pc.is_quitting());
    }
}

/// 入力 (ソケット、ファイル、外部のプログラムの出力)
#[allow(dead_code)] // Pipe の Subprogram は、読み終わるまで子プロセスを持っておくため
enum Input {
    Sock(ClientSocket),
    File(super::stream::FileStream),
    Pipe(super::subprog::Subprogram, super::subprog::PipeReader),
    #[cfg(feature = "rtmp")]
    Rtmp(super::rtmp::RtmpStream),
}

impl Input {
    fn stream(&mut self) -> &mut dyn Stream {
        match self {
            Input::Sock(s) => s,
            Input::File(f) => f,
            Input::Pipe(_, r) => r,
            #[cfg(feature = "rtmp")]
            Input::Rtmp(r) => r,
        }
    }
}

/// プレイリストの入れ子の上限 (C++ 版には上限がなく、プレイリストを指すプレイリストで再帰が深くなった)
const MAX_PLAYLIST_DEPTH: i32 = 8;

/// `URLSource::streamURL`: 次に読む URL (リダイレクト先) を返す。
///
/// `trusted` は、URL が管理者の入力したものか (ローカルのプレイリストの中身を含む)。そうでない
/// (リダイレクト先や HTTP で取ったプレイリストの中身) なら、`pipe:` やファイルを指す URL で
/// 外部のプログラムを起こしたりローカルのファイルを読んだりしないよう、ネットワークの URL だけを
/// 受け付ける (C++ 版は区別しなかった)。
fn stream_url(pc: &Arc<Peercast>, ch: &Arc<Channel>, url: &[u8], depth: i32, trusted: bool) -> Vec<u8> {
    let mut next_url = Vec::new();
    if pc.is_quitting() || !ch.thread.active() {
        return next_url;
    }
    let url = &url[..url.iter().position(|&c| c == 0).unwrap_or(url.len())];
    let url = &url[..url.len().min(255)];
    crate::log_info!("Fetch URL={}", String::from_utf8_lossy(url));

    let r: Result<()> = (|| {
        if !trusted && !crate::url::is_remote_safe_source(url) {
            return Err(Error::stream("Refusing a non-network URL given by the source"));
        }
        let (proto, file_name) = source_protocol(url);
        {
            let mut st = ch.st();
            st.info.src_protocol = proto;
            if st.info.content_type.data == ci::T_PLS {
                st.info.content_type.assign(ci::T_MP3);
            }
        }
        ch.set_status(pc, chn::S_CONNECTING);
        let mut pls: Option<super::playlist::PlayList> = None;
        let mut chunked = false;
        let mut input: Input;
        if proto == ci::SP_HTTP || proto == ci::SP_PCP {
            crate::log_info!("Channel source is HTTP");
            let (host_part, dir) = match file_name.iter().position(|&c| c == b'/') {
                Some(i) => (&file_name[..i], Some(&file_name[i + 1..])),
                None => (file_name, None),
            };
            crate::log_info!("Fetch Host={}", String::from_utf8_lossy(host_part));
            if let Some(d) = dir {
                crate::log_info!("Fetch Dir={}", String::from_utf8_lossy(d));
            }
            let host = Host::from_str_name(host_part, 80);
            let mut sock = ClientSocket::new();
            sock.connect(host)?;
            set_src_stat(ch, Some(sock.shared_stat()));
            let res;
            {
                let mut req = format!("GET /{} HTTP/1.0", String::from_utf8_lossy(dir.unwrap_or(b""))).into_bytes();
                // バイト列のまま書く
                if let Some(d) = dir {
                    req = [&b"GET /"[..], d, b" HTTP/1.0"].concat();
                }
                sock.write_line(&req)?;
                sock.write_line([&b"Host: "[..], host_part].concat())?;
                sock.write_line("Connection: close")?;
                sock.write_line("Accept: */*")?;
                sock.write_line(format!("User-Agent: {}", PCX_AGENT))?;
                sock.write_line(format!("{} {}", PCX_HS_PCP, 1))?;
                sock.write_line("Icy-MetaData:1")?;
                sock.write_line("")?;
                let mut http = Http::new(&mut sock);
                res = http.read_response()?;
                while http.next_header()? {
                    crate::log_info!("Fetch HTTP: {}", String::from_utf8_lossy(&http.cmd_line));
                    {
                        let mut st = ch.st();
                        let tmp = st.info.clone();
                        super::servent::read_icy_header(&http, &mut st.info, None);
                        if !tmp.name.is_empty() {
                            st.info.name = tmp.name;
                        }
                        if !tmp.genre.is_empty() {
                            st.info.genre = tmp.genre;
                        }
                        if !tmp.url.is_empty() {
                            st.info.url = tmp.url;
                        }
                    }
                    if http.is_header("icy-metaint") {
                        ch.st().icy_meta_interval = http.arg_int();
                    } else if http.is_header("Location:") {
                        next_url = http.arg_str().map(|a| a[..a.len().min(255)].to_vec()).unwrap_or_default();
                    } else if http.is_header("Transfer-Encoding:") && http.arg_str() == Some(b"chunked") {
                        chunked = true;
                    }
                    if let Some(arg) = http.arg_str() {
                        if http.is_header("content-type") {
                            let st = crate::http::stristr;
                            if st(arg, b"audio/x-scpls").is_some() {
                                pls = Some(super::playlist::PlayList::new(super::playlist::T_SCPLS, 1000));
                            } else if st(arg, b"audio/mpegurl").is_some()
                                || st(arg, b"audio/x-mpegurl").is_some()
                                || st(arg, b"audio/m3u").is_some()
                                || st(arg, b"text/plain").is_some()
                            {
                                pls = Some(super::playlist::PlayList::new(super::playlist::T_PLS, 1000));
                            }
                        }
                    }
                }
            }
            if !next_url.is_empty() && res == 302 {
                crate::log_info!("Channel redirect: {}", String::from_utf8_lossy(&next_url));
                sock.close();
                return Ok(());
            }
            if res != 200 {
                crate::log_error!("HTTP response: {}", res);
                return Err(Error::stream("Bad HTTP connect"));
            }
            input = Input::Sock(sock);
        } else if proto == ci::SP_RTMP {
            #[cfg(feature = "rtmp")]
            {
                crate::log_info!("Channel source is RTMP");
                let rs = super::rtmp::RtmpStream::open(url)?;
                input = Input::Rtmp(rs);
                ch.st().info.set_content_type(ci::T_FLV);
            }
            #[cfg(not(feature = "rtmp"))]
            {
                crate::log_error!("Not compiled with RTMP support");
                return Err(Error::stream("Unsupported URL"));
            }
        } else if proto == ci::SP_FILE {
            crate::log_info!("Channel source is FILE");
            let fs = super::stream::FileStream::open_read(file_name)?;
            set_src_stat(ch, Some(fs.shared_stat()));
            let ext = crate::strutil::extension_without_dot(file_name);
            let file_type = ci::type_from_str(&ext);
            ch.st().read_delay = true;
            if file_type == ci::T_PLS {
                pls = Some(super::playlist::PlayList::new(super::playlist::T_PLS, 1000));
            } else {
                ch.st().info.set_content_type(file_type);
            }
            input = Input::File(fs);
        } else if proto == ci::SP_PIPE {
            crate::log_info!("Channel source is PIPE");
            let argv = crate::strutil::shellwords(file_name).map_err(Error::format)?;
            if argv.is_empty() {
                return Err(Error::format("Empty command line"));
            }
            let mut sp = super::subprog::Subprogram::new(&argv[0], true, false);
            let env = super::subprog::Environment::from_current_process();
            sp.start(&argv[1..], &env);
            let r = sp.input_stream()?;
            input = Input::Pipe(sp, r);
        } else {
            return Err(Error::stream("Unsupported URL"));
        }

        if let Some(mut pl) = pls.take() {
            crate::log_info!("Channel is Playlist");
            pl.read(input.stream());
            input.stream().close();
            drop(input);
            set_src_stat(ch, None);
            let mut url_num = 0usize;
            let mut u: Vec<u8> = Vec::new();
            // 中身を信じてよいのは、管理者が入力した URL のローカルのプレイリストだけ。
            // HTTP で取ったものは中継元が書いたもの
            let entries_trusted = trusted && proto == ci::SP_FILE;
            let mut u_trusted = false;
            crate::log_info!("Playlist: {} URLs", pl.urls.len());
            if depth >= MAX_PLAYLIST_DEPTH {
                return Err(Error::stream("Playlist nesting too deep"));
            }
            let mut retry = RetryDelay::default();
            while ch.thread.active() && !pl.urls.is_empty() && !pc.is_quitting() {
                if u.is_empty() {
                    u = pl.urls[url_num % pl.urls.len()].clone();
                    url_num += 1;
                    u_trusted = entries_trusted;
                }
                let started = std::time::Instant::now();
                u = stream_url(pc, ch, &u, depth + 1, u_trusted);
                // 返ってきたのはリダイレクト先 (中継元が書いたもの)
                u_trusted = false;
                retry.wait(started, || !ch.thread.active() || pc.is_quitting());
            }
        } else {
            // 配信元が ID を送ってこなければ、自分で作る (最初の配信)
            {
                let bid = pc.chanmgr.broadcast_id();
                let mut st = ch.st();
                if !ci::is_set(&st.info.id) {
                    let mut id = bid;
                    let name = st.info.name.data.clone();
                    let genre = st.info.genre.data.clone();
                    crate::gnuid::encode(&mut id, None, &name, &genre, st.info.bitrate as u8);
                    st.info.id = id;
                }
            }
            ch.set_status(pc, chn::S_BROADCASTING);
            input.stream().set_read_timeout(60000);
            let mut source = SourceStream::create(ch);
            if chunked {
                crate::log_debug!("Dechunker enabled");
                let mut d = Dechunker::new(input.stream());
                run_read_stream(pc, ch, &mut d, &mut source);
            } else {
                run_read_stream(pc, ch, input.stream(), &mut source);
            }
            input.stream().close();
        }
        Ok(())
    })();
    if let Err(e) = r {
        ch.set_status(pc, chn::S_ERROR);
        crate::log_error!("Channel error: {}", e);
        sys::sleep(1000);
    }
    ch.set_status(pc, chn::S_CLOSING);
    set_src_stat(ch, None);
    *ch.source_stream.lock().unwrap_or_else(|e| e.into_inner()) = None;
    next_url
}

#[allow(dead_code)]
fn _unused(_: ChanInfo) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay() {
        let mut r = RetryDelay::default();
        // すぐに終わるのが続くと、2 回目から 1、2、4、8、16 秒、そのあとは 30 秒
        let waits: Vec<u32> = (0..9).map(|_| r.next(50)).collect();
        assert_eq!(waits, [0, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
        // 長く続いたあとに終わったら待たず、次にすぐに終わっても 1 回目の扱い
        assert_eq!(r.next(RetryDelay::QUICK_MS), 0);
        assert_eq!(r.next(9_999), 0);
        assert_eq!(r.next(0), 1000);
        // 何度続いても桁あふれしない
        r.quick_ends = u32::MAX - 1;
        assert_eq!((r.next(0), r.next(0)), (30000, 30000));
    }

    #[test]
    fn retry_wait_stops() {
        let mut r = RetryDelay::default();
        r.quick_ends = 5;
        let t = std::time::Instant::now();
        r.wait(t, || true);
        assert!(t.elapsed().as_millis() < 1000);
    }
}
