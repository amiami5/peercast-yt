#include <gtest/gtest.h>
#include "sstream.h"
#include "atom.h"

class AtomStreamFixture : public ::testing::Test {
public:
    AtomStreamFixture()
    {
        
    }

    void SetUp()
    {
    }

    void TearDown()
    {
    }

    ~AtomStreamFixture()
    {
    }
};

TEST_F(AtomStreamFixture, writeInt)
{
    StringStream s;
    AtomStream a(s);

    a.writeInt("ip", 127<<24|1);
    ASSERT_EQ(std::string({ 'i','p',0,0, 4,0,0,0, 1,0,0,127 }),
              s.str());
}

TEST_F(AtomStreamFixture, writeAddress4)
{
    StringStream s;
    AtomStream a(s);
    IP ip(127<<24|1);
    
    a.writeAddress("ip", ip);
    ASSERT_EQ(std::string({ 'i','p',0,0, 4,0,0,0, 1,0,0,127 }),
              s.str());
}

TEST_F(AtomStreamFixture, writeAddress16)
{
    StringStream s;
    AtomStream a(s);
    in6_addr addr = { { 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1 } };
    IP ip(addr);
    
    a.writeAddress("ip", ip);
    ASSERT_EQ(std::string({ 'i','p',0,0, 16,0,0,0, 1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0 }),
              s.str());
}

TEST_F(AtomStreamFixture, readInt)
{
    StringStream s(std::string({ 'i','p',0,0, 4,0,0,0, 1,0,0,127 }));
    AtomStream a(s);

    int nchildren, size;
    ID4 id = a.read(nchildren, size);
    ASSERT_EQ(0, nchildren);
    ASSERT_EQ(4, size);
    ASSERT_EQ(127<<24|1, a.readInt());
}

TEST_F(AtomStreamFixture, readAddress4)
{
    StringStream s(std::string({ 'i','p',0,0, 4,0,0,0, 1,0,0,127 }));
    AtomStream a(s);

    int nchildren, size;
    ID4 id = a.read(nchildren, size);
    ASSERT_EQ(0, nchildren);
    ASSERT_EQ(4, size);
    ASSERT_EQ(IP(127<<24|1), a.readAddress());
}

TEST_F(AtomStreamFixture, readAddress16)
{
    StringStream s(std::string({ 'i','p',0,0, 16,0,0,0, 1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0 }));
    AtomStream a(s);
    in6_addr addr = { { 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1 } };
    IP ip(addr);
    
    int nchildren, size;
    ID4 id = a.read(nchildren, size);
    ASSERT_EQ(0, nchildren);
    ASSERT_EQ(16, size);
    ASSERT_EQ(ip, a.readAddress());
}

TEST_F(AtomStreamFixture, skipDeeplyNestedThrows)
{
    // 深くネストした atom (親の子がまた親、を繰り返す) は、スタック
    // 枯渇を防ぐため、上限を超えたら例外を投げて中断する。
    StringStream s;
    AtomStream writer(s);

    const int depth = 1000;
    for (int i = 0; i < depth; i++)
        writer.writeParent(ID4("test"), 1);
    // 一番内側は子を持たない (末端) atom。
    writer.writeInt(ID4("leaf"), 42);

    s.rewind();
    AtomStream reader(s);
    int c, d;
    reader.read(c, d);
    ASSERT_THROW(reader.skip(c, d), StreamException);
}

TEST_F(AtomStreamFixture, skipShallowNestingWorks)
{
    // 通常の深さのネストは、これまでどおりスキップできる。
    StringStream s;
    AtomStream writer(s);

    writer.writeParent(ID4("test"), 1);
    writer.writeParent(ID4("test"), 1);
    writer.writeInt(ID4("leaf"), 42);

    s.rewind();
    AtomStream reader(s);
    int c, d;
    reader.read(c, d);
    ASSERT_NO_THROW(reader.skip(c, d));
    ASSERT_TRUE(reader.eof());
}
