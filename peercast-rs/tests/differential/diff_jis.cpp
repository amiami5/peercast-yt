// JISConverter::sjisToUnicode / eucToUnicode の C++ 版と Rust 版の差分テスト。
// 16ビット全て (65536通り x 2関数) を全数比較する。
#include <cstdio>
#include "jis.h"
#include "peercast_rs.h"

int main() {
    long bad = 0;
    for (unsigned v = 0; v <= 0xffff; v++) {
        unsigned short s16 = (unsigned short)v;
        unsigned short a = JISConverter::sjisToUnicode(s16);
        unsigned short b = pcrs_jis_sjis_to_unicode(s16);
        if (a != b) {
            if (bad++ < 10) printf("  [違い] sjisToUnicode(%#06x): C++=%#06x Rust=%#06x\n", v, a, b);
        }
        unsigned short a2 = JISConverter::eucToUnicode(s16);
        unsigned short b2 = pcrs_jis_euc_to_unicode(s16);
        if (a2 != b2) {
            if (bad++ < 10) printf("  [違い] eucToUnicode(%#06x): C++=%#06x Rust=%#06x\n", v, a2, b2);
        }
    }
    printf("65536通り x 2関数 = 131072件を全数比較。違い: %ld\n", bad);
    return bad ? 1 : 0;
}
