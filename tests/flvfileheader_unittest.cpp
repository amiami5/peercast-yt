#include <gtest/gtest.h>

#include "flv.h"
#include "sstream.h"

#ifndef WITH_RUST_CORE
// FLVFileHeader は C++ 版の解析器にだけある。Rust 版 (WITH_RUST_CORE) のテストは
// peercast-rs/src/media/flv.rs にある。

class FLVFileHeaderFixture : public ::testing::Test {
public:
    FLVFileHeaderFixture()
      : data { 0x46, 0x4C, 0x56, 0x01, 0x04, 0x00, 0x00, 0x00, 0x09,
               0x00, 0x00, 0x00, 0x00 }
    {
    }

    void SetUp()
    {
    }

    void TearDown()
    {
    }

    ~FLVFileHeaderFixture()
    {
    }

    FLVFileHeader header;
    unsigned char data[13];
};

TEST_F(FLVFileHeaderFixture, initialState)
{
    ASSERT_EQ(0, header.size);
}

TEST_F(FLVFileHeaderFixture, read)
{
    StringStream mem(std::string(data, data+13));
    header.read(mem);

    ASSERT_EQ(13, header.size);
    ASSERT_EQ(1, header.version);
}



#endif // WITH_RUST_CORE
