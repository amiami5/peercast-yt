//! チャンネルの情報 (core/common/chaninfo.cpp の `ChanInfo` と `TrackInfo`) のうち、状態を持たない
//! 部分。種類と MIME タイプの表、検索の一致、`update` で写す欄の判断、info と track の atom。
//!
//! 文字列は C++ の `String` (NUL で終わる固定長のバッファ) の中身なので、どれも NUL の手前までを
//! 使う。`ChanInfo` の実体と、欄を写すこと (`String` の代入は文字コードの種類も写す) は C++ 側。

use crate::http::stristr;
use crate::pcp::write::{c_str, AtomBuf};
use crate::pcp::*;

pub const T_UNKNOWN: &[u8] = b"UNKNOWN";
pub const T_RAW: &[u8] = b"RAW";
pub const T_MP3: &[u8] = b"MP3";
pub const T_OGG: &[u8] = b"OGG";
pub const T_OGM: &[u8] = b"OGM";
pub const T_MOV: &[u8] = b"MOV";
pub const T_MPG: &[u8] = b"MPG";
pub const T_FLV: &[u8] = b"FLV";
pub const T_MKV: &[u8] = b"MKV";
pub const T_WEBM: &[u8] = b"WEBM";
pub const T_MP4: &[u8] = b"MP4";
pub const T_PLS: &[u8] = b"PLS";

const MIME_MP3: &[u8] = b"audio/mpeg";
const MIME_XOGG: &[u8] = b"application/x-ogg";
const MIME_MOV: &[u8] = b"video/quicktime";
const MIME_MPG: &[u8] = b"video/mpeg";
const MIME_FLV: &[u8] = b"video/x-flv";
const MIME_MKV: &[u8] = b"video/x-matroska";
const MIME_WEBM: &[u8] = b"video/webm";
const MIME_MP4: &[u8] = b"video/mp4";

/// `getTypeExt(TYPE)`
pub fn type_ext(t: &[u8]) -> &'static [u8] {
    match c_str(t) {
        T_OGM | T_OGG => b".ogg",
        T_MP3 => b".mp3",
        T_MOV => b".mov",
        T_FLV => b".flv",
        T_MKV => b".mkv",
        T_WEBM => b".webm",
        T_MP4 => b".mp4",
        _ => b"",
    }
}

/// `getMIMEType(TYPE)`
pub fn mime_type(t: &[u8]) -> &'static [u8] {
    match c_str(t) {
        T_OGG | T_OGM => MIME_XOGG,
        T_MP3 => MIME_MP3,
        T_MOV => MIME_MOV,
        T_MPG => MIME_MPG,
        T_FLV => MIME_FLV,
        T_MKV => MIME_MKV,
        T_WEBM => MIME_WEBM,
        T_MP4 => MIME_MP4,
        _ => b"application/octet-stream",
    }
}

/// `getTypeFromMIME`。`std::string` の比較なので NUL も含めて比べる。C++ 版と同じく、
/// OGM と MP4 にはならない (OGM の行は OGG と同じ MIME タイプで、先の OGG に当たる)。
pub fn type_from_mime(m: &[u8]) -> &'static [u8] {
    match m {
        MIME_XOGG => T_OGG,
        MIME_MP3 => T_MP3,
        MIME_MOV => T_MOV,
        MIME_MPG => T_MPG,
        MIME_FLV => T_FLV,
        MIME_MKV => T_MKV,
        MIME_WEBM => T_WEBM,
        _ => T_UNKNOWN,
    }
}

/// `Sys::stricmp(a, b) == 0` (ASCII の大文字小文字を無視)
fn stricmp_eq(a: &[u8], b: &[u8]) -> bool {
    c_str(a).eq_ignore_ascii_case(b)
}

/// `getTypeFromStr`
pub fn type_from_str(s: &[u8]) -> &'static [u8] {
    for (name, t) in [
        (&b"MP3"[..], T_MP3),
        (b"OGG", T_OGG),
        (b"OGM", T_OGM),
        (b"RAW", T_RAW),
        (b"FLV", T_FLV),
        (b"MKV", T_MKV),
        (b"WEBM", T_WEBM),
        (b"MP4", T_MP4),
        (b"PLS", T_PLS),
        (b"M3U", T_PLS),
    ] {
        if stricmp_eq(s, name) {
            return t;
        }
    }
    T_UNKNOWN
}

/// `PROTOCOL`
pub const SP_UNKNOWN: i32 = 0;
pub const SP_HTTP: i32 = 1;
pub const SP_FILE: i32 = 2;
pub const SP_PCP: i32 = 3;
pub const SP_RTMP: i32 = 4;
pub const SP_PIPE: i32 = 5;

pub fn protocol_str(p: i32) -> &'static [u8] {
    match p {
        SP_HTTP => b"HTTP",
        SP_FILE => b"FILE",
        SP_PCP => b"PCP",
        SP_RTMP => b"RTMP",
        SP_PIPE => b"PIPE",
        _ => b"UNKNOWN",
    }
}

/// `getProtocolFromStr` (C++ 版と同じく PIPE にはならない)
pub fn protocol_from_str(s: &[u8]) -> i32 {
    for (name, p) in [(&b"HTTP"[..], SP_HTTP), (b"FILE", SP_FILE), (b"PCP", SP_PCP), (b"RTMP", SP_RTMP)] {
        if stricmp_eq(s, name) {
            return p;
        }
    }
    SP_UNKNOWN
}

/// `getPlayListExt` (`PlayList::getPlayListType` は OGM なら RAM、それ以外は PLS)
pub fn playlist_ext(content_type: &[u8]) -> &'static [u8] {
    if c_str(content_type) == T_OGM {
        b".ram"
    } else {
        b".m3u"
    }
}

/// `getMIMEType()`: 設定された MIME タイプか、なければ種類から
pub fn effective_mime<'a>(content_type: &[u8], mime: &'a [u8]) -> &'a [u8] {
    let m = c_str(mime);
    if m.is_empty() {
        mime_type(content_type)
    } else {
        m
    }
}

/// `getTypeExt()`: 設定された拡張子か、なければ種類から
pub fn effective_ext<'a>(content_type: &[u8], ext: &'a [u8]) -> &'a [u8] {
    let e = c_str(ext);
    if e.is_empty() {
        type_ext(content_type)
    } else {
        e
    }
}

/// `getTypeStringLong`
pub fn type_string_long(content_type: &[u8], mime: &[u8], ext: &[u8]) -> Vec<u8> {
    let mut v = c_str(content_type).to_vec();
    v.extend_from_slice(b" (");
    v.extend_from_slice(effective_mime(content_type, mime));
    v.extend_from_slice(b"; ");
    v.extend_from_slice(effective_ext(content_type, ext));
    v.extend_from_slice(b")");
    if c_str(mime).is_empty() {
        v.extend_from_slice(b" [no styp]");
    }
    if c_str(ext).is_empty() {
        v.extend_from_slice(b" [no sext]");
    }
    v
}

/// `TrackInfo` の文字列
#[derive(Clone, Copy, Default)]
pub struct Track<'a> {
    pub contact: &'a [u8],
    pub title: &'a [u8],
    pub artist: &'a [u8],
    pub album: &'a [u8],
    pub genre: &'a [u8],
}

/// `ChanInfo` のうち、ここで使う欄
#[derive(Clone, Copy, Default)]
pub struct Info<'a> {
    pub name: &'a [u8],
    pub id: [u8; 16],
    pub bcid: [u8; 16],
    pub bitrate: i32,
    pub content_type: &'a [u8],
    pub mime: &'a [u8],
    pub ext: &'a [u8],
    pub status: i32,
    pub desc: &'a [u8],
    pub genre: &'a [u8],
    pub url: &'a [u8],
    pub comment: &'a [u8],
    pub track: Track<'a>,
}

fn is_set(id: &[u8; 16]) -> bool {
    id.iter().any(|&b| b != 0)
}

/// `String::contains` (大文字小文字を無視して含むか。空の文字列は含まない扱い)
fn contains(s: &[u8], sub: &[u8]) -> bool {
    stristr(c_str(s), c_str(sub)).is_some()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    c_str(a) == c_str(b)
}

/// `matchNameID`
pub fn match_name_id(me: &Info, q: &Info) -> bool {
    (is_set(&q.id) && me.id == q.id) || (!c_str(q.name).is_empty() && contains(me.name, q.name))
}

/// `match(ChanInfo&)`: 状態が指定されていれば一致が必要。ほかの指定された項目のどれかが一致すれば
/// true、どれも指定されていなければ true。
pub fn match_info(me: &Info, q: &Info) -> bool {
    if q.status != 0 && me.status != q.status {
        return false;
    }
    let mut match_any = true;
    let checks: [(bool, &dyn Fn() -> bool); 5] = [
        (q.bitrate != 0, &|| me.bitrate == q.bitrate),
        (is_set(&q.id), &|| me.id == q.id),
        (!same(q.content_type, T_UNKNOWN), &|| same(me.content_type, q.content_type)),
        (!c_str(q.name).is_empty(), &|| contains(me.name, q.name)),
        (!c_str(q.genre).is_empty(), &|| contains(me.genre, q.genre)),
    ];
    for (given, ok) in checks {
        if given {
            if ok() {
                return true;
            }
            match_any = false;
        }
    }
    match_any
}

/// `update` で写す欄 (ビット)
pub const U_BITRATE: u32 = 1 << 0;
pub const U_CONTENT_TYPE: u32 = 1 << 1;
pub const U_MIME: u32 = 1 << 2;
pub const U_EXT: u32 = 1 << 3;
pub const U_DESC: u32 = 1 << 4;
pub const U_NAME: u32 = 1 << 5;
pub const U_COMMENT: u32 = 1 << 6;
pub const U_GENRE: u32 = 1 << 7;
pub const U_URL: u32 = 1 << 8;
pub const U_TRACK_CONTACT: u32 = 1 << 9;
pub const U_TRACK_TITLE: u32 = 1 << 10;
pub const U_TRACK_ARTIST: u32 = 1 << 11;
pub const U_TRACK_ALBUM: u32 = 1 << 12;
pub const U_TRACK_GENRE: u32 = 1 << 13;

/// `update` の結果
#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    /// ID か名前がない情報なので使わない (false を返す)
    Ignore,
    /// 配信者の鍵が違う (`ChanInfo BC key not valid` を出して false を返す)
    BadKey,
    /// `set_bcid` なら配信者の鍵を写す。`copy` の欄を写す。写す欄があれば true を返す
    Apply { set_bcid: bool, copy: u32 },
}

/// `TrackInfo::update` で写す欄
pub fn track_update(me: &Track, t: &Track) -> u32 {
    let mut c = 0;
    for (a, b, bit) in [
        (me.contact, t.contact, U_TRACK_CONTACT),
        (me.title, t.title, U_TRACK_TITLE),
        (me.artist, t.artist, U_TRACK_ARTIST),
        (me.album, t.album, U_TRACK_ALBUM),
        (me.genre, t.genre, U_TRACK_GENRE),
    ] {
        if !same(a, b) {
            c |= bit;
        }
    }
    c
}

/// `ChanInfo::update`
pub fn update(me: &Info, info: &Info) -> Update {
    if !is_set(&info.id) || c_str(info.name).is_empty() {
        return Update::Ignore;
    }
    let set_bcid = !is_set(&me.bcid);
    if !set_bcid && me.bcid != info.bcid {
        return Update::BadKey;
    }
    let mut c = 0;
    if me.bitrate != info.bitrate {
        c |= U_BITRATE;
    }
    for (a, b, bit) in [
        (me.content_type, info.content_type, U_CONTENT_TYPE),
        (me.mime, info.mime, U_MIME),
        (me.ext, info.ext, U_EXT),
        (me.desc, info.desc, U_DESC),
        (me.name, info.name, U_NAME),
        (me.comment, info.comment, U_COMMENT),
        (me.genre, info.genre, U_GENRE),
        (me.url, info.url, U_URL),
    ] {
        if !same(a, b) {
            c |= bit;
        }
    }
    c |= track_update(&me.track, &info.track);
    Update::Apply { set_bcid, copy: c }
}

/// `writeInfoAtoms`
pub fn write_info_atoms(out: &mut AtomBuf, i: &Info) {
    let has_mime = !c_str(i.mime).is_empty();
    let has_ext = !c_str(i.ext).is_empty();
    out.parent(PCP_CHAN_INFO, 7 + has_mime as i32 + has_ext as i32);
    out.string(PCP_CHAN_INFO_NAME, i.name);
    out.int(PCP_CHAN_INFO_BITRATE, i.bitrate);
    out.string(PCP_CHAN_INFO_GENRE, i.genre);
    out.string(PCP_CHAN_INFO_URL, i.url);
    out.string(PCP_CHAN_INFO_DESC, i.desc);
    out.string(PCP_CHAN_INFO_COMMENT, i.comment);
    out.string(PCP_CHAN_INFO_TYPE, i.content_type);
    if has_mime {
        out.string(PCP_CHAN_INFO_STREAMTYPE, i.mime);
    }
    if has_ext {
        out.string(PCP_CHAN_INFO_STREAMEXT, i.ext);
    }
}

/// `writeTrackAtoms`
pub fn write_track_atoms(out: &mut AtomBuf, t: &Track) {
    out.parent(PCP_CHAN_TRACK, 4);
    out.string(PCP_CHAN_TRACK_TITLE, t.title);
    out.string(PCP_CHAN_TRACK_CREATOR, t.artist);
    out.string(PCP_CHAN_TRACK_URL, t.contact);
    out.string(PCP_CHAN_TRACK_ALBUM, t.album);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcp::atom::{AtomStream, MemStream};

    #[test]
    fn tables() {
        assert_eq!(type_ext(b"OGM"), b".ogg");
        assert_eq!(type_ext(b"MPG"), b"");
        assert_eq!(mime_type(b"MPG"), b"video/mpeg");
        assert_eq!(mime_type(b"RAW"), b"application/octet-stream");
        assert_eq!(type_from_mime(b"application/x-ogg"), T_OGG);
        assert_eq!(type_from_mime(b"video/mp4"), T_UNKNOWN);
        assert_eq!(type_from_str(b"m3u"), T_PLS);
        assert_eq!(type_from_str(b"webm\0x"), T_WEBM);
        assert_eq!(type_from_str(b"webmx"), T_UNKNOWN);
        assert_eq!(protocol_from_str(b"pcp"), SP_PCP);
        assert_eq!(protocol_from_str(b"PIPE"), SP_UNKNOWN);
        assert_eq!(protocol_str(9), b"UNKNOWN");
        assert_eq!(playlist_ext(b"OGM"), b".ram");
        assert_eq!(type_string_long(b"FLV", b"", b""), b"FLV (video/x-flv; .flv) [no styp] [no sext]");
        assert_eq!(type_string_long(b"FLV", b"a/b", b".x"), b"FLV (a/b; .x)");
    }

    #[test]
    fn matching() {
        let me = Info { name: b"Hello World", genre: b"Game", bitrate: 500, content_type: b"FLV", ..Default::default() };
        assert!(match_info(&me, &Info { content_type: T_UNKNOWN, ..Default::default() }));
        assert!(match_info(&me, &Info { name: b"world", content_type: T_UNKNOWN, ..Default::default() }));
        assert!(!match_info(&me, &Info { name: b"xyz", content_type: T_UNKNOWN, ..Default::default() }));
        assert!(match_info(&me, &Info { name: b"xyz", bitrate: 500, content_type: T_UNKNOWN, ..Default::default() }));
        assert!(!match_info(&me, &Info { status: 1, content_type: T_UNKNOWN, ..Default::default() }));
        assert!(match_name_id(&me, &Info { name: b"HELLO", ..Default::default() }));
    }

    #[test]
    fn updating() {
        let mut id = [0u8; 16];
        id[0] = 1;
        let me = Info { name: b"a", bcid: id, ..Default::default() };
        assert_eq!(update(&me, &Info { name: b"a", ..Default::default() }), Update::Ignore);
        assert_eq!(update(&me, &Info { name: b"a", id, ..Default::default() }), Update::BadKey);
        let me = Info { name: b"a", ..Default::default() };
        let t = Track { title: b"t", ..Default::default() };
        assert_eq!(
            update(&me, &Info { name: b"b", id, bitrate: 1, track: t, ..Default::default() }),
            Update::Apply { set_bcid: true, copy: U_NAME | U_BITRATE | U_TRACK_TITLE }
        );
    }

    #[test]
    fn info_atoms_read_back() {
        let i = Info { name: b"ch\0junk", bitrate: 300, content_type: b"FLV", mime: b"video/x-flv", ..Default::default() };
        let mut b = AtomBuf::default();
        write_info_atoms(&mut b, &i);
        let mut buf = b.0.clone();
        let mut a = AtomStream::new(MemStream::new(&mut buf));
        let (id, c, _) = a.read().unwrap();
        assert_eq!((id, c), (PCP_CHAN_INFO, 8));
        let (id, _, d) = a.read().unwrap();
        assert_eq!((id, d), (PCP_CHAN_INFO_NAME, 3));
        assert_eq!(a.read_bytes_max(3, 3).unwrap(), b"ch\0");
    }
}
