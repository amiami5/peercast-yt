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
| `str::hexdump`, `str::repeat`, `str::group_digits` | 同上 (段階1b) |
| `str::split` (2引数・3引数), `str::contains` | 同上 |
| `str::replace_prefix`, `str::replace_suffix` | 同上 |
| `str::upcase`, `str::downcase`, `str::capitalize` | 同上 |
| `str::has_prefix`, `str::has_suffix`, `str::join` | 同上 |
| `str::ascii_dump`, `str::extension_without_dot`, `str::count` | 同上 |
| `str::rstrip`, `str::strip`, `str::escapeshellarg_unix` | 同上 |
| `str::to_lines`, `str::indent_tab`, `str::shellwords` | 同上 |
| `md5::hexdigest` | `core/common/md5.cpp` (段階1c) |
| `GnuID::toStr`, `GnuID::fromStr`, `GnuID::encode` | `core/common/gnuid.cpp` (同上、純粋な部分のみ) |
| `JISConverter::sjisToUnicode`, `JISConverter::eucToUnicode` | `core/common/jis.cpp` (段階1d) |

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

## 段階1c で追加したもの

`md5::hexdigest` (RFC 1321 の MD5。`core/common/chanmgr.cpp` でリレー可否を決める認証
トークンの生成に使われている) と、`GnuID` の `toStr`/`fromStr`/`encode` を移植した。
`GnuID::generate`/`GnuID::random`/`GnuIDList` は `sys->rnd()` (乱数) や `sys->getTime()` に
依存する状態持ちのクラスなので、この段階では対象にしていない。

`GnuID::encode` は、IP アドレスを `Host*` から受け取るが、C++ 版の実装は
`(unsigned char*)&h->ip` の**先頭4バイトのメモリ表現**をそのまま使っており、実際の IPv4
アドレスのバイト列とは限らない (`IP` クラスの内部レイアウト次第)。Rust 版もこれをそのまま
踏襲しているだけで、意味のある IP アドレスの計算はしていない。

C++ 版との違いは見つからなかった (MD5、GnuID とも、既知の相違点なし)。

## 段階1d で追加したもの

`JISConverter::sjisToUnicode`/`eucToUnicode` (Shift_JIS・EUC-JPの1文字をUnicodeのコード
ポイントにする、JIS X 0208 の94×94変換表を使う) を移植した。呼び出し元は `_string.cpp` の
Shift_JIS/EUC-JPからUTF-8への変換のみ。変換表 (`src/jis_table.rs`) は元のCソースから
正規表現で機械的に抽出したもので、手作業では書き写していない。

C++版は `unsigned int` の引き算がラップアラウンドすることを前提にした書き方をしている
(範囲外になったら `> 93` の判定で弾かれる)。Rust版も同じ32ビットラップアラウンド演算で
実装し、`sjisToUnicode`/`eucToUnicode` それぞれ65536通り全数の差分テストで、C++版とビット単位
で一致することを確認した (相違なし)。

## C++ 版との違い (段階1b で追加で見つかったもの)

* **`str::split` (2引数・3引数の両方) が NUL バイトで壊れる**: `p = in.c_str()`、
  `sep = separator.c_str()` として `strstr`/`strlen` で処理しているため、C 文字列の終端規則に
  引きずられる。
  - **出力が NUL の手前で切り詰まる**: `haystack` に埋め込み NUL があると、最後の要素
    (`std::string(p)`、長さを指定しないコンストラクタ) が NUL のところで切れる。
  - **区切り文字列が NUL を含むと、その手前の部分だけで区切ってしまう**: 例えば区切りが
    `"a\0bc"` なら、実際には 1 バイトの `"a"` で区切ったのと同じ動きになる。
  - **区切り文字列が NUL から始まる (または空文字列) だと、無限ループしてメモリを使い果たす**:
    `strstr(p, "")` は常に `p` 自身にマッチするため `p` が全く前進せず、`res` に空文字列を
    延々と積み続けて `std::bad_alloc` で落ちる。**外部からの入力を区切り文字列に使う経路が
    あれば、サービス拒否 (DoS) に使える。** 差分テストでこれを実際に再現し、
    AddressSanitizer で `str::split` (str.cpp) が原因であることを特定した。
  Rust 版はバイト列をそのまま扱うので、この 3 つとも起こらない。区切りが空文字列の場合は
  (2引数版のみ) 入力全体を 1 要素として返す。`split_limit` は `limit` で必ず打ち切られるので、
  区切りが実質空でも無限ループにはならない (空要素を `limit - 1` 個積んでから残り全体を返す。
  これは Rust 版でも C++ 版と同じ動き)。

## 差分テスト

```sh
cd tests/differential
make
./diff                       # cgi/str の UTF-8・エスケープ系: 長さ0〜2バイト全網羅+乱数30万件+全コードポイント
./diff --exhaustive3         # さらに長さ3バイトを全網羅 (1,677万通り、数十秒)
./diff_strutil                # str.cpp のその他の関数: 乱数20万組 (既定)
./diff_strutil --random 300000  # 比較件数780万件相当まで増やして実行
./diff_md5_gnuid              # md5::hexdigest と GnuID: 乱数20万組 (既定)
./diff_jis                     # JISConverter: 65536通り全数 (sjis/euc 各1関数)
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
