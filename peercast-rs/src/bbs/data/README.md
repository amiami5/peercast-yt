# 掲示板ビューワーの変換表

掲示板ビューワー (`/cgi-bin/board.cgi`、`thread.cgi`、`post.cgi`) は、もとは Python で書かれていた
(`ui/cgi-bin/bbs_reader.py` など)。Rust 版が同じ結果を返すように、Python の標準ライブラリの変換を
Python 3.14 で書き出したものをここに置いている。どれも一度書き出しただけで、作り直す必要はない。

| ファイル | 中身 |
|---|---|
| `cp932_dec.bin` | `bytes.decode('cp932')` の表。(u16 LE バイト列, u16 LE 文字) の並び。1 バイトの文字はバイト列 < 0x100、2 バイトは先頭のバイトだけでは読めないもの |
| `eucjp_dec.bin` | `bytes.decode('euc_jp')` の表。(u32 LE バイト列 (1〜3 バイトを上位から詰めたもの), u16 LE 文字) |
| `sjis_enc.bin` | `str.encode('shift_jis')` の表 (BMP の文字ごと)。(u16 LE 文字, u16 LE バイト列) |
| `eucjp_enc.bin` | `str.encode('euc_jp')` の表。(u16 LE 文字, u32 LE バイト列) |
| `html5_entities.txt` | `html.entities.html5`。名前 TAB 文字 (16 進、空白区切り) |
| `html_invalid_charrefs.txt` | `html._invalid_charrefs`。数値 (16 進) TAB 置き換える文字 |
| `html_invalid_codepoints.txt` | `html._invalid_codepoints` (16 進)。数値参照でこれらは消える |

表はどれもキーの順に並んでいる (二分探索するため)。BMP の外の文字はどちらの符号化でも書けない
(`&#...;` になる) ことも確かめてある。

読めないバイトの扱い (U+FFFD 1 つにするバイト数) は表では決まらないので、CPython の
`Modules/cjkcodecs/_codecs_jp.c` と同じにしてある (`../codec.rs`): 読めないバイトは 1 バイトずつ、
途中で切れた多バイト文字は残り全部で 1 つ。

## 確かめ方

書き出したときに、乱数で作った入力 (CP932 と EUC-JP の読み各 3000、Shift_JIS と EUC-JP の書き
各 3000、`html.unescape` 3000、`str.splitlines` 1000、`int()` 16) を Python で変換した結果を
JSON (`[{"op": "dec", "codec": "cp932", "in": "<16 進>", "strict": ..., "replace": ...}, ...]`) に
書き出し、Rust 版と全部一致することを確かめた:

```sh
PEERCAST_PY_CASES=/path/to/python_cases.json cargo test --release -p peercast-rs --lib bbs::tests::python_cases_if_available
```

`../fixtures.rs` は、もとの Python 版の CGI スクリプトを、ネットワークの取得を差し替えて動かした
入力と出力 (板・スレッド・書き込み、2ch 形式としたらば、引数の誤りと取得の失敗)。
