#include <gtest/gtest.h>

#include "mkv.h"

#ifndef WITH_RUST_CORE
// MKVStream::unpackUnsignedInt は C++ 版の解析器にだけある。Rust 版 (WITH_RUST_CORE) のテストは
// peercast-rs/src/media/mkv.rs にある。

class MKVStreamFixture : public ::testing::Test {
public:
    MKVStreamFixture()
    {
    }

    void SetUp()
    {
    }

    void TearDown()
    {
    }

    ~MKVStreamFixture()
    {
    }
};

TEST_F(MKVStreamFixture, unpackUnsignedInt)
{
    ASSERT_THROW(MKVStream::unpackUnsignedInt(""), std::runtime_error);
    ASSERT_EQ(1, MKVStream::unpackUnsignedInt("\x01"));
    ASSERT_EQ(258, MKVStream::unpackUnsignedInt("\x01\x02"));
}

#endif // WITH_RUST_CORE
