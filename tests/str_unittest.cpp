#include <gtest/gtest.h>

#include "str.h"
using namespace str;

class strFixture : public ::testing::Test {
public:
};

TEST_F(strFixture, group_digits)
{
    ASSERT_STREQ("0", group_digits("0").c_str());
    ASSERT_STREQ("1", group_digits("1").c_str());
    ASSERT_STREQ("1.0", group_digits("1.0").c_str());
    ASSERT_STREQ("1.1", group_digits("1.1").c_str());
    ASSERT_STREQ("1.1234", group_digits("1.1234").c_str());
    ASSERT_STREQ("123,456", group_digits("123456").c_str());
    ASSERT_STREQ("123,456.789123", group_digits("123456.789123").c_str());

    ASSERT_STREQ("1",             group_digits("1").c_str());
    ASSERT_STREQ("11",            group_digits("11").c_str());
    ASSERT_STREQ("111",           group_digits("111").c_str());
    ASSERT_STREQ("1,111",         group_digits("1111").c_str());
    ASSERT_STREQ("11,111",        group_digits("11111").c_str());
    ASSERT_STREQ("111,111",       group_digits("111111").c_str());
    ASSERT_STREQ("1,111,111",     group_digits("1111111").c_str());
    ASSERT_STREQ("11,111,111",    group_digits("11111111").c_str());
    ASSERT_STREQ("111,111,111",   group_digits("111111111").c_str());
    ASSERT_STREQ("1,111,111,111", group_digits("1111111111").c_str());
}

TEST_F(strFixture, split)
{
    auto vec = split("a b c", " ");

    ASSERT_EQ(3, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b", vec[1].c_str());
    ASSERT_STREQ("c", vec[2].c_str());

    // 末尾空文字列要素の削除は行わない。
    vec = split("a,b,c,", ",");
    ASSERT_EQ(4, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b", vec[1].c_str());
    ASSERT_STREQ("c", vec[2].c_str());
    ASSERT_STREQ("", vec[3].c_str());

    vec = split("a", ",");
    ASSERT_EQ(1, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());

    vec = split("", ",");
    ASSERT_EQ(vec.size(), 1);
    ASSERT_EQ(vec[0], "");
}

TEST_F(strFixture, split_with_limit)
{
    auto vec = split("a b c", " ", 3);

    ASSERT_EQ(3, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b", vec[1].c_str());
    ASSERT_STREQ("c", vec[2].c_str());

    // リミットが実際に分割できる数よりも多い場合。
    vec = split("a b c", " ", 4);
    ASSERT_EQ(3, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b", vec[1].c_str());
    ASSERT_STREQ("c", vec[2].c_str());

    vec = split("a b c", " ", 2);
    ASSERT_EQ(2, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b c", vec[1].c_str());

    vec = split("a b c", " ", 1);
    ASSERT_EQ(1, vec.size());
    ASSERT_STREQ("a b c", vec[0].c_str());

    // 0以下のリミットは不正。
    ASSERT_THROW(split("a b c", " ", 0), std::domain_error);
    ASSERT_THROW(split("a b c", " ", -1), std::domain_error);

    // 末尾空文字列要素の削除は行わない。
    vec = split("a,b,c,", ",", 4);
    ASSERT_EQ(4, vec.size());
    ASSERT_STREQ("a", vec[0].c_str());
    ASSERT_STREQ("b", vec[1].c_str());
    ASSERT_STREQ("c", vec[2].c_str());
    ASSERT_STREQ("", vec[3].c_str());
}

TEST_F(strFixture, codepoint_to_utf8)
{
    ASSERT_STREQ(" ", codepoint_to_utf8(0x20).c_str());
    ASSERT_STREQ("あ", codepoint_to_utf8(12354).c_str());
    ASSERT_STREQ("\xf0\x9f\x92\xa9", codepoint_to_utf8(0x1f4a9).c_str()); // PILE OF POO
}

TEST_F(strFixture, format)
{
    EXPECT_STREQ("a", format("a").c_str());
    EXPECT_STREQ("a", format("%s", "a").c_str());
    EXPECT_STREQ("1", format("%d", 1).c_str());
    EXPECT_STREQ("12", format("%d%d", 1, 2).c_str());
}

TEST_F(strFixture, contains)
{
    ASSERT_TRUE(str::contains("abc", "bc"));
    ASSERT_FALSE(str::contains("abc", "d"));
    ASSERT_TRUE(str::contains("abc", ""));
    ASSERT_TRUE(str::contains("", ""));
    ASSERT_TRUE(str::contains("abc", "abc"));
    ASSERT_FALSE(str::contains("", "abc"));
}

TEST_F(strFixture, replace_prefix)
{
    ASSERT_STREQ("", str::replace_prefix("", "", "").c_str());
    ASSERT_STREQ("b", str::replace_prefix("", "", "b").c_str());
    ASSERT_STREQ("", str::replace_prefix("", "a", "b").c_str());
    ASSERT_STREQ("", str::replace_prefix("", "a", "").c_str());
    ASSERT_STREQ("xbc", str::replace_prefix("abc", "a", "x").c_str());
    ASSERT_STREQ("abc", str::replace_prefix("abc", "x", "x").c_str());
    ASSERT_STREQ("xabc", str::replace_prefix("abc", "", "x").c_str());
    ASSERT_STREQ("bc", str::replace_prefix("abc", "a", "").c_str());
}

TEST_F(strFixture, replace_suffix)
{
    ASSERT_STREQ("", str::replace_suffix("", "", "").c_str());
    ASSERT_STREQ("b", str::replace_suffix("", "", "b").c_str());
    ASSERT_STREQ("", str::replace_suffix("", "a", "b").c_str());
    ASSERT_STREQ("", str::replace_suffix("", "a", "").c_str());
    ASSERT_STREQ("abx", str::replace_suffix("abc", "c", "x").c_str());
    ASSERT_STREQ("abc", str::replace_suffix("abc", "x", "x").c_str());
    ASSERT_STREQ("abcx", str::replace_suffix("abc", "", "x").c_str());
    ASSERT_STREQ("ab", str::replace_suffix("abc", "c", "").c_str());
}

TEST_F(strFixture, capitalize)
{
    ASSERT_STREQ("", str::capitalize("").c_str());
    ASSERT_STREQ("A", str::capitalize("a").c_str());
    ASSERT_STREQ("@", str::capitalize("@").c_str());
    ASSERT_STREQ("Abc", str::capitalize("abc").c_str());
    ASSERT_STREQ("Abc", str::capitalize("ABC").c_str());
    ASSERT_STREQ("A@B", str::capitalize("a@b").c_str());
    ASSERT_STREQ("Content-Type", str::capitalize("CONTENT-TYPE").c_str());
    ASSERT_STREQ("Content-Type", str::capitalize("content-type").c_str());
    ASSERT_STREQ("Content-Type", str::capitalize("Content-Type").c_str());
    ASSERT_STREQ("あいうえお漢字αβγ", str::downcase("あいうえお漢字αβγ").c_str());
}

TEST_F(strFixture, upcase)
{
    ASSERT_STREQ("", str::upcase("").c_str());
    ASSERT_STREQ("A", str::upcase("a").c_str());
    ASSERT_STREQ("@", str::upcase("@").c_str());
    ASSERT_STREQ("ABC", str::upcase("abc").c_str());
    ASSERT_STREQ("ABC", str::upcase("ABC").c_str());
    ASSERT_STREQ("A@B", str::upcase("a@b").c_str());
    ASSERT_STREQ("CONTENT-TYPE", str::upcase("CONTENT-TYPE").c_str());
    ASSERT_STREQ("CONTENT-TYPE", str::upcase("content-type").c_str());
    ASSERT_STREQ("CONTENT-TYPE", str::upcase("Content-Type").c_str());
    ASSERT_STREQ("あいうえお漢字αβγ", str::downcase("あいうえお漢字αβγ").c_str());
}

TEST_F(strFixture, downcase)
{
    ASSERT_STREQ("", str::downcase("").c_str());
    ASSERT_STREQ("a", str::downcase("a").c_str());
    ASSERT_STREQ("@", str::downcase("@").c_str());
    ASSERT_STREQ("abc", str::downcase("abc").c_str());
    ASSERT_STREQ("abc", str::downcase("ABC").c_str());
    ASSERT_STREQ("a@b", str::downcase("a@b").c_str());
    ASSERT_STREQ("content-type", str::downcase("CONTENT-TYPE").c_str());
    ASSERT_STREQ("content-type", str::downcase("content-type").c_str());
    ASSERT_STREQ("content-type", str::downcase("Content-Type").c_str());
    ASSERT_STREQ("あいうえお漢字αβγ", str::downcase("あいうえお漢字αβγ").c_str());
}

TEST_F(strFixture, is_prefix_of)
{
    ASSERT_TRUE(str::is_prefix_of("", ""));
    ASSERT_TRUE(str::is_prefix_of("", "a"));
    ASSERT_TRUE(str::is_prefix_of("a", "a"));
    ASSERT_TRUE(str::is_prefix_of("a", "abc"));
    ASSERT_FALSE(str::is_prefix_of("b", ""));
    ASSERT_FALSE(str::is_prefix_of("b", "a"));
    ASSERT_FALSE(str::is_prefix_of("b", "abc"));
    ASSERT_TRUE(str::is_prefix_of("abc", "abc"));
    ASSERT_TRUE(str::is_prefix_of("あ", "あいうえお"));
}

TEST_F(strFixture, has_suffix)
{
    ASSERT_TRUE(str::has_suffix("", ""));
    ASSERT_TRUE(str::has_suffix("a", ""));
    ASSERT_TRUE(str::has_suffix("a", "a"));
    ASSERT_TRUE(str::has_suffix("abc", "c"));
    ASSERT_FALSE(str::has_suffix("", "b"));
    ASSERT_FALSE(str::has_suffix("a", "b"));
    ASSERT_FALSE(str::has_suffix("abc", "b"));
    ASSERT_TRUE(str::has_suffix("abc", "abc"));
    ASSERT_TRUE(str::has_suffix("あいうえお", "お"));
}

TEST_F(strFixture, join)
{
    ASSERT_STREQ("", str::join("", {}).c_str());
    ASSERT_STREQ("", str::join(",", {}).c_str());
    ASSERT_STREQ("a,b", str::join(",", {"a", "b"}).c_str());
    ASSERT_STREQ("ab", str::join("", {"a", "b"}).c_str());
    ASSERT_STREQ("ab", str::join("", str::split("a,b", ",")).c_str());
}

TEST_F(strFixture, extension_without_dot)
{
    ASSERT_STREQ("",    str::extension_without_dot("").c_str());
    ASSERT_STREQ("flv", str::extension_without_dot("test.flv").c_str());
    ASSERT_STREQ("FLV", str::extension_without_dot("TEST.FLV").c_str());
    ASSERT_STREQ("",    str::extension_without_dot("test.").c_str());
    ASSERT_STREQ("gz",  str::extension_without_dot("hoge.tar.gz").c_str());
}

TEST_F(strFixture, count)
{
    ASSERT_THROW(str::count("", ""), std::domain_error);
    ASSERT_THROW(str::count("abc", ""), std::domain_error);

    ASSERT_EQ(0, str::count("", "a"));
    ASSERT_EQ(1, str::count("abc", "a"));
    ASSERT_EQ(2, str::count("aba", "a"));

    ASSERT_EQ(1, str::count("aba", "ab"));
    ASSERT_EQ(2, str::count("abab", "ab"));
    ASSERT_EQ(3, str::count("  ab   ab   ab  ", "ab"));
}

TEST_F(strFixture, rstrip)
{
    ASSERT_EQ("", str::rstrip(""));
    ASSERT_EQ("", str::rstrip(" "));
    ASSERT_EQ("", str::rstrip("\t\r\n"));
    ASSERT_EQ("a", str::rstrip("a"));
    ASSERT_EQ("a", str::rstrip("a "));
    ASSERT_EQ("a", str::rstrip("a\t\r\n"));
    ASSERT_EQ(" a", str::rstrip(" a "));
    ASSERT_EQ("a", str::rstrip({ 'a', '\0', '\n' }));
}

TEST_F(strFixture, escapeshellarg_unix)
{
    ASSERT_EQ("\'\'", str::escapeshellarg_unix(""));
    ASSERT_EQ("\'abc\'\"\'\"\'def\'", str::escapeshellarg_unix("abc\'def"));
    ASSERT_EQ("\'ａｂｃ\'\"\'\"\'ｄｅｆ\'", str::escapeshellarg_unix("ａｂｃ\'ｄｅｆ"));
}

TEST_F(strFixture, STR)
{
    // no args
    ASSERT_EQ("", STR());

    // one arg
    ASSERT_EQ("", STR(""));
    ASSERT_EQ("", STR(std::string()));
    ASSERT_EQ("0", STR(0));

    // two args
    ASSERT_EQ("x = 1", STR("x = ", 1));
    ASSERT_EQ("x = 1", STR("x = ", (double) 1));
    ASSERT_EQ("x = 1.234", STR("x = ", 1.2340));
    ASSERT_EQ("x = A", STR("x = ", 'A'));
    ASSERT_EQ("x = 65", STR("x = ", 65));
    ASSERT_EQ("x = A", STR("x = ", (char) 65));

    // many args
    ASSERT_EQ("A1B2C3\n", STR("A", 1, "B", 2, "C", 3, '\n'));
}

TEST_F(strFixture, to_lines)
{
    ASSERT_EQ(std::vector<std::string>({}), str::to_lines(""));
    ASSERT_EQ(std::vector<std::string>({"\n"}), str::to_lines("\n"));
    ASSERT_EQ(std::vector<std::string>({"a\n", "b\n"}), str::to_lines("a\nb\n"));
    ASSERT_EQ(std::vector<std::string>({"a\n", "b"}), str::to_lines("a\nb"));
}

TEST_F(strFixture, indent_tab)
{
    ASSERT_EQ("", indent_tab(""));
    ASSERT_EQ("\t\n", indent_tab("\n"));
    ASSERT_EQ("\tfoo", indent_tab("foo"));
    ASSERT_EQ("\tfoo\n", indent_tab("foo\n"));
    ASSERT_EQ("\tfoo\n\tbar", indent_tab("foo\nbar"));
    ASSERT_EQ("\tfoo\n\tbar\n", indent_tab("foo\nbar\n"));
}

TEST_F(strFixture, inspect)
{
    ASSERT_EQ("\"\"", inspect(""));
    ASSERT_EQ("\"a\"", inspect("a"));
    ASSERT_EQ("\"\\n\"", inspect("\n"));
    ASSERT_EQ("\"\\x00\"", inspect(std::string({'\0'})));
    ASSERT_EQ("\"表\"", inspect("表")); // 表
    ASSERT_EQ("\"漢\"", inspect("漢")); // 漢
    ASSERT_EQ("\"\\x95\\\\\"", inspect("\x95\\")); // 表 in Shift_JIS
    ASSERT_EQ("\"\\x8a\\xbf\"", inspect("\x8A\xBF")); // 漢 in Shift_JIS
}

TEST_F(strFixture, validate_utf8)
{
    ASSERT_TRUE(validate_utf8(""));
    ASSERT_TRUE(validate_utf8(std::string({'\0'}))); // "\0"
    ASSERT_TRUE(validate_utf8(std::string({(char)0xef, (char)0xbb, (char)0xbf}))); // BOM
    ASSERT_TRUE(validate_utf8("a"));
    ASSERT_TRUE(validate_utf8("あ"));
    ASSERT_TRUE(validate_utf8("💩")); // PILE OF POO
    ASSERT_TRUE(validate_utf8("aあ💩"));

    ASSERT_FALSE(validate_utf8("\xff"));

    // Shift_JIS
    ASSERT_FALSE(validate_utf8("\x95\\"));   // 表
    ASSERT_FALSE(validate_utf8("\x8A\xBF")); // 漢
    ASSERT_FALSE(validate_utf8("\x83" "A")); // ア; KATAKANA LETTER A
    ASSERT_FALSE(validate_utf8("\xB1"));     // ｱ; HALFWIDTH KATAKANA LETTER A
}

TEST_F(strFixture, strip)
{
    ASSERT_EQ("", str::strip(""));
    ASSERT_EQ("", str::strip(" "));
    ASSERT_EQ("", str::strip("\t\r\n"));
    ASSERT_EQ("a", str::strip("a"));
    ASSERT_EQ("a", str::strip("a "));
    ASSERT_EQ("a", str::strip("a\t\r\n"));
    ASSERT_EQ("a", str::strip(" a"));
    ASSERT_EQ("a", str::strip("\t\r\na"));
    ASSERT_EQ("a", str::strip(" a "));
    ASSERT_EQ("a", str::strip({ '\0', '\n', 'a', '\0', '\n' }));
}

TEST_F(strFixture, truncate_utf8)
{
    ASSERT_EQ(str::truncate_utf8("", 1000), "");

    ASSERT_EQ(str::truncate_utf8("a", 0), "");
    ASSERT_EQ(str::truncate_utf8("a", 1), "a");
    ASSERT_EQ(str::truncate_utf8("a", 2), "a");

    ASSERT_EQ(str::truncate_utf8("あ", 0), "");
    ASSERT_EQ(str::truncate_utf8("あ", 1), "");
    ASSERT_EQ(str::truncate_utf8("あ", 2), "");
    ASSERT_EQ(str::truncate_utf8("あ", 3), "あ");
    ASSERT_EQ(str::truncate_utf8("あ", 4), "あ");

    // PILE OF POO
    ASSERT_EQ(str::truncate_utf8("💩", 0), "");
    ASSERT_EQ(str::truncate_utf8("💩", 1), "");
    ASSERT_EQ(str::truncate_utf8("💩", 2), "");
    ASSERT_EQ(str::truncate_utf8("💩", 3), "");
    ASSERT_EQ(str::truncate_utf8("💩", 4), "💩");
    ASSERT_EQ(str::truncate_utf8("💩", 5), "💩");

    ASSERT_THROW(str::truncate_utf8("\xff", 1), std::invalid_argument);

    // Shift_JIS
    ASSERT_THROW(str::truncate_utf8("\x95\\", 2), std::invalid_argument);   // 表
    ASSERT_THROW(str::truncate_utf8("\x8A\xBF", 2), std::invalid_argument); // 漢
    ASSERT_THROW(str::truncate_utf8("\x83" "A", 2), std::invalid_argument); // ア; KATAKANA LETTER A
    ASSERT_THROW(str::truncate_utf8("\xB1", 1), std::invalid_argument);     // ｱ; HALFWIDTH KATAKANA LETTER A

    ASSERT_EQ(truncate_utf8("Aあ", 0), "");
    ASSERT_EQ(truncate_utf8("Aあ", 1), "A");
    ASSERT_EQ(truncate_utf8("Aあ", 2), "A");
    ASSERT_EQ(truncate_utf8("Aあ", 3), "A");
    ASSERT_EQ(truncate_utf8("Aあ", 4), "Aあ");
    ASSERT_EQ(truncate_utf8("Aあ", 5), "Aあ");

    // Can handle NUL
    ASSERT_EQ(truncate_utf8({0}, 0), std::string({}));
    ASSERT_EQ(truncate_utf8({0}, 1), std::string({0}));
    ASSERT_EQ(truncate_utf8({0}, 2), std::string({0}));
    ASSERT_EQ(truncate_utf8({0}, 3), std::string({0}));
}

TEST_F(strFixture, json_inspect_invalidInput)
{
    ASSERT_THROW(str::json_inspect("\xff" "A"), std::invalid_argument); // invalid UTF-8
    ASSERT_NO_THROW(str::json_inspect("A"));
}

TEST_F(strFixture, json_inspect)
{
    ASSERT_EQ( str::json_inspect(""),
               "\"\"" );
    ASSERT_EQ( str::json_inspect("ABCあいう"),
               "\"ABCあいう\"" );
    ASSERT_EQ( str::json_inspect(" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"),
               "\" !\\\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\"" ); // ASCII printables
    ASSERT_EQ( str::json_inspect("\x01"),
               "\"\\u0001\"" ); // ^A
}

TEST_F(strFixture, shellwords)
{
    typedef std::vector<std::string> string_vector;

    ASSERT_EQ(str::shellwords(""), string_vector({}));

    ASSERT_EQ(str::shellwords(" \t "), string_vector({}));

    ASSERT_EQ(str::shellwords("\\ "), string_vector({" "}));

    ASSERT_EQ(str::shellwords("\"\""), string_vector({""}));

    ASSERT_EQ(str::shellwords("\"\"\"\"\"\""), string_vector({""}));

    ASSERT_EQ(str::shellwords("\"\" \"\" \"\""), string_vector({"", "", ""}));

    ASSERT_EQ(str::shellwords("a b"), string_vector({"a", "b"}));

    ASSERT_EQ(str::shellwords(" a  b "), string_vector({"a", "b"}));

    ASSERT_EQ(str::shellwords("\'a b\'"), string_vector({"a b"}));

    ASSERT_EQ(str::shellwords("a\\ b"), string_vector({"a b"}));

    ASSERT_ANY_THROW(str::shellwords("'a\\'b'"));

    ASSERT_EQ(str::shellwords("'a''b'"), string_vector({"ab"}));

    ASSERT_EQ(str::shellwords("a\'\'b"), string_vector({"ab"}));

    // an unescaped single quote in double quotes
    ASSERT_EQ(str::shellwords("\"\'\""), string_vector({"\'"}));

    // an unescaped double quote in single quotes.
    ASSERT_EQ(str::shellwords("\'\"\'"), string_vector({"\""}));

    ASSERT_EQ(str::shellwords("こんにちは"), string_vector({"こんにちは"}));

    ASSERT_EQ(str::shellwords("やま かわ"), string_vector({"やま", "かわ"}));

    ASSERT_EQ(str::shellwords("\"やま\" \"かわ\""), string_vector({"やま", "かわ"}));
    ASSERT_EQ(str::shellwords("\"やま かわ\""), string_vector({"やま かわ"}));

    ASSERT_EQ(str::shellwords("\\a"), string_vector({"a"}));
    ASSERT_EQ(str::shellwords("\"\\a\""), string_vector({"\\a"}));
    ASSERT_EQ(str::shellwords("\'\\a\'"), string_vector({"\\a"}));

    ASSERT_ANY_THROW(str::shellwords("\'\\\'\'"));
    ASSERT_EQ(str::shellwords("\'\\\"\'"), string_vector({"\\\""}));

    ASSERT_EQ(str::shellwords("\"\\\"\""), string_vector({"\""}));
    ASSERT_EQ(str::shellwords("\"\\\'\""), string_vector({"\\\'"}));

    ASSERT_EQ(str::shellwords("\"\\\\\""), string_vector({"\\"}));
    ASSERT_EQ(str::shellwords("\'\\\\\'"), string_vector({"\\\\"}));
    ASSERT_EQ(str::shellwords("\\\\"), string_vector({"\\"}));
}

TEST(strIsHttpUrl, basic)
{
    ASSERT_TRUE(str::is_http_url("http://example.com/"));
    ASSERT_TRUE(str::is_http_url("https://example.com/"));
    ASSERT_TRUE(str::is_http_url("HTTP://EXAMPLE.COM/"));
    ASSERT_FALSE(str::is_http_url(""));
    ASSERT_FALSE(str::is_http_url("http:/x"));
    ASSERT_FALSE(str::is_http_url("http:"));
    ASSERT_FALSE(str::is_http_url("javascript:alert(1)"));
    ASSERT_FALSE(str::is_http_url(" http://example.com/"));
    ASSERT_FALSE(str::is_http_url("ftp://example.com/"));
    ASSERT_FALSE(str::is_http_url("//example.com/"));
    ASSERT_FALSE(str::is_http_url("www.example.com"));
}
