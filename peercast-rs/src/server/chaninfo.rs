//! チャンネルの情報 (core/common/chaninfo.cpp の `ChanInfo` と `TrackInfo`)。判断と atom の組み立ては
//! 段階 7b の `crate::chaninfo`。

use super::pcstr::{PcString, StrType};
use super::state::{obj, s, Value};
use super::sys;
use super::xmlnode::XmlNode;
use crate::chaninfo as ci;
use crate::pcp::write::AtomBuf;

pub use crate::chaninfo::{
    protocol_from_str, protocol_str, type_from_str, SP_FILE, SP_HTTP, SP_PCP, SP_PIPE, SP_RTMP, SP_UNKNOWN, T_FLV, T_MKV, T_MOV, T_MP3, T_MP4, T_MPG, T_OGG, T_OGM,
    T_PLS, T_RAW, T_UNKNOWN, T_WEBM,
};

/// `ChanInfo::STATUS`
pub const S_UNKNOWN: i32 = 0;
pub const S_PLAY: i32 = 1;

/// `TrackInfo`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub contact: PcString,
    pub title: PcString,
    pub artist: PcString,
    pub album: PcString,
    pub genre: PcString,
}

impl TrackInfo {
    pub fn clear(&mut self) {
        *self = TrackInfo::default();
    }

    pub fn view(&self) -> ci::Track<'_> {
        ci::Track {
            contact: &self.contact.data,
            title: &self.title.data,
            artist: &self.artist.data,
            album: &self.album.data,
            genre: &self.genre.data,
        }
    }

    /// `update`
    pub fn update(&mut self, t: &TrackInfo) -> bool {
        let c = ci::track_update(&self.view(), &t.view());
        self.apply(t, c);
        c != 0
    }

    fn apply(&mut self, t: &TrackInfo, c: u32) {
        if c & ci::U_TRACK_CONTACT != 0 {
            self.contact = t.contact.clone();
        }
        if c & ci::U_TRACK_TITLE != 0 {
            self.title = t.title.clone();
        }
        if c & ci::U_TRACK_ARTIST != 0 {
            self.artist = t.artist.clone();
        }
        if c & ci::U_TRACK_ALBUM != 0 {
            self.album = t.album.clone();
        }
        if c & ci::U_TRACK_GENRE != 0 {
            self.genre = t.genre.clone();
        }
    }
}

/// `ChanInfo`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChanInfo {
    pub name: PcString,
    pub id: [u8; 16],
    pub bc_id: [u8; 16],
    pub bitrate: i32,
    pub content_type: PcString,
    pub mime_type: PcString,
    pub stream_ext: PcString,
    pub src_protocol: i32,
    pub last_play_start: u32,
    pub last_play_end: u32,
    pub num_skips: u32,
    pub created_time: u32,
    pub status: i32,
    pub track: TrackInfo,
    pub desc: PcString,
    pub genre: PcString,
    pub url: PcString,
    pub comment: PcString,
}

impl Default for ChanInfo {
    /// `init`
    fn default() -> Self {
        ChanInfo {
            name: PcString::default(),
            id: [0; 16],
            bc_id: [0; 16],
            bitrate: 0,
            content_type: PcString::new(ci::T_UNKNOWN),
            mime_type: PcString::default(),
            stream_ext: PcString::default(),
            src_protocol: SP_UNKNOWN,
            last_play_start: 0,
            last_play_end: 0,
            num_skips: 0,
            created_time: 0,
            status: S_UNKNOWN,
            track: TrackInfo::default(),
            desc: PcString::default(),
            genre: PcString::default(),
            url: PcString::default(),
            comment: PcString::default(),
        }
    }
}

pub fn is_set(id: &[u8; 16]) -> bool {
    id.iter().any(|&b| b != 0)
}

pub fn id_str(id: &[u8; 16]) -> String {
    String::from_utf8_lossy(&crate::gnuid::to_str(id)).into_owned()
}

impl ChanInfo {
    pub fn new() -> ChanInfo {
        ChanInfo::default()
    }

    /// `init()`
    pub fn init(&mut self) {
        *self = ChanInfo::default();
    }

    /// `initNameID`: ID として読めればその ID、でなければ名前
    pub fn init_name_id(s: &[u8]) -> ChanInfo {
        let mut i = ChanInfo::default();
        i.id = crate::gnuid::from_str(s);
        if !is_set(&i.id) {
            i.name.assign(s);
        }
        i
    }

    pub fn view(&self) -> ci::Info<'_> {
        ci::Info {
            name: &self.name.data,
            id: self.id,
            bcid: self.bc_id,
            bitrate: self.bitrate,
            content_type: &self.content_type.data,
            mime: &self.mime_type.data,
            ext: &self.stream_ext.data,
            status: self.status,
            desc: &self.desc.data,
            genre: &self.genre.data,
            url: &self.url.data,
            comment: &self.comment.data,
            track: self.track.view(),
        }
    }

    /// `update`: 変わった欄を写す。写したものがあれば true
    pub fn update(&mut self, info: &ChanInfo) -> bool {
        match ci::update(&self.view(), &info.view()) {
            ci::Update::Ignore => false,
            ci::Update::BadKey => {
                crate::log_error!("ChanInfo BC key not valid");
                false
            }
            ci::Update::Apply { set_bcid, copy } => {
                if set_bcid {
                    self.bc_id = info.bc_id;
                }
                if copy & ci::U_BITRATE != 0 {
                    self.bitrate = info.bitrate;
                }
                if copy & ci::U_CONTENT_TYPE != 0 {
                    self.content_type = info.content_type.clone();
                }
                if copy & ci::U_MIME != 0 {
                    self.mime_type = info.mime_type.clone();
                }
                if copy & ci::U_EXT != 0 {
                    self.stream_ext = info.stream_ext.clone();
                }
                if copy & ci::U_DESC != 0 {
                    self.desc = info.desc.clone();
                }
                if copy & ci::U_NAME != 0 {
                    self.name = info.name.clone();
                }
                if copy & ci::U_COMMENT != 0 {
                    self.comment = info.comment.clone();
                }
                if copy & ci::U_GENRE != 0 {
                    self.genre = info.genre.clone();
                }
                if copy & ci::U_URL != 0 {
                    self.url = info.url.clone();
                }
                self.track.apply(&info.track, copy);
                copy != 0
            }
        }
    }

    /// `match(ChanInfo&)`
    pub fn matches(&self, q: &ChanInfo) -> bool {
        ci::match_info(&self.view(), &q.view())
    }

    /// `matchNameID`
    pub fn match_name_id(&self, q: &ChanInfo) -> bool {
        ci::match_name_id(&self.view(), &q.view())
    }

    /// `getUptime`: `max_uptime` (0 なら制限なし) で頭打ち
    pub fn uptime(&self, max_uptime: u32) -> u32 {
        let upt = if self.last_play_start != 0 { sys::get_time().wrapping_sub(self.last_play_start) } else { 0 };
        if max_uptime != 0 && upt > max_uptime {
            max_uptime
        } else {
            upt
        }
    }

    /// `getAge`
    pub fn age(&self) -> u32 {
        sys::get_time().wrapping_sub(self.created_time)
    }

    /// `isPrivate` (`bcID.getFlags() & 1`)
    pub fn is_private(&self) -> bool {
        self.bc_id[0] & 1 != 0
    }

    /// `getTypeStr`
    pub fn type_str(&self) -> &[u8] {
        &self.content_type.data
    }

    /// `getTypeStringLong`
    pub fn type_string_long(&self) -> Vec<u8> {
        ci::type_string_long(&self.content_type.data, &self.mime_type.data, &self.stream_ext.data)
    }

    /// `getTypeExt`
    pub fn type_ext(&self) -> Vec<u8> {
        ci::effective_ext(&self.content_type.data, &self.stream_ext.data).to_vec()
    }

    /// `getMIMEType`
    pub fn mime(&self) -> Vec<u8> {
        ci::effective_mime(&self.content_type.data, &self.mime_type.data).to_vec()
    }

    /// `getPlayListExt`
    pub fn playlist_ext(&self) -> &'static [u8] {
        ci::playlist_ext(&self.content_type.data)
    }

    /// `setContentType`
    pub fn set_content_type(&mut self, t: &[u8]) {
        self.content_type.assign(ci::type_from_str(t));
        self.mime_type.assign(ci::mime_type(t));
        self.stream_ext.assign(ci::type_ext(t));
    }

    /// `writeInfoAtoms`
    pub fn write_info_atoms(&self, out: &mut AtomBuf) {
        ci::write_info_atoms(out, &self.view());
    }

    /// `writeTrackAtoms`
    pub fn write_track_atoms(&self, out: &mut AtomBuf) {
        ci::write_track_atoms(out, &self.track.view());
    }

    fn uni(s: &PcString) -> String {
        String::from_utf8_lossy(&s.converted(StrType::UnicodeSafe)).into_owned()
    }

    /// `createChannelXML`
    pub fn channel_xml(&self, max_uptime: u32) -> XmlNode {
        let mut a = format!(
            "channel name=\"{}\" id=\"{}\" bitrate=\"{}\" type=\"{}\" genre=\"{}\" desc=\"{}\" url=\"{}\" uptime=\"{}\" comment=\"{}\" skips=\"{}\" age=\"{}\" bcflags=\"{}\"",
            Self::uni(&self.name),
            id_str(&self.id),
            self.bitrate,
            String::from_utf8_lossy(&self.content_type.data),
            Self::uni(&self.genre),
            Self::uni(&self.desc),
            Self::uni(&self.url),
            self.uptime(max_uptime) as i32,
            Self::uni(&self.comment),
            self.num_skips as i32,
            self.age() as i32,
            self.bc_id[0] as i32
        )
        .into_bytes();
        a.truncate(8191);
        XmlNode::new(a)
    }

    /// `createRelayChannelXML`
    pub fn relay_channel_xml(&self, max_uptime: u32) -> XmlNode {
        XmlNode::new(format!(
            "channel id=\"{}\" uptime=\"{}\" skips=\"{}\" age=\"{}\"",
            id_str(&self.id),
            self.uptime(max_uptime) as i32,
            self.num_skips as i32,
            self.age() as i32
        ))
    }

    /// `createTrackXML`
    pub fn track_xml(&self) -> XmlNode {
        XmlNode::new(format!(
            "track title=\"{}\" artist=\"{}\" album=\"{}\" genre=\"{}\" contact=\"{}\"",
            Self::uni(&self.track.title),
            Self::uni(&self.track.artist),
            Self::uni(&self.track.album),
            Self::uni(&self.track.genre),
            Self::uni(&self.track.contact)
        ))
    }

    /// `getState`
    pub fn state(&self) -> Value {
        obj(vec![
            ("name", s(&self.name.data)),
            ("desc", s(&self.desc.data)),
            ("genre", s(&self.genre.data)),
            ("url", s(&self.url.data)),
            ("comment", s(&self.comment.data)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_and_match() {
        let mut a = ChanInfo::new();
        a.id = [1; 16];
        a.name.assign(b"test");
        let mut b = a.clone();
        b.bitrate = 500;
        b.genre.assign(b"g");
        assert!(a.update(&b));
        assert_eq!(a.bitrate, 500);
        assert_eq!(a.genre.data, b"g");
        assert!(!a.update(&b));
        let q = ChanInfo::init_name_id(b"TE");
        assert!(a.match_name_id(&q));
        let q = ChanInfo::init_name_id(&crate::gnuid::to_str(&[1; 16]));
        assert_eq!(q.id, [1; 16]);
        assert!(q.name.is_empty());
    }
}
