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

## 段階2 で追加したもの (`String`)

`core/common/_string.cpp` の `String` クラス (256 バイト固定長の文字列) のうち、入力を解釈する
変換関数を `src/pcstring.rs` に移植した。

| 関数 | 内容 |
|---|---|
| `ASCII2ESC`, `ESC2ASCII` | `%XX` (`%%XX`) 形式への変換と、その逆 |
| `ASCII2META` | `;` → `:`、`%` → `%%` |
| `BASE642ASCII`, `base64WordToChars` | base64 の復号 (`stream.cpp` からも使われる) |
| `UNKNOWN2UNICODE` | 文字コード不明の文字列を 1 文字ずつ UTF-8 / Shift_JIS / EUC-JP / Latin-1 と推測して UTF-8 にする |
| `setFromString`, `setUnquote`, `setFromStopwatch` | コマンドライン引数・設定値の取り出し、経過時間の表示 |

`setFromTime` (`localtime_r` を使う) と、単なるバッファ操作 (`set`, `append`, `operator=`,
`sprintf` など) は C++ のまま残した。`String` はメンバー変数 `data` を直接読み書きする
コードが全体に散らばっているので、クラスそのものは段階 9 まで C++ に残る。

### C++ 版との違い

* **`ESC2ASCII` が、末尾の不完全な `%` で終端を越えて読む**: `"ab%"` や `"ab%4"` のように
  `%` の後ろに 2 文字ないと、C++ 版は終端の NUL とその次のバイトを 16 進数字として読み、
  さらに読み取り位置を 2 つ進めて終端を飛び越え、`data` の後ろのメモリを NUL に出会うまで
  読み続ける (範囲外読み出し)。Rust 版は入力の後ろを 0 とみなして 1 バイトを出力し、
  そこで止まる (C++ 版で後ろのメモリが 0 だった場合と同じ結果)。
* **`UNKNOWN2UNICODE` が、末尾の途中までしかない UTF-8 で終端を越えて読む**: 先頭バイトが
  示す長さ分を、終端の NUL があっても読み進めていた。Rust 版は入力の終わりで止まる。
* **`%` の後ろの 16 進数字でないバイトの扱いが CPU で違った**: C++ 版は `char` で計算するので、
  0x80 以上のバイトが来ると、x86 (符号付き `char`) と ARM の Linux (符号なし `char`) で
  結果が違っていた。Rust 版は CPU によらず符号付きとして計算する (x86 の C++ 版と同じ)。
  差分テストは `-fsigned-char` でコンパイルして、どの CPU でも同じ基準で比べる。
* `BASE642ASCII` は出力の長さを確かめずに `data` に書いていた。`convertTo` 経由では入力が
  255 バイト以下なので実害はないが、Rust 版は出力を `MAX_LEN - 1` バイトで切り詰める。

## 段階3a で追加したもの (HTTP の行の解析)

`src/http.rs`。`HTTP` クラスはソケットから 1 行読む部分 (`readLine`) を C++ に残し、
読んだ行の解釈だけを Rust にした。

| 関数 | 内容 |
|---|---|
| `HTTP::readResponse` | ステータス行からステータスコードを取り出す (行をコードの後ろで切る副作用も同じ) |
| `HTTP::nextHeader` | `名前: 値` の行を分け、名前を大文字にする |
| `HTTP::parseAuthorizationHeader` | `Basic` 認証のユーザー名とパスワード |
| `HTTP::isCrossOriginRequest`, `HTTP::isLoopbackHostHeader` | CSRF・DNS リバインディング対策の判定 |
| `cgi::parseHttpDate` | RFC 1123 / RFC 1036 / asctime 形式の日付 |

### C++ 版との違い

* **ステータスコードの桁あふれ**: C++ 版は `atoi` を使っており、`int` に収まらない数字の
  結果は未定義 (x86-64 の Linux では -1 になっていた。`long` の幅で CPU によっても違う)。
  Rust 版は `int` の最大値・最小値に丸める。
* **`parseHttpDate` が例外を投げる**: 日付の数字が `int` に収まらないと、C++ 版は
  `std::stoi` の `std::out_of_range` をそのまま投げていた。Rust 版は -1 (解釈できない) を返す。
  なお、この関数は今のところテストからしか呼ばれていない。
* C++ 版と同じにしてあるが、おかしな点: `parseHttpDate` の RFC 1036 形式
  (`Sunday, 06-Nov-94 ...`) は、曜日の名前から "day" を除いた部分を 3 文字の略号と比べるので、
  Sunday, Monday, Friday 以外は解釈できない。

### 見つけたが、この段階では直していないもの

* `HTTP::getResponse` の `if (contentLengthStr.empty())` は条件が逆になっている。
  Content-Length があるときに接続が閉じるまで読み、ないときに 0 バイトだけ読む。
  ソケットを読む側の処理なので、段階 9 で扱う。

## 段階3b で追加したもの (AMF0、chunked 転送)

### `Stream` から読む解析器の設計

解析器が C++ の `Stream` から読む必要がある場合は、**Rust 側が C++ のコールバックを呼んで
1 バイトずつ (または決まった長さを) 読む**形にした (`src/reader.rs`、C の型は `pcrs_reader`)。

* C++ 側のコールバック (`core/common/rustbridge.h` の `StreamReader`) が `Stream::readChar` などを
  そのまま呼ぶ。`MemoryStream` はデータが尽きても例外を投げずに 0 を返す、といった各ストリームの
  細かい挙動は変わらない。
* コールバックの中で起きた C++ の例外は、Rust を通り抜けられないので、その場で捕まえて保存し、
  Rust の関数から戻ったあとで投げ直す。例外の型とメッセージは C++ 版と同じになる。
* 読んだ結果の組み立て (AMF0 の `amf0::Value` など) も、Rust からの通知 (`pcrs_amf0_builder`) を
  受けて C++ 側で行う。信頼できない入力の解釈は Rust 側だけで行い、C++ 側は Rust が確かめた
  構造をなぞるだけになる。

メモリ上にそろったデータを解析するもの (段階 3a の HTTP の行など) は、これまでどおりバイト列を
受け取る。

### 置き換えたもの

| 関数 | 内容 |
|---|---|
| `amf0::Deserializer` の全メソッド | `src/amf0.rs`。FLV のメタデータ (onMetaData) の解析に使われる |
| `Dechunker::getNextChunk` | `src/dechunk.rs`。HTTP の chunked 転送の 1 チャンク |

`Dechunker::read` (読んだチャンクを溜めて返す部分) と `amf0::Value` (値の型そのもの) は C++ に残る。

### C++ 版との違い

* **`readDouble` がリトルエンディアンの CPU でしか正しく動かなかった**: C++ 版は 8 バイトを
  逆順にメモリへ書いて `double` とみなしていた。Rust 版はどの CPU でも正しい値になる
  (x86 と ARM の一般的な構成はリトルエンディアンなので、今の利用環境での結果は同じ)。
* **不明な型のエラーメッセージが CPU で違った**: `"unknown AMF value type N"` の N を `char` で
  表示しており、0x80 以上の型が x86 では負の数、ARM の Linux では正の数になっていた。
  Rust 版は常に符号付き (x86 の C++ 版と同じ)。

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
./diff_string                  # String: 長さ0〜2全網羅+長さ3〜4を絞ったバイトで全通り+乱数30万件 (約980万件)
./diff_http                    # HTTP の行の解析と parseHttpDate: 見本の変異+乱数 (約120万件)
./diff_amf0_dechunk            # AMF0 と Dechunker: 生成した値の変異・切り詰め (約100万件)
```

`diff_http` のように、C++ 版をクラスごと呼びたい差分テストは、Rust を使わずにビルドした
C++ のコア一式 (`cxxcore.a`、`make` が自動で作る) にリンクします。

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
