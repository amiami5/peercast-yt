#include <algorithm>
#include <iterator>

#include "dechunker.h"
#include "defer.h"

using namespace std;

int Dechunker::hexValue(char c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    else if (c >= 'a' && c <= 'f')
        return c - 'a' + 0xa;
    else if (c >= 'A' && c <= 'F')
        return c - 'A' + 0xA;
    else
        return -1;
}

// 読み込めた分だけ返してもいいのかなぁ…？read の仕様がわからない。
// MemoryStream だと要求されただけのデータがなかったら 0 を返して
// いるが、FileStream だと読めた分だけ読んでいる。
//
// きっちり size バイト読めるまでブロックして欲しい。UClientSocket
// はそういう作りになってるな。上流はClientSocketが本番環境だから、
// それでいこう。
int  Dechunker::read(void *buf, int aSize)
{
    if (m_eof)
        throw StreamException("Closed on read");

    if (aSize < 0)
        throw GeneralException("Bad argument");

    size_t size = aSize;

    char *p = (char*) buf;

    while (true)
    {
        if (m_buffer.size() >= size)
        {
            while (size > 0)
            {
                *p++ = m_buffer.front();
                m_buffer.pop_front();
                size--;
            }
            int r = p - (char*)buf;
            updateTotals(r, 0);
            return r;
        } else {
            getNextChunk();
            continue;
        }
    }
}

#ifndef WITH_RUST_CORE
// WITH_RUST_CORE のときは rustcore.cpp (peercast-rs の src/dechunk.rs) を使う。
void Dechunker::getNextChunk()
{
    size_t size = 0;
    int digits = 0;

    // チャンクサイズを読み込む。
    while (true)
    {
        char c = m_stream.readChar();
        if (c == '\r')
        {
            c = m_stream.readChar();
            if (c != '\n')
                throw StreamException("Protocol error");
            break;
        }
        int v = hexValue(c);
        if (v < 0)
            throw StreamException("Protocol error");

        // 桁あふれを防ぐ。16 進 8 桁 (32 ビット) を超える大きさは不正。
        if (++digits > 8)
            throw StreamException("Chunk size too large");
        size *= 0x10;
        size += v;
    }

    // 巨大なメモリ確保を防ぐ。
    if (size > MAX_CHUNK_SIZE)
        throw StreamException("Chunk size too large");

    if (size == 0)
    {
        if (m_stream.readChar() != '\r' || m_stream.readChar() != '\n')
            throw StreamException("Protocol error");
        m_eof = true;
        throw StreamException("Closed on read");
    }

    char *buf = new char[size];
    Defer cleanup([=]() { delete[] buf; });
    size_t r;

    r = m_stream.read(buf, size);
    if (r != size)
    {
        throw StreamException("Premature end");
    }
    copy(buf, buf + size, back_inserter(m_buffer));

    if (m_stream.readChar() != '\r' || m_stream.readChar() != '\n')
        throw StreamException("Protocol error");
}
#endif // WITH_RUST_CORE
