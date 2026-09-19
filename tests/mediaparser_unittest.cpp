#include <gtest/gtest.h>

#include "channel.h"
#include "flv.h"
#include "mp4.h"
#include "sstream.h"
#include "amf0.h"

// 不正なメディアストリームによるバッファオーバーフローの回帰テスト。

namespace {

std::string be32(uint32_t v)
{
    return std::string({ (char)(v >> 24), (char)(v >> 16), (char)(v >> 8), (char)v });
}

// FLV タグ (11 バイトのヘッダー + データ + 4 バイトの PreviousTagSize)。
std::string flvTag(int type, const std::string& body)
{
    std::string t;
    t += (char) type;
    t += (char) (body.size() >> 16);
    t += (char) (body.size() >> 8);
    t += (char) body.size();
    t += std::string(7, '\0');      // timestamp, timestamp ext., stream id
    t += body;
    t += be32(body.size() + 11);
    return t;
}

} // namespace

TEST(MediaParserSecurity, flvHeadPacketTooLarge)
{
    // onMetaData のオブジェクトが大きく、ヘッダーパケット (16384 バイト) に
    // 収まらない。以前は headPack.data をはみ出して書き込んでいた。
    std::string meta;
    meta += amf0::Value("onMetaData").serialize();
    meta += amf0::Value::object({
            { "videodatarate", 100 },
            { "junk", std::string(20000, 'x') }
        }).serialize();

    std::string flv = std::string("FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00", 13);
    flv += flvTag(FLVTag::T_SCRIPT, meta);

    StringStream in(flv);
    auto ch = std::make_shared<Channel>();
    FLVStream stream;
    stream.readHeader(in, ch);
    ASSERT_THROW(stream.readPacket(in, ch), StreamException);
}

TEST(MediaParserSecurity, mp4BoxSizeTooSmall)
{
    // ボックスサイズ 2 (ヘッダーより小さい): 以前は new uint8_t[2] に
    // 4 バイト memcpy していた。
    StringStream in(be32(2) + "ftyp" + std::string(64, '\0'));
    auto ch = std::make_shared<Channel>();
    MP4Stream stream;
    ASSERT_THROW(stream.readHeader(in, ch), StreamException);
}

TEST(MediaParserSecurity, mp4BoxSizeZero)
{
    StringStream in(be32(0) + "ftyp" + std::string(64, '\0'));
    auto ch = std::make_shared<Channel>();
    MP4Stream stream;
    ASSERT_THROW(stream.readHeader(in, ch), StreamException);
}

TEST(MediaParserSecurity, mp4BoxSizeHuge)
{
    // 4GB 近いサイズを確保しようとしない。
    StringStream in(be32(0xfffffff0u) + "ftyp" + std::string(64, '\0'));
    auto ch = std::make_shared<Channel>();
    MP4Stream stream;
    ASSERT_THROW(stream.readHeader(in, ch), StreamException);
}
