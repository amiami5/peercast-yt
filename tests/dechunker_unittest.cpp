#include <gtest/gtest.h>

#include "dechunker.h"
#include "sstream.h"

class DechunkerFixture : public ::testing::Test {
public:
};

TEST_F(DechunkerFixture, hexValue)
{
    ASSERT_EQ(0, Dechunker::hexValue('0'));
    ASSERT_EQ(9, Dechunker::hexValue('9'));
    ASSERT_EQ(0xa, Dechunker::hexValue('a'));
    ASSERT_EQ(0xf, Dechunker::hexValue('f'));
    ASSERT_EQ(0xa, Dechunker::hexValue('A'));
    ASSERT_EQ(0xf, Dechunker::hexValue('F'));
    ASSERT_EQ(-1, Dechunker::hexValue('g'));
    ASSERT_EQ(-1, Dechunker::hexValue('G'));
}

TEST_F(DechunkerFixture, readChar)
{
    StringStream mem;

    mem.writeString("4\r\n"
                    "Wiki\r\n"
                    "5\r\n"
                    "pedia\r\n"
                    "E\r\n"
                    " in\r\n"
                    "\r\n"
                    "chunks.\r\n"
                    "0\r\n"
                    "\r\n");
    mem.rewind();

    Dechunker dechunker(mem);

    ASSERT_EQ('W', dechunker.readChar());
    ASSERT_EQ('i', dechunker.readChar());
    ASSERT_EQ('k', dechunker.readChar());
    ASSERT_EQ('i', dechunker.readChar());
    ASSERT_EQ('p', dechunker.readChar());
    ASSERT_EQ('e', dechunker.readChar());
    ASSERT_EQ('d', dechunker.readChar());
    ASSERT_EQ('i', dechunker.readChar());
    ASSERT_EQ('a', dechunker.readChar());
    ASSERT_EQ(' ', dechunker.readChar());
    ASSERT_EQ('i', dechunker.readChar());
    ASSERT_EQ('n', dechunker.readChar());
    ASSERT_EQ('\r', dechunker.readChar());
    ASSERT_EQ('\n', dechunker.readChar());
    ASSERT_EQ('\r', dechunker.readChar());
    ASSERT_EQ('\n', dechunker.readChar());
    ASSERT_EQ('c', dechunker.readChar());
    ASSERT_EQ('h', dechunker.readChar());
    ASSERT_EQ('u', dechunker.readChar());
    ASSERT_EQ('n', dechunker.readChar());
    ASSERT_EQ('k', dechunker.readChar());
    ASSERT_EQ('s', dechunker.readChar());
    ASSERT_EQ('.', dechunker.readChar());
    ASSERT_THROW(dechunker.readChar(), StreamException);
}

TEST_F(DechunkerFixture, read)
{
    StringStream mem;

    mem.writeString("4\r\n"
                    "Wiki\r\n"
                    "5\r\n"
                    "pedia\r\n"
                    "E\r\n"
                    " in\r\n"
                    "\r\n"
                    "chunks.\r\n"
                    "0\r\n"
                    "\r\n");
    mem.rewind();

    Dechunker dechunker(mem);

    char buf[24] = "";
    ASSERT_EQ(23, dechunker.read(buf, 23));
    ASSERT_STREQ("Wikipedia in\r\n\r\nchunks.", buf);
}

TEST_F(DechunkerFixture, chunkSizeOverflowIsRejected)
{
    // 16 進 16 桁 (size_t が 64 ビット) 。以前は桁あふれして巨大な
    // new char[size] を行っていた。
    StringStream in("FFFFFFFFFFFFFFFF\r\nabc\r\n0\r\n\r\n");
    Dechunker dechunker(in);
    char buf[3];
    ASSERT_THROW(dechunker.read(buf, 3), StreamException);
}

TEST_F(DechunkerFixture, hugeChunkSizeIsRejected)
{
    // 8 桁に収まるが上限 (16 MiB) を超える。
    StringStream in("7FFFFFFF\r\nabc\r\n0\r\n\r\n");
    Dechunker dechunker(in);
    char buf[3];
    ASSERT_THROW(dechunker.read(buf, 3), StreamException);
}

TEST_F(DechunkerFixture, normalChunkStillWorks)
{
    StringStream in("3\r\nabc\r\n0\r\n\r\n");
    Dechunker dechunker(in);
    char buf[3];
    ASSERT_EQ(3, dechunker.read(buf, 3));
    ASSERT_EQ(0, memcmp(buf, "abc", 3));
}
