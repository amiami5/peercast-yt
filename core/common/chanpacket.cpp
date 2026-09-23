// ------------------------------------------------
// File : cstream.cpp
// Date: 4-apr-2002
// Author: giles
// Desc:
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

#include "chanpacket.h"
#include "sys.h"
#include "stream.h"

// -----------------------------------
void ChanPacket::init(TYPE t, const void *p, unsigned int l, unsigned int _pos)
{
    type = t;
    if (l > MAX_DATALEN)
        throw StreamException("Packet data too large");
    len = l;
    memcpy(data, p, len);
    pos = _pos;
}

// -----------------------------------
void ChanPacket::writeRaw(Stream &out)
{
    out.write(data, len);
}

// -----------------------------------
ChanPacket& ChanPacket::operator=(const ChanPacket& other)
{
    this->type = other.type;
    this->len  = other.len;
    this->pos  = other.pos;
    this->sync = other.sync;
    this->cont = other.cont;
    memcpy(this->data, other.data, this->len);

    return *this;
}

#ifdef WITH_RUST_CORE
// ChanPacketBuffer の処理は peercast-rs (src/chanpacket.rs)。パケットと位置はこのクラスのメンバーの
// まま、そこを指すもの (pcrs_cpb) を渡す。ロックと readPacket の待ち合わせはここで行う。

#include <cstddef>

static_assert(sizeof(ChanPacket) == sizeof(pcrs_chan_packet), "ChanPacket layout");
static_assert(offsetof(ChanPacket, type) == offsetof(pcrs_chan_packet, type), "ChanPacket layout");
static_assert(offsetof(ChanPacket, len) == offsetof(pcrs_chan_packet, len), "ChanPacket layout");
static_assert(offsetof(ChanPacket, pos) == offsetof(pcrs_chan_packet, pos), "ChanPacket layout");
static_assert(offsetof(ChanPacket, sync) == offsetof(pcrs_chan_packet, sync), "ChanPacket layout");
static_assert(offsetof(ChanPacket, cont) == offsetof(pcrs_chan_packet, cont), "ChanPacket layout");
static_assert(offsetof(ChanPacket, data) == offsetof(pcrs_chan_packet, data), "ChanPacket layout");
static_assert(sizeof(unsigned int) == sizeof(uint32_t), "unsigned int is 32 bits");

static pcrs_chan_packet* rp(ChanPacket& p) { return reinterpret_cast<pcrs_chan_packet*>(&p); }
static uint32_t* rp(volatile unsigned int& v) { return reinterpret_cast<uint32_t*>(const_cast<unsigned int*>(&v)); }
static uint32_t* rp(unsigned int& v) { return reinterpret_cast<uint32_t*>(&v); }

pcrs_cpb ChanPacketBuffer::view()
{
    return { rp(packets[0]), rp(lastPos), rp(firstPos), rp(safePos), rp(readPos), rp(writePos), rp(accept), rp(lastWriteTime) };
}

// -----------------------------------
// (使われていないようだ。)
int ChanPacketBuffer::copyFrom(ChanPacketBuffer &buf, unsigned int reqPos)
{
    lock.lock();
    buf.lock.lock();

    pcrs_cpb v = view(), src = buf.view();
    int r = pcrs_cpb_copy_from(&v, &src, reqPos);

    buf.lock.unlock();
    lock.unlock();
    return r;
}

bool ChanPacketBuffer::findPacket(unsigned int spos, ChanPacket &pack)
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_find_packet(&v, spos, rp(pack));
}

unsigned int ChanPacketBuffer::getLatestPos()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_LATEST_POS, 0);
}

unsigned int ChanPacketBuffer::getLatestNonContinuationPos()
{
    if (writePos == 0)
        return 0;

    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_LATEST_NONCONT_POS, 0);
}

unsigned int ChanPacketBuffer::getOldestNonContinuationPos()
{
    if (writePos == 0)
        return 0;

    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_OLDEST_NONCONT_POS, 0);
}

unsigned int ChanPacketBuffer::getOldestPos()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_OLDEST_POS, 0);
}

// C++ 版と同じく、ロックを取らずに読む
unsigned int ChanPacketBuffer::findOldestPos(unsigned int spos)
{
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_FIND_OLDEST_POS, spos);
}

unsigned int ChanPacketBuffer::getStreamPos(unsigned int index)
{
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_STREAM_POS, index);
}

unsigned int ChanPacketBuffer::getStreamPosEnd(unsigned int index)
{
    pcrs_cpb v = view();
    return pcrs_cpb_pos(&v, PCRS_CPB_STREAM_POS_END, index);
}

bool ChanPacketBuffer::writePacket(ChanPacket &pack, bool updateReadPos)
{
    if (pack.len == 0)
        return false;
    // C++ 版と同じく、willSkip は書き込みとは別のロックで確かめ、時刻はロックの中で読む
    if (willSkip())
        return false;
    std::lock_guard<std::recursive_mutex> cs(lock);
    unsigned int now = sys->getTime();
    pcrs_cpb v = view();
    return pcrs_cpb_write_packet(&v, rp(pack), updateReadPos, now);
}

void ChanPacketBuffer::readPacket(ChanPacket &pack)
{
    unsigned int tim = sys->getTime();

    lock.lock();
    pcrs_cpb v = view();

    if (pcrs_cpb_read_state(&v, true) == 1)
    {
        lock.unlock();
        throw StreamException("Read too far behind");
    }

    while (pcrs_cpb_read_state(&v, false) == 2)
    {
        lock.unlock();
        sys->sleepIdle();
        if ((sys->getTime() - tim) > 30)
        {
            throw TimeoutException();
        }
        lock.lock();
    }

    pcrs_cpb_take(&v, rp(pack));
    lock.unlock();

    sys->sleepIdle();
}

bool ChanPacketBuffer::willSkip()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    pcrs_cpb v = view();
    return pcrs_cpb_will_skip(&v);
}

#else
// -----------------------------------
// (使われていないようだ。)
int ChanPacketBuffer::copyFrom(ChanPacketBuffer &buf, unsigned int reqPos)
{
    lock.lock();
    buf.lock.lock();

    firstPos = 0;
    lastPos = 0;
    safePos = 0;
    readPos = 0;

    for (unsigned int i = buf.firstPos; i <= buf.lastPos; i++)
    {
        ChanPacket *src = &buf.packets[i%MAX_PACKETS];
        if (src->type & accept)
        {
            if (src->pos >= reqPos)
            {
                lastPos = writePos;
                packets[writePos++] = *src;
            }
        }
    }

    buf.lock.unlock();
    lock.unlock();
    return lastPos - firstPos;
}

// ------------------------------------------------------------------
// ストリームポジションが spos か、それよりも新しいパケットが見付かれ
// ば pack に代入する。見付かった場合は true, そうでなければ false を
// 返す。
bool ChanPacketBuffer::findPacket(unsigned int spos, ChanPacket &pack)
{
    std::lock_guard<std::recursive_mutex> cs(lock);

    if (writePos == 0)
        return false;

    // unsigned int fpos = getStreamPos(firstPos);
    // if (spos < fpos)
    //     spos = fpos;

    // このループ、lastPos == UINT_MAX の時終了しないのでは？ …4G パ
    // ケットも送らないか。
    ChanPacket* candidate = nullptr;
    for (unsigned int i = firstPos; i <= lastPos; i++)
    {
        ChanPacket& p = packets[i%MAX_PACKETS];
        if (p.pos >= spos)
        {
            if (!candidate)
                candidate = &p;
            else if (p.pos < candidate->pos)
                candidate = &p;
        }
    }

    if (candidate)
    {
        pack = *candidate;
        return true;
    }else
    {
        return false;
    }
}

// ------------------------------------------------------------------
// バッファー内の一番新しいパケットのストリームポジションを返す。まだ
// パケットがない場合は 0 を返す。
unsigned int    ChanPacketBuffer::getLatestPos()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    if (!writePos)
        return 0;
    else
        return getStreamPos(lastPos);
}

// ------------------------------------------------------------------
unsigned int    ChanPacketBuffer::getLatestNonContinuationPos()
{
    if (writePos == 0)
        return 0;

    std::lock_guard<std::recursive_mutex> cs(lock);

    for (int64_t i = lastPos; i >= firstPos; i--)
    {
        ChanPacket &p = packets[i%MAX_PACKETS];
        if (!p.cont)
            return p.pos;
    }

    return 0;
}

// ------------------------------------------------------------------
unsigned int    ChanPacketBuffer::getOldestNonContinuationPos()
{
    if (writePos == 0)
        return 0;

    std::lock_guard<std::recursive_mutex> cs(lock);

    for (int64_t i = firstPos; i <= lastPos; i++)
    {
        ChanPacket &p = packets[i%MAX_PACKETS];
        if (!p.cont)
            return p.pos;
    }

    return 0;
}

// ------------------------------------------------------------------
// バッファー内の一番古いパケットのストリームポジションを返す。まだパ
// ケットが無い場合は 0 を返す。
unsigned int    ChanPacketBuffer::getOldestPos()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    if (!writePos)
        return 0;
    else
        return getStreamPos(firstPos);
}

// -----------------------------------
unsigned int    ChanPacketBuffer::findOldestPos(unsigned int spos)
{
    unsigned int min = getStreamPos(safePos);
    unsigned int max = getStreamPos(lastPos);

    if (min > spos)
        return min;

    if (max < spos)
        return max;

    return spos;
}

// -------------------------------------------------------------------
// パケットインデックス index のパケットのストリームポジションを返す。
unsigned int    ChanPacketBuffer::getStreamPos(unsigned int index)
{
    return packets[index%MAX_PACKETS].pos;
}

// -------------------------------------------------------------------
// パケットインデックス index のパケットの次のパケットのストリームポジ
// ションを計算する。
unsigned int    ChanPacketBuffer::getStreamPosEnd(unsigned int index)
{
    return packets[index%MAX_PACKETS].pos + packets[index%MAX_PACKETS].len;
}

// -----------------------------------
bool ChanPacketBuffer::writePacket(ChanPacket &pack, bool updateReadPos)
{
    if (pack.len)
    {
        if (willSkip()) // too far behind
            return false;

        lock.lock();

        pack.sync = writePos;
        packets[writePos%MAX_PACKETS] = pack;
        lastPos = writePos;
        writePos++;

        if (writePos >= MAX_PACKETS)
            firstPos = writePos-MAX_PACKETS;
        else
            firstPos = 0;

        if (writePos >= NUM_SAFEPACKETS)
            safePos = writePos - NUM_SAFEPACKETS;
        else
            safePos = 0;

        if (updateReadPos)
            readPos = writePos;

        lastWriteTime = sys->getTime();

        lock.unlock();
        return true;
    }

    return false;
}

// -----------------------------------
void    ChanPacketBuffer::readPacket(ChanPacket &pack)
{
    unsigned int tim = sys->getTime();

    lock.lock();

    if (readPos < firstPos)
    {
        lock.unlock();
        throw StreamException("Read too far behind");
    }

    while (readPos >= writePos)
    {
        lock.unlock();
        sys->sleepIdle();
        if ((sys->getTime() - tim) > 30)
        {
            throw TimeoutException();
        }
        lock.lock();
    }

    pack = packets[readPos%MAX_PACKETS];
    readPos++;
    lock.unlock();

    sys->sleepIdle();
}

// ------------------------------------------------------------
// バッファーがいっぱいなら true を返す。そうでなければ false。
bool    ChanPacketBuffer::willSkip()
{
    std::lock_guard<std::recursive_mutex> cs(lock);
    return ((writePos - readPos) >= MAX_PACKETS);
}
#endif // WITH_RUST_CORE
