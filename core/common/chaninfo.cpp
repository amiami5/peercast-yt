// ------------------------------------------------
// File : chaninfo.cpp
// Date: 4-apr-2002
// Author: giles
//
// (c) 2002 peercast.org
//
// ------------------------------------------------
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
// ------------------------------------------------

#include "str.h"
#include "chaninfo.h"
#include "pcp.h"
#include "chanmgr.h"
#include "playlist.h"
#include "cgi.h"

const ::String ChanInfo::T_UNKNOWN = "UNKNOWN";
const ::String ChanInfo::T_RAW = "RAW";
const ::String ChanInfo::T_MP3 = "MP3";
const ::String ChanInfo::T_OGG = "OGG";
const ::String ChanInfo::T_OGM = "OGM";
const ::String ChanInfo::T_MOV = "MOV";
const ::String ChanInfo::T_MPG = "MPG";
const ::String ChanInfo::T_FLV = "FLV";
const ::String ChanInfo::T_MKV = "MKV";
const ::String ChanInfo::T_WEBM = "WEBM";
const ::String ChanInfo::T_MP4 = "MP4";
const ::String ChanInfo::T_PLS = "PLS";

#ifdef WITH_RUST_CORE
#include "rustbridge.h"
#include "rustchan.h"

// 種類と MIME タイプの表、検索の一致、update で写す欄の判断、atom の組み立ては Rust 版
// (peercast-rs の chaninfo.rs)。欄を写すこと (String の代入) はここで行う。
#endif

// -----------------------------------
const char *ChanInfo::getTypeStr()
{
    return contentType.cstr();
}

// -----------------------------------
#ifdef WITH_RUST_CORE
std::string ChanInfo::getTypeStringLong()
{
    pcrs_chan_info v = rsInfo(*this);
    return rustbridge::RustBuf(pcrs_chaninfo_type_string_long(&v)).str();
}
#else
std::string ChanInfo::getTypeStringLong()
{
    std::string buf = std::string(getTypeStr()) +
        " (" + getMIMEType() + "; " + getTypeExt() + ")";

    if (MIMEType == "")
        buf += " [no styp]";
    if (streamExt == "")
        buf += " [no sext]";

    return buf;
}
#endif // WITH_RUST_CORE

// -----------------------------------
const char *ChanInfo::getTypeExt()
{
    if (streamExt.isEmpty()) {
        return getTypeExt(contentType);
    }
    else {
        return streamExt.cstr();
    }
}

// -----------------------------------
const char *ChanInfo::getMIMEType() const
{
    if (MIMEType.isEmpty()) {
        return getMIMEType(contentType);
    }
    else {
        return MIMEType.c_str();
    }
}

// -----------------------------------
const char *ChanInfo::getTypeStr(const TYPE& t)
{
    return t.c_str();
}

// -----------------------------------
#ifdef WITH_RUST_CORE
const char *ChanInfo::getProtocolStr(PROTOCOL t)
{
    return pcrs_chaninfo_protocol_str(t);
}
#else
const char *ChanInfo::getProtocolStr(PROTOCOL t)
{
    switch (t)
    {
        case SP_HTTP: return "HTTP";
        case SP_FILE: return "FILE";
        case SP_PCP: return "PCP";
        case SP_RTMP: return "RTMP";
        case SP_PIPE: return "PIPE";
        default: return "UNKNOWN";
    }
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
ChanInfo::PROTOCOL ChanInfo::getProtocolFromStr(const char *str)
{
    return static_cast<PROTOCOL>(pcrs_chaninfo_protocol_from_str(reinterpret_cast<const uint8_t*>(str), strlen(str)));
}
#else
ChanInfo::PROTOCOL ChanInfo::getProtocolFromStr(const char *str)
{
    if (Sys::stricmp(str, "HTTP")==0)
        return SP_HTTP;
    else if (Sys::stricmp(str, "FILE")==0)
        return SP_FILE;
    else if (Sys::stricmp(str, "PCP")==0)
        return SP_PCP;
    else if (Sys::stricmp(str, "RTMP")==0)
        return SP_RTMP;
    else
        return SP_UNKNOWN;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
const char *ChanInfo::getTypeExt(TYPE t)
{
    return pcrs_chaninfo_type_ext(reinterpret_cast<const uint8_t*>(t.cstr()), strlen(t.cstr()));
}
#else
const char *ChanInfo::getTypeExt(TYPE t)
{
    if (t == ChanInfo::T_OGM || t == ChanInfo::T_OGG)
        return ".ogg";
    else if (t == ChanInfo::T_MP3)
        return ".mp3";
    else if (t == ChanInfo::T_MOV)
        return ".mov";
    else if (t == ChanInfo::T_FLV)
        return ".flv";
    else if (t == ChanInfo::T_MKV)
        return ".mkv";
    else if (t == ChanInfo::T_WEBM)
        return ".webm";
    else if (t == ChanInfo::T_MP4)
        return ".mp4";
    else
        return "";
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
const char *ChanInfo::getMIMEType(TYPE t)
{
    return pcrs_chaninfo_mime_type(reinterpret_cast<const uint8_t*>(t.cstr()), strlen(t.cstr()));
}
#else
const char *ChanInfo::getMIMEType(TYPE t)
{
    if (t == ChanInfo::T_OGG)
        return MIME_XOGG;
    else if (t == ChanInfo::T_OGM)
        return MIME_XOGG;
    else if (t == ChanInfo::T_MP3)
        return MIME_MP3;
    else if (t == ChanInfo::T_MOV)
        return MIME_MOV;
    else if (t == ChanInfo::T_MPG)
        return MIME_MPG;
    else if (t == ChanInfo::T_FLV)
        return MIME_FLV;
    else if (t == ChanInfo::T_MKV)
        return MIME_MKV;
    else if (t == ChanInfo::T_WEBM)
        return MIME_WEBM;
    else if (t == ChanInfo::T_MP4)
        return MIME_MP4;
    else
        return "application/octet-stream";
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
ChanInfo::TYPE ChanInfo::getTypeFromMIME(const std::string& mediaType)
{
    return pcrs_chaninfo_type_from_mime(reinterpret_cast<const uint8_t*>(mediaType.data()), mediaType.size());
}
#else
ChanInfo::TYPE ChanInfo::getTypeFromMIME(const std::string& mediaType)
{
    if (mediaType == MIME_XOGG)
        return T_OGG;
    else if (mediaType == MIME_XOGG)
        return T_OGM;
    else if (mediaType == MIME_MP3)
        return T_MP3;
    else if (mediaType == MIME_MOV)
        return T_MOV;
    else if (mediaType == MIME_MPG)
        return T_MPG;
    else if (mediaType == MIME_FLV)
        return T_FLV;
    else if (mediaType == MIME_MKV)
        return T_MKV;
    else if (mediaType == MIME_WEBM)
        return T_WEBM;
    else
        return T_UNKNOWN;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
ChanInfo::TYPE ChanInfo::getTypeFromStr(const char *str)
{
    return pcrs_chaninfo_type_from_str(reinterpret_cast<const uint8_t*>(str), strlen(str));
}
#else
ChanInfo::TYPE ChanInfo::getTypeFromStr(const char *str)
{
    if (Sys::stricmp(str, "MP3")==0)
        return T_MP3;
    else if (Sys::stricmp(str, "OGG")==0)
        return T_OGG;
    else if (Sys::stricmp(str, "OGM")==0)
        return T_OGM;
    else if (Sys::stricmp(str, "RAW")==0)
        return T_RAW;
    else if (Sys::stricmp(str, "FLV")==0)
        return T_FLV;
    else if (Sys::stricmp(str, "MKV")==0)
        return T_MKV;
    else if (Sys::stricmp(str, "WEBM")==0)
        return T_WEBM;
    else if (Sys::stricmp(str, "MP4")==0)
        return T_MP4;
    else if (Sys::stricmp(str, "PLS")==0)
        return T_PLS;
    else if (Sys::stricmp(str, "M3U")==0)
        return T_PLS;
    else
        return T_UNKNOWN;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
bool    ChanInfo::matchNameID(ChanInfo &inf)
{
    pcrs_chan_info me = rsInfo(*this), q = rsInfo(inf);
    return pcrs_chaninfo_match(&me, &q, true);
}
#else
bool    ChanInfo::matchNameID(ChanInfo &inf)
{
    if (inf.id.isSet())
        if (id.isSame(inf.id))
            return true;

    if (!inf.name.isEmpty())
        if (name.contains(inf.name))
            return true;

    return false;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
bool    ChanInfo::match(ChanInfo &inf)
{
    pcrs_chan_info me = rsInfo(*this), q = rsInfo(inf);
    return pcrs_chaninfo_match(&me, &q, false);
}
#else
bool    ChanInfo::match(ChanInfo &inf)
{
    bool matchAny=true;

    if (inf.status != S_UNKNOWN)
    {
        if (status != inf.status)
            return false;
    }

    if (inf.bitrate != 0)
    {
        if (bitrate == inf.bitrate)
            return true;
        matchAny = false;
    }

    if (inf.id.isSet())
    {
        if (id.isSame(inf.id))
            return true;
        matchAny = false;
    }

    if (inf.contentType != T_UNKNOWN)
    {
        if (contentType == inf.contentType)
            return true;
        matchAny = false;
    }

    if (!inf.name.isEmpty())
    {
        if (name.contains(inf.name))
            return true;
        matchAny = false;
    }

    if (!inf.genre.isEmpty())
    {
        if (genre.contains(inf.genre))
            return true;
        matchAny = false;
    }

    return matchAny;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
bool ChanInfo::update(const ChanInfo &info)
{
    pcrs_chan_info me = rsInfo(*this), in = rsInfo(info);
    uint32_t copy = 0;
    int r = pcrs_chaninfo_update(&me, &in, &copy);
    if (r == 0)
        return false;
    if (r == 1)
    {
        LOG_ERROR("ChanInfo BC key not valid");
        return false;
    }
    if (r == 3)
        bcID = info.bcID;

    if (copy & PCRS_CI_BITRATE)       bitrate = info.bitrate;
    if (copy & PCRS_CI_CONTENT_TYPE)  contentType = info.contentType;
    if (copy & PCRS_CI_MIME)          MIMEType = info.MIMEType;
    if (copy & PCRS_CI_EXT)           streamExt = info.streamExt;
    if (copy & PCRS_CI_DESC)          desc = info.desc;
    if (copy & PCRS_CI_NAME)          name = info.name;
    if (copy & PCRS_CI_COMMENT)       comment = info.comment;
    if (copy & PCRS_CI_GENRE)         genre = info.genre;
    if (copy & PCRS_CI_URL)           url = info.url;
    if (copy & PCRS_CI_TRACK_CONTACT) track.contact = info.track.contact;
    if (copy & PCRS_CI_TRACK_TITLE)   track.title = info.track.title;
    if (copy & PCRS_CI_TRACK_ARTIST)  track.artist = info.track.artist;
    if (copy & PCRS_CI_TRACK_ALBUM)   track.album = info.track.album;
    if (copy & PCRS_CI_TRACK_GENRE)   track.genre = info.track.genre;
    return copy != 0;
}
#else
bool ChanInfo::update(const ChanInfo &info)
{
    bool changed = false;

    // check valid id
    if (!info.id.isSet())
        return false;

    // only update from chaninfo that has full name etc..
    if (info.name.isEmpty())
        return false;

    // check valid broadcaster key
    if (bcID.isSet())
    {
        if (!bcID.isSame(info.bcID))
        {
            LOG_ERROR("ChanInfo BC key not valid");
            return false;
        }
    }else
    {
        bcID = info.bcID;
    }

    if (bitrate != info.bitrate)
    {
        bitrate = info.bitrate;
        changed = true;
    }

    if (!contentType.isSame(info.contentType))
    {
        contentType = info.contentType;
        changed = true;
    }

    if (!MIMEType.isSame(info.MIMEType))
    {
        MIMEType = info.MIMEType;
        changed = true;
    }

    if (!streamExt.isSame(info.streamExt))
    {
        streamExt = info.streamExt;
        changed = true;
    }

    if (!desc.isSame(info.desc))
    {
        desc = info.desc;
        changed = true;
    }

    if (!name.isSame(info.name))
    {
        name = info.name;
        changed = true;
    }

    if (!comment.isSame(info.comment))
    {
        comment = info.comment;
        changed = true;
    }

    if (!genre.isSame(info.genre))
    {
        genre = info.genre;
        changed = true;
    }

    if (!url.isSame(info.url))
    {
        url = info.url;
        changed = true;
    }

    if (track.update(info.track))
        changed = true;

    return changed;
}
#endif // WITH_RUST_CORE

// -----------------------------------
void ChanInfo::initNameID(const char *n)
{
    init();
    id.fromStr(n);
    if (!id.isSet())
        name.set(n);
}

// -----------------------------------
void ChanInfo::init()
{
    status = S_UNKNOWN;
    name.clear();
    bitrate = 0;
    contentType = T_UNKNOWN;
    MIMEType.clear();
    streamExt.clear();
    srcProtocol = SP_UNKNOWN;
    id.clear();
    url.clear();
    genre.clear();
    comment.clear();
    track.clear();
    lastPlayStart = 0;
    lastPlayEnd = 0;
    numSkips = 0;
    bcID.clear();
    createdTime = 0;
}

// -----------------------------------
unsigned int ChanInfo::getUptime()
{
    // calculate uptime and cap if requested by settings.
    unsigned int upt;
    upt = lastPlayStart?(sys->getTime()-lastPlayStart):0;
    if (chanMgr->maxUptime)
        if (upt > chanMgr->maxUptime)
            upt = chanMgr->maxUptime;
    return upt;
}

// -----------------------------------
unsigned int ChanInfo::getAge()
{
    return sys->getTime()-createdTime;
}

#ifndef WITH_RUST_CORE
// ------------------------------------------
// WITH_RUST_CORE のビルドでは peercast-rs (src/pcp) が読む
void ChanInfo::readTrackAtoms(AtomStream &atom, int numc)
{
    for (int i=0; i<numc; i++)
    {
        int c, d;
        ID4 id = atom.read(c, d);
        if (id == PCP_CHAN_TRACK_TITLE)
        {
            atom.readString(track.title.data, sizeof(track.title.data), d);
        }else if (id == PCP_CHAN_TRACK_CREATOR)
        {
            atom.readString(track.artist.data, sizeof(track.artist.data), d);
        }else if (id == PCP_CHAN_TRACK_URL)
        {
            atom.readString(track.contact.data, sizeof(track.contact.data), d);
            if (!str::is_http_url(track.contact.cstr()))
                track.contact.clear();
        }else if (id == PCP_CHAN_TRACK_ALBUM)
        {
            atom.readString(track.album.data, sizeof(track.album.data), d);
        }else
            atom.skip(c, d);
    }
}

// ------------------------------------------
void ChanInfo::readInfoAtoms(AtomStream &atom, int numc)
{
    for (int i=0; i<numc; i++)
    {
        int c, d;
        ID4 id = atom.read(c, d);
        if (id == PCP_CHAN_INFO_NAME)
        {
            atom.readString(name.data, sizeof(name.data), d);
        }else if (id == PCP_CHAN_INFO_BITRATE)
        {
            bitrate = atom.readInt();
        }else if (id == PCP_CHAN_INFO_GENRE)
        {
            atom.readString(genre.data, sizeof(genre.data), d);
        }else if (id == PCP_CHAN_INFO_URL)
        {
            atom.readString(url.data, sizeof(url.data), d);
            // 他のノードから届いた URL は、UI でリンクとして表示される。
            // "javascript:" などが入り込まないよう、http(s) のみ許可する。
            if (!str::is_http_url(url.cstr()))
                url.clear();
        }else if (id == PCP_CHAN_INFO_DESC)
        {
            atom.readString(desc.data, sizeof(desc.data), d);
        }else if (id == PCP_CHAN_INFO_COMMENT)
        {
            atom.readString(comment.data, sizeof(comment.data), d);
        }else if (id == PCP_CHAN_INFO_TYPE)
        {
            atom.readString(contentType.data, sizeof(contentType.data), d);
        }else if (id == PCP_CHAN_INFO_STREAMTYPE)
        {
            atom.readString(MIMEType.data, sizeof(MIMEType.data), d);
        }else if (id == PCP_CHAN_INFO_STREAMEXT)
        {
            atom.readString(streamExt.data, sizeof(streamExt.data), d);
        }else
            atom.skip(c, d);
    }
}
#endif

// -----------------------------------
#ifdef WITH_RUST_CORE
void ChanInfo::writeInfoAtoms(AtomStream &atom)
{
    pcrs_chan_info v = rsInfo(*this);
    rustbridge::RustBuf b(pcrs_chaninfo_write_atoms(&v, false));
    std::string bytes = b.str();
    atom.io.write(bytes.data(), bytes.size());
}
#else
void ChanInfo::writeInfoAtoms(AtomStream &atom)
{
    int natoms = 7;

    natoms += !MIMEType.isEmpty();
    natoms += !streamExt.isEmpty();

    atom.writeParent(PCP_CHAN_INFO, natoms);
        atom.writeString(PCP_CHAN_INFO_NAME, name.cstr());
        atom.writeInt(PCP_CHAN_INFO_BITRATE, bitrate);
        atom.writeString(PCP_CHAN_INFO_GENRE, genre.cstr());
        atom.writeString(PCP_CHAN_INFO_URL, url.cstr());
        atom.writeString(PCP_CHAN_INFO_DESC, desc.cstr());
        atom.writeString(PCP_CHAN_INFO_COMMENT, comment.cstr());
        atom.writeString(PCP_CHAN_INFO_TYPE, getTypeStr());
        if (!MIMEType.isEmpty())
            atom.writeString(PCP_CHAN_INFO_STREAMTYPE, MIMEType.cstr());
        if (!streamExt.isEmpty())
            atom.writeString(PCP_CHAN_INFO_STREAMEXT, streamExt.cstr());
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
void ChanInfo::writeTrackAtoms(AtomStream &atom)
{
    pcrs_chan_info v = rsInfo(*this);
    rustbridge::RustBuf b(pcrs_chaninfo_write_atoms(&v, true));
    std::string bytes = b.str();
    atom.io.write(bytes.data(), bytes.size());
}
#else
void ChanInfo::writeTrackAtoms(AtomStream &atom)
{
    atom.writeParent(PCP_CHAN_TRACK, 4);
        atom.writeString(PCP_CHAN_TRACK_TITLE, track.title.cstr());
        atom.writeString(PCP_CHAN_TRACK_CREATOR, track.artist.cstr());
        atom.writeString(PCP_CHAN_TRACK_URL, track.contact.cstr());
        atom.writeString(PCP_CHAN_TRACK_ALBUM, track.album.cstr());
}
#endif // WITH_RUST_CORE

// -----------------------------------
XML::Node *ChanInfo::createChannelXML()
{
    String nameUNI = name;
    nameUNI.convertTo(String::T_UNICODESAFE);

    String urlUNI = url;
    urlUNI.convertTo(String::T_UNICODESAFE);

    String genreUNI = genre;
    genreUNI.convertTo(String::T_UNICODESAFE);

    String descUNI = desc;
    descUNI.convertTo(String::T_UNICODESAFE);

    String commentUNI = comment;
    commentUNI.convertTo(String::T_UNICODESAFE);

    return new XML::Node("channel name=\"%s\" id=\"%s\" bitrate=\"%d\" type=\"%s\" genre=\"%s\" desc=\"%s\" url=\"%s\" uptime=\"%d\" comment=\"%s\" skips=\"%d\" age=\"%d\" bcflags=\"%d\"",
        nameUNI.cstr(),
        id.str().c_str(),
        bitrate,
        getTypeStr(),
        genreUNI.cstr(),
        descUNI.cstr(),
        urlUNI.cstr(),
        getUptime(),
        commentUNI.cstr(),
        numSkips,
        getAge(),
        bcID.getFlags()
        );
}

// -----------------------------------
XML::Node *ChanInfo::createRelayChannelXML()
{
    return new XML::Node("channel id=\"%s\" uptime=\"%d\" skips=\"%d\" age=\"%d\"",
        id.str().c_str(),
        getUptime(),
        numSkips,
        getAge()
        );
}

// -----------------------------------
XML::Node *ChanInfo::createTrackXML()
{
    String titleUNI = track.title;
    titleUNI.convertTo(String::T_UNICODESAFE);

    String artistUNI = track.artist;
    artistUNI.convertTo(String::T_UNICODESAFE);

    String albumUNI = track.album;
    albumUNI.convertTo(String::T_UNICODESAFE);

    String genreUNI = track.genre;
    genreUNI.convertTo(String::T_UNICODESAFE);

    String contactUNI = track.contact;
    contactUNI.convertTo(String::T_UNICODESAFE);

    return new XML::Node("track title=\"%s\" artist=\"%s\" album=\"%s\" genre=\"%s\" contact=\"%s\"",
        titleUNI.cstr(),
        artistUNI.cstr(),
        albumUNI.cstr(),
        genreUNI.cstr(),
        contactUNI.cstr()
        );
}

// -----------------------------------
void ChanInfo::init(const char *n, GnuID &cid, TYPE tp, int br)
{
    init();

    name.set(n);
    bitrate = br;
    contentType = tp;
    id = cid;
}

// -----------------------------------
void ChanInfo::init(const char *fn)
{
    init();

    if (fn)
        name.set(fn);
}

// -----------------------------------
void ChanInfo::setContentType(TYPE type)
{
    this->contentType    = getTypeFromStr(type.cstr());
    this->MIMEType       = getMIMEType(type);
    this->streamExt      = getTypeExt(type);
}

// -----------------------------------
#ifdef WITH_RUST_CORE
bool TrackInfo::update(const TrackInfo &inf)
{
    pcrs_chan_info me = {}, in = {};
    rsTrack(me, *this);
    rsTrack(in, inf);
    uint32_t copy = pcrs_trackinfo_update(&me, &in);
    if (copy & PCRS_CI_TRACK_CONTACT) contact = inf.contact;
    if (copy & PCRS_CI_TRACK_TITLE)   title = inf.title;
    if (copy & PCRS_CI_TRACK_ARTIST)  artist = inf.artist;
    if (copy & PCRS_CI_TRACK_ALBUM)   album = inf.album;
    if (copy & PCRS_CI_TRACK_GENRE)   genre = inf.genre;
    return copy != 0;
}
#else
bool TrackInfo::update(const TrackInfo &inf)
{
    bool changed = false;

    if (!contact.isSame(inf.contact))
    {
        contact = inf.contact;
        changed = true;
    }

    if (!title.isSame(inf.title))
    {
        title = inf.title;
        changed = true;
    }

    if (!artist.isSame(inf.artist))
    {
        artist = inf.artist;
        changed = true;
    }

    if (!album.isSame(inf.album))
    {
        album = inf.album;
        changed = true;
    }

    if (!genre.isSame(inf.genre))
    {
        genre = inf.genre;
        changed = true;
    }

    return changed;
}
#endif // WITH_RUST_CORE

// -----------------------------------
#ifdef WITH_RUST_CORE
const char* ChanInfo::getPlayListExt()
{
    return pcrs_chaninfo_playlist_ext(reinterpret_cast<const uint8_t*>(contentType.cstr()), strlen(contentType.cstr()));
}
#else
const char* ChanInfo::getPlayListExt()
{
    switch (PlayList::getPlayListType(contentType))
    {
    case PlayList::T_RAM:
        return ".ram";
    case PlayList::T_PLS:
        return ".m3u"; // or could be .pls ...
    case PlayList::T_SCPLS:
        return ".pls";
    case PlayList::T_NONE:
    default:
        return "";
    }
}
#endif // WITH_RUST_CORE

// -----------------------------------
amf0::Value ChanInfo::getState()
{
    return amf0::Value::object(
        {
            {"name", name.c_str()},
            {"desc", desc.c_str()},
            {"genre", genre.c_str()},
            {"url", url.c_str()},
            {"comment", comment.c_str()},
        });
}

