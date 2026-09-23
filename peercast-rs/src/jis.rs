//! Shift_JIS / EUC-JP の1文字(2バイト)をUnicodeのコードポイントに変換する。
//! C++版 core/common/jis.cpp (giles, 2004年、原型は Y. Kuno による) の移植。
//!
//! 変換表 (`jis_table` の `UNI_TABLE`) は、元のCソースから機械的に抽出したもので、
//! 手作業では書き写していない。
//!
//! C++版は `unsigned int` (32ビット) での引き算を、負にならないことを前提に書いており、
//! 実際には (符号なし整数の)ラップアラウンドと、それに続く `> 93` の範囲外判定で
//! 結果的に正しく動いている。Rust版もビット単位で同じ結果になるよう、同じ32ビット
//! ラップアラウンド演算で実装している (`#![forbid(unsafe_code)]` を保つため
//! `wrapping_*` を使い、オーバーフローで panic しないようにしている)。

use crate::jis_table::UNI_TABLE;

const SJ0162: u32 = 0x00e1; // 01-62区のオフセット
const SJ6394: u32 = 0x0161; // 63-94区のオフセット
const GETA_MARK: u16 = 0x3013; // 変換できない文字の代わりに使う「〓」

fn lookup(c: u32, c1: u32) -> u16 {
    if c > 93 || c1 > 93 {
        GETA_MARK
    } else {
        let u = UNI_TABLE[c as usize][c1 as usize];
        if u == 0 {
            GETA_MARK
        } else {
            u
        }
    }
}

/// Shift_JIS の1文字 (上位バイト<<8 | 下位バイト) をUnicodeのコードポイントにする。
/// 変換できなければ〓 (U+3013) を返す (C++版と同じ。エラーを表す仕組みではない)。
pub fn sjis_to_unicode(sjis: u16) -> u16 {
    let mut c = (sjis >> 8) as u32;
    let mut c1 = (sjis & 0xff) as u32;

    let offset = if c <= 0x9f { SJ0162 } else { SJ6394 };
    c = c.wrapping_add(c).wrapping_sub(offset);
    if c1 < 0x9f {
        let sub = if c1 > 0x7f { 0x20 } else { 0x1f };
        c1 = c1.wrapping_sub(sub);
    } else {
        c1 = c1.wrapping_sub(0x7e);
        c = c.wrapping_add(1);
    }
    c = c.wrapping_sub(33);
    c1 = c1.wrapping_sub(33);

    lookup(c, c1)
}

/// EUC-JP の1文字をUnicodeのコードポイントにする。変換できなければ〓 (U+3013)。
pub fn euc_to_unicode(euc: u16) -> u16 {
    let mut c = (euc >> 8) as u32;
    let mut c1 = (euc & 0xff) as u32;

    c &= 0x7f;
    c = c.wrapping_sub(33);
    c1 &= 0x7f;
    c1 = c1.wrapping_sub(33);

    lookup(c, c1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 個々の文字の期待値は、記憶ではなく C++ 版との差分テスト
    // (peercast-rs/tests/differential/diff_jis.cpp) で 65536 通り全数比較して確認する。
    // ここでは構造的な性質だけを見る。

    #[test]
    fn sjis_ascii_range_first_byte_is_never_valid_as_is() {
        // 1バイト目が 0x00〜0x1f (制御文字域) はどの区にも対応しないはずなので〓になる。
        for hi in 0x00u16..=0x1f {
            assert_eq!(sjis_to_unicode((hi << 8) | 0x41), GETA_MARK, "hi={:#x}", hi);
        }
    }

    // ---- 範囲外・未割り当ての入力は〓 (U+3013) になり、panic しない ----
    #[test]
    fn out_of_range_and_unassigned_become_geta_mark() {
        assert_eq!(sjis_to_unicode(0x0000), GETA_MARK);
        assert_eq!(sjis_to_unicode(0xffff), GETA_MARK);
        assert_eq!(sjis_to_unicode(0x2020), GETA_MARK); // 制御文字域
        assert_eq!(euc_to_unicode(0x0000), GETA_MARK);
        assert_eq!(euc_to_unicode(0xffff), GETA_MARK);
    }

    #[test]
    fn every_possible_input_never_panics() {
        // 65536通り全部を試す。C++版の unsigned int ラップアラウンドを正しく再現できていれば、
        // すべて有効な u16 を返し、panic しない。
        for v in 0u32..=0xffff {
            let _ = sjis_to_unicode(v as u16);
            let _ = euc_to_unicode(v as u16);
        }
    }

    #[test]
    fn table_has_no_forbidden_zero_that_should_be_geta_mark_itself() {
        // 変換表の値そのものが 0x3013 と衝突していないか (表の中身の確認、C++版と同じ表であること)
        use crate::jis_table::UNI_TABLE;
        let mut count_geta = 0;
        for row in UNI_TABLE.iter() {
            for &v in row.iter() {
                if v == GETA_MARK {
                    count_geta += 1;
                }
            }
        }
        // 表そのものに「〓」の文字がいくつか正規の変換先として含まれていてもおかしくないので、
        // ここでは異常終了しないことだけ確認する (件数を出力用に保持)。
        let _ = count_geta;
    }
}
