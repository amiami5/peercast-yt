#include <gtest/gtest.h>

#include "chandir.h"

class ChannelEntryFixture : public ::testing::Test {
public:
};

TEST_F(ChannelEntryFixture, constructor)
{
    ASSERT_THROW(ChannelEntry({}, ""), std::runtime_error);
}

TEST_F(ChannelEntryFixture, textToChannelEntries)
{
    std::vector<std::string> errors;
    auto vec = ChannelEntry::textToChannelEntries("予定地<>97968780D09CC97BB98D4A2BF221EDE7<>127.0.0.1:7144<>http://www.example.com/<>プログラミング<>peercastをいじる - &lt;Free&gt;<>-1<>-1<>428<>FLV<><><><><>%E4%BA%88%E5%AE%9A%E5%9C%B0<>1:14<>click<><>1\n", "", errors);

    ASSERT_EQ(1, vec.size());

    auto& entry = vec[0];

    ASSERT_STREQ("予定地", entry.name.c_str());
    ASSERT_STREQ("97968780D09CC97BB98D4A2BF221EDE7", ((std::string) entry.id).c_str());
    ASSERT_EQ(428, entry.bitrate);
    ASSERT_STREQ("FLV", entry.contentTypeStr.c_str());
    ASSERT_STREQ("peercastをいじる - &lt;Free&gt;", entry.desc.c_str());
    ASSERT_STREQ("プログラミング", entry.genre.c_str());
    ASSERT_STREQ("http://www.example.com/", entry.url.c_str());
    ASSERT_STREQ("127.0.0.1:7144", entry.tip.c_str());
    ASSERT_STREQ("1:14", entry.uptime.c_str());
    ASSERT_STREQ("%E4%BA%88%E5%AE%9A%E5%9C%B0", entry.encodedName.c_str());
    ASSERT_EQ(-1, entry.numDirects);
    ASSERT_EQ(-1, entry.numRelays);
}

TEST_F(ChannelEntryFixture, nonHttpUrlsAreDropped)
{
    // YP のフィードの url と trackContact は UI でリンクになる。
    std::vector<std::string> errors;
    auto vec = ChannelEntry::textToChannelEntries(
        "ch<>97968780D09CC97BB98D4A2BF221EDE7<>127.0.0.1:7144<>javascript:alert(1)<>g<>d<>-1<>-1<>428<>FLV<><><><>data:text/html,x<>ch<>1:14<>click<><>1\n", "", errors);
    ASSERT_EQ(1, vec.size());
    ASSERT_STREQ("", vec[0].url.c_str());
    ASSERT_STREQ("", vec[0].trackContact.c_str());
}

TEST_F(ChannelEntryFixture, parseErrorsAndUrls)
{
    std::vector<std::string> errors;
    auto vec = ChannelEntry::textToChannelEntries(
        "bad line\n"
        "ch<> 7+a-1-fzz9 \t0x0Xx1  ++--1q12345<>t<>http://u/<>g<>d<> 12<>+3<>-4<>FLV<><><><><>%20<>1:14<>click<><>5\r\n"
        "\n", "http://yp/sub/index.txt", errors);
    ASSERT_EQ(1, vec.size());
    ASSERT_EQ(2, errors.size());
    EXPECT_EQ("Parse error at line 1.", errors[0]);
    EXPECT_EQ("Parse error at line 3.", errors[1]);

    auto& e = vec[0];
    // GnuID は 2 文字ずつ strtoul で読む (空白と符号も読む)
    EXPECT_EQ("070AFFF1000900000001" "0000FF002345", e.id.str());
    EXPECT_EQ(12, e.numDirects);
    EXPECT_EQ(3, e.numRelays);
    EXPECT_EQ(-4, e.bitrate);
    EXPECT_EQ(5, e.direct);
    EXPECT_EQ("http://yp/sub/chat.php?cn=%20", e.chatUrl());
    EXPECT_EQ("http://yp/sub/getgmt.php?cn=%20", e.statsUrl());

    e.feedUrl = "noslash";
    EXPECT_EQ("", e.chatUrl());
    e.feedUrl = "http://yp/index.txt";
    e.encodedName = "";
    EXPECT_EQ("", e.statsUrl());
}
