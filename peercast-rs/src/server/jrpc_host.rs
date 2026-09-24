//! JSON-RPC の API (`crate::jrpc`) から見たサーバーの状態 (C++ 版の core/common/rustjrpc.h の
//! `JrpcHost`)。

use std::sync::Arc;

use super::chaninfo::ChanInfo;
use super::channel::{self, Channel};
use super::host::Host;
use super::pcstr::{self, PcString, StrType};
use super::peercast::Peercast;
use super::stream::{FileStream, StreamExt};
use super::sys;
use crate::jrpc::{
    self, ChannelData, FetchRequest, FoundData, FoundHit, HostError, HostResult, InfoData, Level, RelayTreeData, ServentData, SettingKey,
    Settings, Status, YpEntry,
};

pub struct JrpcHost<'a> {
    pub pc: &'a Arc<Peercast>,
}

fn info_data(i: &ChanInfo) -> InfoData {
    InfoData {
        id: i.id,
        name: i.name.data.clone(),
        content_type: i.content_type.data.clone(),
        mime: i.mime_type.data.clone(),
        desc: i.desc.data.clone(),
        genre: i.genre.data.clone(),
        url: i.url.data.clone(),
        comment: i.comment.data.clone(),
        bitrate: i.bitrate,
        track_contact: i.track.contact.data.clone(),
        track_title: i.track.title.data.clone(),
        track_artist: i.track.artist.data.clone(),
        track_album: i.track.album.data.clone(),
        track_genre: i.track.genre.data.clone(),
    }
}

fn host_str(h: &Host) -> Vec<u8> {
    h.str().into_bytes()
}

impl JrpcHost<'_> {
    fn channel_data(&self, c: &Arc<Channel>) -> ChannelData {
        let pc = self.pc;
        let (info, status, source_url, source_host, ip_version, stream_pos) = {
            let st = c.st();
            (st.info.clone(), st.status, st.source_url.data.clone(), st.source_host.host, st.ip_version, st.stream_pos)
        };
        let sock_host = c.sock.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| host_str(&s.host));
        ChannelData {
            uptime: info.uptime(pc.chanmgr.max_uptime()),
            src_protocol: info.src_protocol,
            info: info_data(&info),
            status,
            source_url,
            source_host: host_str(&source_host),
            local_relays: c.local_relays(pc, true),
            local_directs: c.local_listeners(pc, true),
            total_relays: c.total_relays(pc),
            total_directs: c.total_listeners(pc),
            is_broadcasting: c.is_broadcasting(),
            is_full: c.is_full(pc),
            is_receiving: c.is_receiving(),
            ip_version,
            sock_host,
            source_rate: c.source_rate(false) as i32,
            stream_pos,
        }
    }
}

/// `String` に代入したもの
fn pcs(s: &[u8]) -> PcString {
    PcString::with_type(&pcstr::cut(s), StrType::Ascii)
}

impl jrpc::Host for JrpcHost<'_> {
    fn log(&mut self, level: Level, msg: &[u8]) {
        let m = String::from_utf8_lossy(msg);
        match level {
            Level::Debug => crate::log_debug!("{}", m),
            Level::Info => crate::log_info!("{}", m),
            Level::Warn => crate::log_warn!("{}", m),
            Level::Error => crate::log_error!("{}", m),
        }
    }

    fn agent(&mut self) -> Vec<u8> {
        super::http::PCX_AGENT.as_bytes().to_vec()
    }

    fn log_lines(&mut self) -> HostResult<Vec<Vec<u8>>> {
        let mut lines = Vec::new();
        super::log::with_buffer(|b| {
            b.each_line(|time, ty, line| {
                let mut buf = Vec::new();
                if ty != super::log::Level::None {
                    let t = sys::time_string(time);
                    let end = t.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(0, |i| i + 1);
                    buf.extend_from_slice(&t[..end]);
                    buf.extend_from_slice(b" [");
                    buf.extend_from_slice(ty.type_str().as_bytes());
                    buf.extend_from_slice(b"] ");
                }
                buf.extend_from_slice(line);
                lines.push(buf);
            })
        });
        Ok(lines)
    }

    fn clear_log(&mut self) -> HostResult<()> {
        super::log::with_buffer(|b| b.clear());
        Ok(())
    }

    fn log_level(&mut self) -> HostResult<i32> {
        Ok(super::log::level())
    }

    fn set_log_level(&mut self, level: i32) -> HostResult<()> {
        super::log::set_level(level);
        Ok(())
    }

    fn fetch(&mut self, req: &FetchRequest) -> HostResult<Option<[u8; 16]>> {
        let pc = self.pc;
        let mut info = ChanInfo::new();
        info.name = pcs(&req.name);
        info.desc = pcs(&req.desc);
        info.genre = pcs(&req.genre);
        info.url = pcs(&req.url);
        info.bitrate = req.bitrate;
        info.set_content_type(&pcstr::cut(&req.type_str));
        super::servent_http::set_broadcast_id_channel_id(pc, &mut info, &pc.chanmgr.broadcast_id());
        let c = pc.chanmgr.create_channel(pc, &info, None);
        if req.ipv6 {
            c.st().ip_version = channel::IP_V6;
            pc.servmgr.check_firewall_ipv6();
        }
        c.start_url(pc, &req.url);
        Ok(Some(c.id()))
    }

    fn channels(&mut self) -> HostResult<Vec<ChannelData>> {
        Ok(self.pc.chanmgr.channels().iter().map(|c| self.channel_data(c)).collect())
    }

    fn find_channel(&mut self, id: &[u8; 16]) -> HostResult<Option<ChannelData>> {
        Ok(self.pc.chanmgr.find_channel_by_id(id).map(|c| self.channel_data(&c)))
    }

    fn servents(&mut self, id: &[u8; 16]) -> HostResult<Vec<ServentData>> {
        let mut v = Vec::new();
        for sv in self.pc.servmgr.servents() {
            let st = sv.st().clone();
            if st.chan_id != *id {
                continue;
            }
            let stat = sv.sock_stat();
            v.push(ServentData {
                index: sv.index,
                type_str: sv.type_str().as_bytes().to_vec(),
                status_str: sv.status_str().as_bytes().to_vec(),
                send_rate: stat.as_ref().map_or(0, |s| s.bytes_out_per_sec()),
                recv_rate: stat.as_ref().map_or(0, |s| s.bytes_in_per_sec()),
                protocol: st.output_protocol,
                agent: st.agent.data.clone(),
                sock_host: if sv.has_sock() { Some(host_str(&st.sock_host)) } else { None },
            });
        }
        Ok(v)
    }

    fn stop_connection(&mut self, id: &[u8; 16], connection_id: i32) -> HostResult<bool> {
        for sv in self.pc.servmgr.servents() {
            let (chan, ty) = {
                let st = sv.st();
                (st.chan_id, st.ty)
            };
            if sv.index == connection_id && chan == *id && ty == super::servent::T_RELAY {
                sv.abort();
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn relay_tree(&mut self, id: &[u8; 16]) -> HostResult<RelayTreeData> {
        let pc = self.pc;
        let ch = match pc.chanmgr.find_channel_by_id(id) {
            Some(c) => c,
            None => return Ok(RelayTreeData::NoChannel),
        };
        let list = match pc.chanmgr.with_hitlist_by_id(id, |l| l.hits.clone()) {
            Some(l) => l,
            None => return Ok(RelayTreeData::NoHitList),
        };
        let is_tracker = ch.is_broadcasting();
        let (skips, uptime, uphost, ipv6) = {
            let st = ch.st();
            (
                st.info.num_skips,
                st.info.uptime(pc.chanmgr.max_uptime()),
                if is_tracker { Host::none() } else { st.source_host.host },
                st.ip_version == 6,
            )
        };
        let mut me = pc.servmgr.init_local_hit(
            pc,
            ch.local_listeners(pc, true),
            ch.local_relays(pc, true),
            skips as i32,
            uptime,
            ch.is_playing(),
            ch.raw_data.oldest_pos(),
            ch.raw_data.latest_pos(),
            ch.can_add_relay(pc),
            uphost,
            ipv6,
        );
        me.tracker = is_tracker;
        let mut hits = vec![me];
        for h in list {
            crate::log_debug!("HostGraph: {}", h.rhost[0].str());
            hits.push(h);
        }
        Ok(RelayTreeData::Hits(hits.iter().map(|h| (h.view(), h.rhost[0].ip.str().into_bytes())).collect()))
    }

    fn bump(&mut self, id: &[u8; 16]) -> HostResult<bool> {
        match self.pc.chanmgr.find_channel_by_id(id) {
            Some(c) => {
                c.st().bump = true;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn play(&mut self, id: &[u8; 16]) -> HostResult<()> {
        let mut info = ChanInfo::new();
        info.id = *id;
        self.pc.chanmgr.find_and_play_channel(self.pc, &info, false);
        Ok(())
    }

    fn stop_channel(&mut self, id: &[u8; 16]) -> HostResult<()> {
        if let Some(c) = self.pc.chanmgr.find_channel_by_id(id) {
            c.thread.shutdown();
        }
        Ok(())
    }

    fn root_host(&mut self) -> HostResult<Vec<u8>> {
        Ok(self.pc.servmgr.settings().root_host.data.clone())
    }

    fn clear_root_host(&mut self) -> HostResult<()> {
        self.pc.servmgr.settings().root_host.clear();
        Ok(())
    }

    fn settings(&mut self) -> HostResult<Settings> {
        let s = self.pc.servmgr.settings().clone();
        Ok(Settings {
            max_relays: s.max_relays,
            max_relays_per_channel: self.pc.chanmgr.settings().max_relays_per_channel,
            max_direct: s.max_direct,
            max_bitrate_out: s.max_bitrate_out,
        })
    }

    fn set_setting(&mut self, key: SettingKey, value: i32) -> HostResult<()> {
        let sm = &self.pc.servmgr;
        match key {
            SettingKey::MaxRelays => sm.set_max_relays(value),
            SettingKey::MaxRelaysPerChannel => self.pc.chanmgr.settings().max_relays_per_channel = value,
            SettingKey::MaxDirect => sm.settings().max_direct = value as u32,
            SettingKey::MaxBitrateOut => sm.settings().max_bitrate_out = value as u32,
        }
        Ok(())
    }

    fn status(&mut self) -> HostResult<Status> {
        let sm = &self.pc.servmgr;
        let (h, lip) = {
            let s = sm.settings();
            (s.server_host, s.server_local_ip)
        };
        Ok(Status {
            uptime: sm.uptime(),
            firewall: sm.get_firewall(4),
            global_ip: h.ip_str().into_bytes(),
            port: h.port,
            local_ip: lip.str().into_bytes(),
        })
    }

    fn state(&mut self, which: usize) -> HostResult<Vec<u8>> {
        let pc = self.pc;
        let v = match which {
            0 => pc.servmgr.state(pc),
            1 => pc.chanmgr.state(pc),
            2 => super::state::obj(super::stats::state().into_iter().map(|(k, v)| (k, super::state::s(v))).collect()),
            3 => pc.notifications().state(),
            4 => match super::html::root_scope(pc) {
                super::html::Scope::Root(m) => m.get(&b"sys"[..]).cloned().unwrap_or(super::state::Value::Null),
                _ => super::state::Value::Null,
            },
            _ => pc.yplist.state(),
        };
        v.inspect().map_err(|e| HostError::Exception(format!("{:?}", e).into_bytes()))
    }

    fn update_info(&mut self, id: &[u8; 16], f: &[Vec<u8>; 10]) -> HostResult<()> {
        let ch = match self.pc.chanmgr.find_channel_by_id(id) {
            Some(c) => c,
            None => return Ok(()),
        };
        let mut i = ch.info();
        i.name = pcs(&f[0]);
        i.desc = pcs(&f[1]);
        i.genre = pcs(&f[2]);
        i.url = pcs(&f[3]);
        i.comment = pcs(&f[4]);
        i.track.contact = pcs(&f[5]);
        i.track.title = pcs(&f[6]);
        i.track.artist = pcs(&f[7]);
        i.track.album = pcs(&f[8]);
        i.track.genre = pcs(&f[9]);
        ch.update_info(self.pc, &i);
        Ok(())
    }

    fn yp_channels(&mut self) -> HostResult<Vec<YpEntry>> {
        Ok(self
            .pc
            .servmgr
            .channel_directory
            .channels()
            .iter()
            .map(|c| YpEntry {
                feed_url: c.feed_url.clone(),
                name: c.name.clone(),
                id: c.id,
                tip: c.tip.clone(),
                url: c.url.clone(),
                genre: c.genre.clone(),
                desc: c.desc.clone(),
                comment: c.comment.clone(),
                bitrate: c.bitrate,
                content_type: c.content_type.clone(),
                track_name: c.track_name.clone(),
                track_album: c.track_album.clone(),
                track_artist: c.track_artist.clone(),
                track_contact: c.track_contact.clone(),
                num_directs: c.num_directs,
                num_relays: c.num_relays,
            })
            .collect())
    }

    fn read_storage(&mut self, key: &[u8]) -> HostResult<Option<Vec<u8>>> {
        let path = [&self.pc.app.state_dir[..], b"/", key, b".json"].concat();
        let mut f = match FileStream::open_read(&path) {
            Ok(f) => f,
            Err(e) if e.is_stream() => return Ok(None),
            Err(e) => return Err(HostError::Exception(e.msg.into_bytes())),
        };
        let len = f.length().max(0) as usize;
        let data = f.read_n(len).map_err(|e| HostError::Exception(e.msg.into_bytes()))?;
        Ok(Some(data))
    }

    fn write_storage(&mut self, key: &[u8], value: &[u8]) -> HostResult<()> {
        let path = [&self.pc.app.state_dir[..], b"/", key, b".json"].concat();
        let mut f = FileStream::open_write(&path).map_err(|e| HostError::Exception(e.msg.into_bytes()))?;
        f.write_string(value).map_err(|e| HostError::Exception(e.msg.into_bytes()))?;
        f.flush();
        Ok(())
    }

    fn channels_found(&mut self) -> HostResult<Vec<FoundData>> {
        let now = sys::get_time();
        let max = self.pc.chanmgr.max_uptime();
        Ok(self
            .pc
            .chanmgr
            .hitlists()
            .iter()
            .map(|l| FoundData {
                info: info_data(&l.info),
                uptime: l.info.uptime(max),
                skips: l.info.num_skips,
                age: l.info.age(),
                bcflags: l.info.bc_id[0],
                hosts: l.num_hits(),
                listeners: l.num_listeners(),
                relays: l.num_relays(),
                firewalled: l.num_firewalled(),
                closest: l.closest_hit(),
                furthest: l.furthest_hit(),
                newest: now.wrapping_sub(l.newest_hit()),
                hits: l
                    .hits
                    .iter()
                    .filter(|h| h.host.ip.is_set())
                    .map(|h| FoundHit {
                        ip: host_str(&h.host),
                        hops: h.num_hops,
                        listeners: h.num_listeners,
                        relays: h.num_relays,
                        uptime: h.up_time,
                        push: h.firewalled,
                        relay: h.relay,
                        direct: h.direct,
                        cin: h.cin,
                        stable: h.stable,
                        version: h.version,
                        update: now.wrapping_sub(h.time),
                        tracker: h.tracker,
                    })
                    .collect(),
            })
            .collect())
    }
}

/// `JrpcApi::call`
pub fn call(pc: &Arc<Peercast>, request: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
    jrpc::call(request, &mut JrpcHost { pc })
}

/// `JrpcApi` のメソッドを直接呼ぶ (`getVersionInfo`、`getChannels` など)
pub fn invoke(pc: &Arc<Peercast>, method: &str) -> Result<crate::json::Value, Vec<u8>> {
    jrpc::invoke(method.as_bytes(), Vec::new(), &mut JrpcHost { pc }).map_err(jrpc::call_what)
}
