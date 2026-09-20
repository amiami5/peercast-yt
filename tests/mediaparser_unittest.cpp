#include <gtest/gtest.h>

#include "channel.h"
#include "flv.h"
#include "mp4.h"
#include "mkv.h"
#include "mp3.h"
#include "nsv.h"
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

TEST(MediaParserSecurity, negativeLengthSkipAndReadThrow)
{
    // Stream::skip / read に負の長さを渡すと、以前は StringStream で
    // スタックバッファ (4096 バイト) をはみ出してクラッシュした。
    StringStream s(std::string(100000, 'A'));
    char buf[3];
    s.read(buf, 3);
    ASSERT_THROW(s.skip((int) 0xFFFFFFF0u), StreamException);
    ASSERT_THROW(s.read(buf, -1), StreamException);

    std::string data(16, 'B');
    MemoryStream m(&data[0], data.size());
    ASSERT_THROW(m.read(buf, -1), StreamException);
    ASSERT_THROW(m.write(buf, -1), StreamException);
}

TEST(MediaParserSecurity, mkvHugeElementSizeRejected)
{
    // Tracks 要素の TrackEntry の中に、サイズが 0xFFFFFFF0 (int にすると負) の
    // 未知の子要素がある MKV ヘッダー。以前は readTracks() の skip() が
    // スタックバッファをはみ出してクラッシュした。
    std::string trackEntry;
    trackEntry += std::string("\x86", 1);                                  // CodecID
    trackEntry += std::string("\x01\x00\x00\x00\xff\xff\xff\xf0", 8);   // サイズ 0xFFFFFFF0
    trackEntry += std::string(6, 'A');

    std::string tracksBody;
    tracksBody += std::string("\xae", 1);                                  // TrackEntry
    tracksBody += std::string(1, (char)(0x80 | trackEntry.size()));
    tracksBody += trackEntry;
    // 後ろに 4096 バイトより大きい Void 要素を置く。負の skip() が
    // 残りのデータをスタックの 4096 バイトのバッファにコピーしてしまう。
    const int padding = 8000;
    tracksBody += std::string("\xec", 1);                                  // Void
    tracksBody += std::string(1, (char)(0x40 | (padding >> 8)));
    tracksBody += std::string(1, (char)(padding & 0xff));
    tracksBody += std::string(padding, 'A');

    std::string tracks = std::string("\x16\x54\xae\x6b", 4) +
                         std::string(1, (char)(0x40 | (tracksBody.size() >> 8))) +
                         std::string(1, (char)(tracksBody.size() & 0xff)) + tracksBody;

    // Segment: サイズは 8 バイトの VInt で表す
    std::string segment = std::string("\x18\x53\x80\x67", 4) +
                          std::string("\x01\x00\x00\x00\x00\x00", 6) +
                          std::string(1, (char)(tracks.size() >> 8)) +
                          std::string(1, (char)(tracks.size() & 0xff)) +
                          tracks;

    std::string ebml("\x1a\x45\xdf\xa3\x80", 5);

    StringStream in(ebml + segment);
    auto ch = std::make_shared<Channel>();
    MKVStream stream;
    ASSERT_THROW(stream.readHeader(in, ch), StreamException);
}

// ICY メタデータ (ストリームに埋め込まれた 16 バイト単位のブロック) が NUL 終端
// されていなくても、バッファの外を読まない。以前は char buf[1024] を未初期化・
// 未終端のまま C 文字列として解析していた。
// 通常のビルドでは検出できない。AddressSanitizer を付けたビルドで失敗する。
TEST(MediaParserSecurity, mp3UnterminatedIcyMetadata)
{
    auto ch = std::make_shared<Channel>();
    ch->icyMetaInterval = 16;

    std::string data(16, 'x');
    data += (char) 64;                  // メタデータ長 64 * 16 = 1024 バイト
    data += std::string(1024, 'A');     // NUL も '=' も含まない
    data += std::string(4096, 'B');

    StringStream in(data);
    MP3Stream stream;
    ASSERT_NO_THROW(stream.readPacket(in, ch));
}

TEST(MediaParserSecurity, nsvUnterminatedIcyMetadata)
{
    auto ch = std::make_shared<Channel>();
    ch->icyMetaInterval = 16;

    std::string data(16, 'x');
    data += (char) 64;
    data += std::string(1024, 'A');
    data += std::string(4096, 'B');

    StringStream in(data);
    NSVStream stream;
    ASSERT_NO_THROW(stream.readPacket(in, ch));
}
