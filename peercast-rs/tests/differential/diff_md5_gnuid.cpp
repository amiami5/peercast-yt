// md5::hexdigest と GnuID (toStr/fromStr/encode) の C++ 版と Rust 版の差分テスト。
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#include "gnuid.h"
#include "servent.h"
#include "md5.h"
#include "peercast_rs.h"

using std::string;

class Sys; extern Sys *sys; Sys *sys = nullptr; // このテストでは呼ばれないダミー

static const uint8_t* B(const string& s) { return reinterpret_cast<const uint8_t*>(s.data()); }
static string take(pcrs_buf b) { string s(reinterpret_cast<const char*>(b.ptr), b.len); pcrs_buf_free(b); return s; }

static long g_compared = 0, g_bad = 0;

int main(int argc, char** argv) {
    long n = 200000; uint64_t seed = 1;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--random") && i + 1 < argc) n = atol(argv[++i]);
        else if (!strcmp(argv[i], "--seed") && i + 1 < argc) seed = strtoull(argv[++i], nullptr, 10);
    }
    uint64_t x = seed * 0x9e3779b97f4a7c15ULL + 11;
    auto next = [&]() { x ^= x << 13; x ^= x >> 7; x ^= x << 17; return x; };

    // md5::hexdigest: 長さ0〜300バイトの乱数入力
    for (long i = 0; i < n; i++) {
        size_t len = next() % 300;
        string s;
        for (size_t j = 0; j < len; j++) s += (char)(next() & 0xff);
        string l = md5::hexdigest(s);
        string r = take(pcrs_md5_hexdigest(B(s), s.size()));
        g_compared++;
        if (l != r && g_bad++ < 5) printf("  [違い] md5::hexdigest len=%zu\n     C++ : %s\n     Rust: %s\n", len, l.c_str(), r.c_str());
    }
    printf("md5::hexdigest: 完了\n");

    // GnuID::toStr / fromStr
    for (long i = 0; i < n; i++) {
        GnuID g1;
        for (int j = 0; j < 16; j++) g1.id[j] = next() & 0xff;
        string l = g1.str();
        uint8_t rbuf[33] = {0};
        pcrs_gnuid_to_str(g1.id, rbuf);
        string r((char*)rbuf, 32);
        g_compared++;
        if (l != r && g_bad++ < 5) printf("  [違い] GnuID::toStr\n     C++ : %s\n     Rust: %s\n", l.c_str(), r.c_str());

        // fromStr: 正しい32文字、短い文字列、余分な文字が付いたものを試す
        for (string variant : {l, l.substr(0, next() % 33), l + "XYZ", string()}) {
            GnuID g2; g2.fromStr(variant.c_str());
            uint8_t rid[16];
            pcrs_gnuid_from_str(B(variant), variant.size(), rid);
            g_compared++;
            if (memcmp(g2.id, rid, 16) != 0 && g_bad++ < 5)
                printf("  [違い] GnuID::fromStr(\"%s\")\n", variant.c_str());
        }
    }
    printf("GnuID::toStr/fromStr: 完了\n");

    // GnuID::encode
    for (long i = 0; i < n; i++) {
        GnuID g1, g2;
        for (int j = 0; j < 16; j++) g1.id[j] = g2.id[j] = next() & 0xff;

        bool use_ip = next() % 2 == 0;
        Host h;
        uint8_t ipbuf[4] = {(uint8_t)next(), (uint8_t)next(), (uint8_t)next(), (uint8_t)next()};
        if (use_ip) memcpy(&h.ip, ipbuf, 4); // IP の先頭4バイトだけ使われる実装なので、これで十分

        string salt1, salt2;
        for (int j = 0, m = next() % 20; j < m; j++) salt1 += (char)(1 + next() % 255); // NUL を含めない (C文字列として渡すため)
        for (int j = 0, m = next() % 20; j < m; j++) salt2 += (char)(1 + next() % 255);
        uint8_t salt3 = next() & 0xff;

        g1.encode(use_ip ? &h : nullptr, salt1.c_str(), salt2.c_str(), salt3);
        pcrs_gnuid_encode(g2.id, use_ip ? reinterpret_cast<const uint8_t*>(&h.ip) : nullptr, use_ip,
                           B(salt1), salt1.size(), B(salt2), salt2.size(), salt3);
        g_compared++;
        if (memcmp(g1.id, g2.id, 16) != 0 && g_bad++ < 5)
            printf("  [違い] GnuID::encode (use_ip=%d salt1len=%zu salt2len=%zu salt3=%u)\n", use_ip, salt1.size(), salt2.size(), salt3);
    }
    printf("GnuID::encode: 完了\n");

    printf("比較件数: %ld  違い: %ld\n", g_compared, g_bad);
    return g_bad ? 1 : 0;
}
