# peercast-rs

PeerCast YT の C++ コア (`core/common`) を段階的に置き換えていく Rust 実装です。
Rust コード自体は普通のライブラリですが、`crate-type = ["staticlib"]` で C++ から
呼べる静的ライブラリとしてもビルドされます。全体の計画は
[`../docs/rust-migration.md`](../docs/rust-migration.md) を参照してください。

## 構成

* `src/utf8.rs`, `src/inspect.rs`, `src/cgi.rs`, `src/url.rs`, `src/entities.rs` — 本体。
  `#![deny(unsafe_code)]` (ワークスペース全体、`ffi` モジュールを除く)。
* `src/ffi.rs` — C ABI (`extern "C"`)。`unsafe` を使うのはここだけ。
* `include/peercast_rs.h` — 上記に対応する C ヘッダー。
* `../core/common/rustcore.cpp` — C++ 側の橋渡し。`cgi::escape` などの関数を、
  `WITH_RUST_CORE` が定義されているときだけ Rust 呼び出しに差し替える。
* `tests/differential/` — C++ 版と Rust 版に同じ入力を与えて出力を比べるテスト (下記)。

## この段階で置き換えた関数

| 関数 | 元の場所 |
|---|---|
| `cgi::escape`, `cgi::unescape` | `core/common/cgi.cpp` |
| `cgi::escape_html`, `cgi::unescape_html` | 同上 |
| `cgi::escape_javascript` | 同上 |
| `cgi::isSafeLocalPath` | 同上 |
| `str::validate_utf8`, `str::truncate_utf8`, `str::valid_utf8` | `core/common/str.cpp` |
| `str::codepoint_to_utf8`, `str::inspect`, `str::json_inspect` | 同上 |
| `str::is_http_url` | 同上 |

## C++ 版との違い

比較の過程で、C++ 版に次のバグが見つかりました。Rust 版では直っています。

* **UTF-8 の検査が甘い**: `validate_utf8` (および `valid_utf8`, `inspect`, `json_inspect`,
  `truncate_utf8` の判定) が、過長表現 (`C0 80` など)、サロゲート単体 (`ED A0 80`)、
  U+10FFFF を超える符号 (`F5`〜`F7` 始まりや `F4 90` 以降) を「正しい UTF-8」としていた。
  Rust 版は Rust 標準ライブラリの厳密な検査 (RFC 3629 準拠) を使う。
* **`codepoint_to_utf8` が U+0080〜U+07FF を間違って符号化する**: 先頭バイトを
  `0xB0 | (cp >> 6)` にしていた (正しくは `0xC0 | (cp >> 6)`)。この範囲の文字は全て
  不正な UTF-8 になっていた。`unescape_html` で `&copy;` `&#169;` のような Latin-1
  範囲の実体参照・数値参照を使うと、壊れた HTML が出力される。サロゲート
  (U+D800〜U+DFFF) は符号化できない値として扱い、`unescape_html` は
  代わりに U+FFFD (置換文字) を出力する。
* **`unescape_html` の数値参照が 32 ビットで切り詰まる**: `&#4294967361;` (2^32 + 65) が
  `&#65;` と同じ 'A' になっていた。Rust 版はオーバーフローする数値参照を全て
  無効なコードポイント (→ U+FFFD) として扱う。
* **`unescape_html` が無効なコードポイントで例外を投げる**: `&#x110000;` のような、
  存在しないコードポイントを指す参照を渡すと、C++ 版はその場で例外を投げて
  (呼び出し元が捕まえなければ) 落ちる。Rust 版は U+FFFD を出力して処理を続ける。
* **`unescape` が不正な `%XX` で未初期化の値を使う**: `%` の直後が 16 進数 2 桁でない場合
  (`%zz`, `%4` など)、C++ 版は `sscanf` の戻り値を確認せず、初期化していない変数の値を
  出力に混ぜていた。Rust 版は `%` をそのまま出力する。

## 差分テスト

```sh
cd tests/differential
make
./diff                       # 長さ 0〜2 バイトを全網羅 + 乱数 30 万件 + 全コードポイント
./diff --exhaustive3         # さらに長さ 3 バイトを全網羅 (1,677 万通り、数十秒)
```

C++ 版の関数をそのままコンパイルしたもの (`WITH_RUST_CORE` を定義しない `cgi.cpp` / `str.cpp`) と、
`libpeercast_rs.a` に、1 バイトずつ変えた入力を大量に与えて比較します。上に挙げた既知の違いは
理由ごとに数えて表示し、それ以外の食い違いを「説明のつかない違い」として報告します
(0 件であることを確認済み)。

## 単体テスト

```sh
cargo test --release
```

C++ 版の既存の gtest (`tests/cgi_unittest.cpp`, `tests/str_unittest.cpp`) にある期待値は、
`src/cgi.rs` と `src/utf8.rs` / `src/inspect.rs` のテストに移してあります。

## ビルドへの組み込み

`ui/linux/Makefile` から使われます。ルートの `README.md` の「Linuxでのビルド」を参照してください。
CMake と `ui/mingui` は、この段階ではまだ対象にしていません (常に C++ 版を使います)。
