// ------------------------------------------------
// File : mp3.h
// Date: 28-may-2003
// Author: giles
//
// (c) 2002-3 peercast.org
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

#ifndef _MP3_H
#define _MP3_H

#include "channel.h"

#ifdef WITH_RUST_CORE
// WITH_RUST_CORE のときは peercast-rs (src/media/mp3.rs) の実装を使う。
#include "rustmedia.h"

// ----------------------------------------------
class MP3Stream : public rustbridge::MediaStream
{
public:
    MP3Stream() : MediaStream(PCRS_MEDIA_MP3) {}
};

#else // WITH_RUST_CORE

// ----------------------------------------------
class MP3Stream : public ChannelStream
{
public:
    void    readHeader(Stream &, std::shared_ptr<Channel>) override;
    int     readPacket(Stream &, std::shared_ptr<Channel>) override;
    void    readEnd(Stream &, std::shared_ptr<Channel>) override;
};

#endif // WITH_RUST_CORE

#endif
