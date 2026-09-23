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

C++ 版との違いは見つからなかった (MD5、GnuID とも、既知の相違点なし)。ただし段階 7a で、
`fromStr` が空白と符号を読む `strtoul` の動きを再現していなかったことがわかり、直した
(段階 7a の節を参照)。

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

## 段階3c で追加したもの (XML)

`src/xml.rs`。`XML::read` (字句解析)、`XML::Node::setAttributes` (属性の解析)、
`XML::Node::getBinaryContent` (16 進の内容) を置き換えた。ノードの木は、Rust からの通知を受けて
C++ 側 (`rustbridge.h` の `XmlReader`) が組み立てる。XML はプレイリスト (ASX) と YP の
アップロード帯域測定 (uptest) の応答の読み取りに使われている。

PCP の atom (`atom.h` の `AtomStream`) は、atom の頭を読むだけの薄い読み出し口で、中身の解釈は
`pcp.cpp` 側にある。段階 6 で PCP の解析をまとめて Rust にするときに一緒に扱う。

### C++ 版との違い

* **`XML::read` の共有バッファ**: C++ 版はバッファを `static` で持っていて、別々のスレッドで
  同時に XML を読むと中身が混ざった。また、タグや内容の長さが上限 (100 KiB) ちょうどのとき、
  終端の NUL をバッファの 1 バイト外に書いていた。Rust 版は呼び出しごとにバッファを持つ。
* **`getBinaryContent` の範囲外読み出し**: 16 進数字が奇数個だと、C++ 版は最後の 1 文字の次に
  終端の NUL を読み、さらにその先のメモリを読み進めていた。Rust 版は足りない 1 文字を NUL と
  みなして止まる。内容がない (NULL) ノードでは、C++ 版は落ちていたが、Rust 版は空として扱う。
* `getBinaryContent` の 16 進数字でないバイトの値は、C++ 版では `char` の符号で CPU により
  違っていた。Rust 版は CPU によらず符号付きとして計算する。
* C++ 版と同じにしてあるが、おかしな点: コメントは中身に `>` があるとそこで終わる。閉じタグの
  名前は確かめない。属性値の閉じる `"` がないと、値の最後の 1 文字が落ちる。`findAttr` は属性名の
  前方一致で探す。

## 段階3d で追加したもの (URL)

`src/url.rs` に、`LUrlParser::clParseURL::ParseURL` と `GetPort` (`URI` クラスの中身) と、
`URLSource::getSourceProtocol` (配信元の URL の先頭から入力元の種類を決める) を移した。
`URLSource` のそれ以外 (配信元に接続して読み続ける処理) は段階 7〜9 で扱う。

### C++ 版との違い

* **ポート番号の桁あふれ**: C++ 版は `atoi` を使っており、2^32 を超える数が一周して小さな値に
  なっていた (例えば `http://h:4294967376/` のポートが 80 になる。`long` の幅で CPU によっても違う)。
  Rust 版は範囲を超える数をすべて無効なポートとする (URI の既定のポートが使われる)。

### 見つけたが、この段階では直していないもの

* `URLSource::streamURL` は、プレイリストの中の URL を自分自身の再帰呼び出しで読むので、
  プレイリストを指すプレイリストが続くと再帰が深くなる。段階 7〜9 で扱う。

## 段階4 で追加したもの (メディアコンテナ)

`src/media/` に、配信の中身を解析する `ChannelStream` の派生クラス (`FLVStream`, `MKVStream`,
`OGGStream`, `MP3Stream`, `MP4Stream`) を移した。FLV、Matroska/WebM、Ogg (Vorbis/Theora)、MP3
(ICY メタデータを含む)、fragmented MP4 を扱う。NSV と Windows Media 系 (ASF, MMS, WMHTTP) は
移植せずにサポートをやめた (ルートの `README.md` の「本家との違い」を参照)。

### 設計

* 解析器は、入力の `Stream` とチャンネル (`Channel`) を、`Host` トレイト (C の型は
  `pcrs_media_host`) のメソッドだけで触る。C++ 側の実装 (`core/common/rustmedia.h` の
  `rustbridge::MediaHost`) が、C++ 版の解析器と同じ順序で `Channel` のメンバー (`streamPos`,
  `headPack`, `info` など) を読み書きする。入力の読み出しは段階 3b と同じコールバック (`pcrs_reader`)。
* 解析器の状態 (FLV の溜めているタグ、OGG のヘッダーなど) は Rust 側が持つ (`pcrs_media_new` /
  `pcrs_media_free`)。`FLVStream` などは `rustbridge::MediaStream` を継承するだけのクラスになった。
* ヘッダーの中身 (クラスの定義) が変わるので、gtest も `WITH_RUST_CORE` でコンパイルする
  (`ui/linux/tests/Makefile` が `libpeercast.a` を見て自動で決める)。C++ 版の内部のクラス
  (`FLVTag`, `FLVFileHeader`, `MKVStream::unpackUnsignedInt`) の gtest は C++ 版のビルドでだけ動き、
  同じ内容のテストは `src/media/*.rs` にある。
* FLV の `readPacket` は、パケットを送らなかったとき続きが読めれば自分を再帰呼び出ししていた。
  Rust 版は同じことをループで行う。
* MKV は `Stream::read(int)` (4096 バイトずつ `read(void*, int)` を呼ぶ) を Rust 側で行うので、
  大きな要素 (最大 256 MiB) でも、実際に届いた分しかメモリを確保しない (C++ 版と同じ)。

### C++ 版との違い

* **初期化していないメモリ**: 読み出しが足りなかったときのバッファの残り (FLV のタグ、MP4 の
  ボックス、OGG のページ、MP3 のパケットなど) のように、C++ 版が初期化していないメモリを読んで
  いた箇所は、すべて 0 として扱う。
* **MKV の短い SimpleBlock**: トラック番号とタイムコードしかない SimpleBlock で、C++ 版は
  フラグのバイトとしてデータの外を読んでいた。Rust 版は 0 (キーフレームでない) とみなす。
* **OGG Vorbis のコメントの長さ**: ちょうど 8192 バイトのコメントで、C++ 版は終端の NUL を
  スタックのバッファの外に 1 バイト書いていた。Rust 版は書かない。
* **OGG Vorbis のコメント数**: データが尽きたあとも、ヘッダーに書かれたコメント数 (最大約 21 億)
  の回数だけ空回りしてログを出していた (事実上止まる)。空回りの間は何も変わらないので、Rust 版は
  残りが 4 バイト (長さのフィールド) に満たなくなったらループを抜ける。
* **FLV のメタデータのビットレート**: videodatarate と audiodatarate の和が 2^31 以上のとき、
  C++ 版は `double` を `int` に変換する未定義動作で、CPU によって値が違った (x86 では -2^31)。
  Rust 版は `int` の最大値にする。
* **バイト順**: OGG のグラニュール位置とシリアル番号、Vorbis のヘッダーの整数は、C++ 版は CPU の
  バイト順で読んでいた (ビッグエンディアンの CPU では誤り)。Rust 版は仕様どおりリトルエンディアン
  で読む (x86 と ARM の一般的な構成では同じ値)。
* **符号付き整数の桁あふれ** (MKV の待ち時間、Theora の時刻の計算): C++ 版で未定義動作だった
  ところは、2 の補数で一周する値にした (一般的な CPU での C++ 版と同じ値)。
* **MKV のヘッダーの溜め方**: C++ 版は Cluster までの要素を全部メモリに溜めてから大きさを
  確かめていた。Rust 版は 16 KiB を超えた分は溜めずに長さだけ数える (エラーになることと、その
  メッセージは同じ)。

## 段階5a で追加したもの (テンプレートエンジン)

`src/template/` に、HTML テンプレートエンジン (`core/common/template.cpp` の `Template`) の
式の字句解析・構文解析・評価と、ディレクティブ (`{$式}` `{\式}` `{!式}` `{@if}` `{@elsif}`
`{@else}` `{@foreach}` `{@let}` `{@loop}` `{@fragment}`) の読み出しを移した。

### 設計

* 変数の値を持つスコープ (`servMgr` などの状態を返す `RootObjectScope`、`HTTPRequestScope`、
  `GenericScope`) と、`=~` `!~` の正規表現 (`Regexp`、中身は `std::regex`) は C++ のまま。
  Rust は `pcrs_template_host` のコールバックで変数を引き、`{@let}` などのスコープを置く。
  正規表現を Rust に移すには ECMAScript の正規表現エンジンが要るので、後の段階で扱う。
* `Template` クラスの形 (メンバーとメソッド) は変わらない。WITH_RUST_CORE のときは、各メソッドが
  `pcrs_template_call` を呼ぶ (`core/common/rusttemplate.h`)。値は、型 1 バイトと中身の
  簡単な形式 (`src/template/value.rs`) で受け渡す。
* 出力は Rust 側で溜め、呼び出しの終わりに (エラーのときも、それまでの分を) 書く。

### C++ 版との違い

* **評価の順序**: C++ 版は `==` や関数の引数などの評価順序を決めていなかった (C++11 では
  未規定)。Rust 版は、x86-64 の GCC でビルドした C++ 版に合わせた (二項演算子 `==` `!=` `=~` は
  右辺から、`replacePrefix` などの引数は右から、`{@let x = 式}` などは右辺を評価してから
  変数を作る)。
* **`nth` の範囲外**: C++ 版は添字を確かめずに読んでいた (未定義動作で、落ちるか不定の値)。
  Rust 版は `GeneralException` ("nth: index out of range") にする。
* **入れ子の深さ**: C++ 版は式やディレクティブの入れ子、関数の再帰に上限がなく、深すぎると
  スタックを使い果たして落ちた。Rust 版は 200 段で `GeneralException`
  ("Template: nesting too deep") にする。
* **例外のあとのスコープ**: `{@let}` などの途中で例外が起きると、C++ 版は破棄された (スタック上の)
  スコープへのポインタを `Template` に残していた。Rust 版は、置いたスコープを取り除く。
* **`{@loop}` の回数**: 変数の値が `int` に収まらない数 (か NaN) のとき、C++ 版は未定義動作で、
  CPU によって違った (x86 では -2^31 で 1 回も回らず、ARM では `int` の最大値で約 21 億回回る)。
  Rust 版は、どの CPU でも x86 と同じく -2^31 にする。

## 段階5b で追加したもの (Accept-Language、コンソールの引数)

* `src/public.rs`: `PublicController::acceptableLanguages` (Accept-Language ヘッダーの解釈) と
  `formatUptime`。q 値は glibc の `atof` と同じく読む (`src/strtod.rs`。16 進数、`inf`、
  `nan(...)`、空白や途中の文字の扱いも同じ)。
* `src/commands.rs`: 管理画面のコンソールのコマンドの引数の解釈 (`commands.cpp` の
  `parse_options`)。

段階5 の残り (`html.cpp` の HTML の出力、`commands.cpp` の各コマンドの本体、`public.cpp` の
HTTP の処理) は、入力を解釈せず、C++ のチャンネルやサーバーの管理 (`servMgr`、`chanMgr`) を
呼ぶだけなので、それらを移す段階 7〜9 で扱う。テンプレートのスコープと正規表現も同じ。

### C++ 版との違い

* **Accept-Language の並べ替え**: C++ 版は `std::sort` で q 値の大きい順に並べていた。
  タグが 16 個以下なら挿入ソートになるので、Rust 版は同じ挿入ソートを使う (結果は同じ)。
  17 個以上のとき、C++ 版は q 値が同じタグの順序が実装次第で、q 値が NaN (`q=nan`) だと
  比較が一貫せず、配列の外を読むことがあった (ネットワークから届くヘッダーで起きる)。
  Rust 版は個数によらず同じ挿入ソートで、同じ q 値は書かれた順になる。
* NUL を含むヘッダーや引数は、段階1 の `str::split` の違い (C++ 版は NUL で切れる) のとおり。

## 段階6a で追加したもの (PCP の受け取ったパケットの処理)

`src/pcp/` に、ほかのノードから受け取った PCP のパケットの処理 (`core/common/pcp.cpp` の
`PCPStream::procAtom` 以下と、`chaninfo.cpp` の `ChanInfo::readInfoAtoms` / `readTrackAtoms`) を
移した。`chan` (チャンネルの情報とストリームのパケット)、`host` (ヒット)、`root`、`bcst` (中継)、
`push`、`helo`、`mesg`、`ok`、`quit`、`atom` を扱う。

### 設計

* atom の読み書き (`AtomStream`) は、C++ 版と同じく 16KB のバッファ (`ChanPacket::data`) の上で
  行う (`src/pcp/atom.rs`)。バッファの終わりを越える読み出しは 0 を返して位置を進めない、4096
  バイトずつ読み飛ばす、といった `MemoryStream` と `Stream` の振る舞いも同じにした。`helo` への
  返事 (`oleh`) を受け取ったパケットのバッファに書き込む (送られず、後ろの atom を上書きする)
  ことや、中継する `bcst` のパケットを別のバッファに組み立てることも同じ。
* チャンネル、ヒットリスト、サーバーの状態 (`chanMgr`、`servMgr`、`Channel`) は C++ に残り、
  `Host` トレイト (C の型は `pcrs_pcp_host`) で読み書きする。C++ 側の実装
  (`core/common/rustpcp.h` の `rustbridge::PcpHost`) は、C++ 版の同じ箇所をそのまま写したもの。
  `PCPStream::readPacket` のソケットの読み書きは C++ のまま (段階 9)。
* `readInfoAtoms` などの C++ 版のメソッドはなくなるので、それを呼んでいた gtest
  (`pcpstream_unittest.cpp` の 2 件) は C++ 版のビルドでだけ動く。同じ内容のテストは
  `src/pcp/tests.rs` にある。`chaninfo_unittest.cpp` の URL のテスト 3 件は、Rust 版のビルドでは
  PCP のパケットとして受け取り、ヒットリストに入る値で確かめる。

### C++ 版との違い

* **入れ子の深さ**: C++ 版は `atom` の子や `bcst` の中の atom の入れ子に上限がなく、`bcst` の
  入れ子ごとに 16KB のバッファをスタックに取るので、数百段の入れ子でスタックを使い果たして
  落ちた (ネットワークから届くパケットで起きる)。Rust 版は 64 段を超えると `StreamException`
  ("PCP: atom nesting too deep") にする。
* **NUL で終わらない文字列の atom**: C++ 版は、文字列を読んだ `String` のまだ書いていない部分
  (初期化されていないスタックのメモリ) を文字列の続きとして使い、チャンネルの情報に入れて
  ほかのノードへ中継することもあった。Rust 版はそこを 0 とみなし、文字列は atom の中身で終わる
  (ChanInfo の newInfo は、0 で埋めた記憶領域の上に作る)。
* **バッファの終わりでの空回り**: 子の数を大きく偽った atom があると、C++ 版はバッファの終わりに
  着いたあとも子の数だけ (最大約 21 億回) ループし、ID 0 の atom を読み飛ばし続けた (`host`
  などでは毎回ログを出す)。空回りの間は何も変わらないので、Rust 版はそこでループを終える
  (ログは 1 回)。
* **`char` の符号**: `ttl`、`hops`、`grp` などの 1 バイトの値は、C++ 版では CPU によって符号が
  違った (x86 は符号付き、ARM の Linux は符号なし。例えば `ttl` が 0 の中継の扱いが変わる)。
  Rust 版は CPU によらず x86 と同じ符号付き。

## 段階6b で追加したもの (PCP のハンドシェイクで受け取る atom)

`src/pcp/handshake.rs` に、PCP のハンドシェイクで相手から受け取る atom の読み取りを移した。
`Servent::handshakeIncomingPCP` の `helo`、`handshakeOutgoingPCP` の `oleh`、`pingHost` の `oleh`、
`PCPStream::readVersion`。

* ソケットなどの C++ の `Stream` から、段階 3b と同じ `pcrs_reader` で直接読む。このため atom の
  層 (`src/pcp/atom.rs`) は、下の `Stream` を `AtomIo` トレイトで抽象化した (6a の `MemStream` と、
  `Stream` を読む `StreamIo`)。
* 返事を書くこと (`oleh`、`quit`) と、読んだ値を使った処理 (servMgr の IP アドレスやファイア
  ウォールの状態の更新、`pingHost` など) は C++ のまま。servent.cpp は、`core/common/rustpcp.h` の
  `readIncomingHelo` などを呼ぶ。C++ 版と同じく、エラーのときもそれまでに読んだ値 (`rid`、
  `agent` など) を呼んだ側の変数に入れてから例外を投げる。
* `GeneralException` はコピーすると `msg` が古い `msgbuf` を指したままになる (C++ 版の例外クラスの
  性質)。`StreamException` は `std::make_exception_ptr` などでコピーせず、その場で作って投げる。

### C++ 版との違い

* エージェント名 (`agnt`) を読む `char arg[64]` の、まだ書いていない部分は C++ 版では初期化されて
  いなかった。Rust 版は 0 とみなす (6a の文字列と同じ)。

## 段階6c で追加したもの (チャンネルのパケットのバッファ)

`src/chanpacket.rs` に、チャンネルのパケットのバッファ (`core/common/chanpacket.cpp` の
`ChanPacketBuffer`。最近の 64 個のパケットを輪の形に持つ) の処理を移した。パケットの書き込みと
読み出し、ストリーム位置からのパケットの探し方、一番古い・新しい位置、統計。

* `ChanPacketBuffer` のメンバー (`writePos` や `lastWriteTime` など) は、チャンネルや配信の処理が
  直接読み書きしているので、パケットと位置は C++ のクラスのメンバーのまま置き、そこを指すもの
  (`pcrs_cpb`) を Rust に渡す。`ChanPacket` の並びは `static_assert` で確かめる。
* ロックと、`readPacket` の待ち合わせ (`sleepIdle` と 30 秒のタイムアウト) は C++ に残る。
  段階 7 でチャンネルを Rust に移すとき、同じ Rust のコードを Rust 側の記憶領域で使う。
* `ChanPacket` の小さなメソッド (`init`、`writeRaw`、`operator=`) は C++ のまま。

### C++ 版との違い

* `lastPos` が `UINT_MAX` (4G 個目のパケット) のとき、C++ 版は `findPacket` などのループが
  終わらなかった。Rust 版は 1 周で終える。
* 使われていない `copyFrom` は、C++ 版は書き先の番号を 64 で割らずに `packets[writePos++]` に
  書いていた (配列の外に書く)。Rust 版は 64 で割った位置に書く。

## 段階7a で追加したもの (イエローページのチャンネル一覧)

`src/chandir.rs` に、イエローページの index.txt の解釈 (`core/common/chandir.cpp` の
`ChannelEntry::textToChannelEntries` と `ChannelEntry` のコンストラクタ) と、`chatUrl` / `statsUrl`、
`ChannelDirectory::getState` が使う `directoryUrlOf` と `formatTime` を移した。

* 1 行を 1 チャンネルとして Rust が解釈し、コールバック (`pcrs_chandir_parse` の `on_entry` と
  `on_error`) で C++ に返す。C++ はそれで `ChannelEntry` を作る。
* 一覧の入れ物 (`m_channels`、`m_feeds`) と、一覧を取りに行く処理 (フィードごとのスレッドと
  HTTP)、`writeChannelVariable` などは C++ に残る。ほかのコード (`servmgr`、`jrpc`、`channel`) が
  直接触っているので、それらと一緒に段階 9 で移す。

### C++ 版との違い

* 数の欄 (直接・リレーの数、ビットレート、`direct`) が `int` に収まらないとき、C++ 版の `atoi` は
  x86-64 の Linux では一周した値になる (C 標準では未定義)。Rust 版は段階 3a と同じく範囲の端に丸める。
* NUL を含む行は、段階 1 の `str::split` の違い (C++ 版は NUL で切れる) のとおり。

### 段階 1c の `GnuID::fromStr` の修正

`GnuID::fromStr` は 2 文字ずつ `strtoul(buf, nullptr, 16)` で読む。`strtoul` は先頭の空白と符号を
読むので、C++ 版は `" 7"` と `"+7"` を 7、`"-1"` を 0xFF と読む。段階 1c の Rust 版はこれを 0 と
していた (段階 1c の差分テストは 16 進数の文字しか試していなかった)。index.txt のチャンネル ID の
欄の差分テストで見つけたので、C++ 版と同じにした。`diff_md5_gnuid` に 2 文字の組み合わせを全部
試す比較を足した。

## 段階7b で追加したもの (チャンネルの情報と、中継しているホストの一覧)

`src/chaninfo.rs` に `ChanInfo` と `TrackInfo` (`core/common/chaninfo.cpp`)、`src/chanhit.rs` に
`ChanHit` と `ChanHitList` (`core/common/chanhit.cpp`) の、状態を持たない部分を移した。

* atom を書く側: `writeInfoAtoms`、`writeTrackAtoms`、`ChanHit::writeAtoms`。Rust がバイト列に
  組み立て (`src/pcp/write.rs` の `AtomBuf`)、C++ はそれを一度に `Stream` に書く。
* `ChanInfo`: 種類・MIME タイプ・プロトコルの表、`getTypeStringLong`、`getPlayListExt`、検索の
  一致 (`match`、`matchNameID`)、`update` と `TrackInfo::update` で写す欄の判断。欄を写すこと
  (`String` の代入は文字コードの種類も写す) は C++ 側。
* `ChanHit`: 版の文字列、色、`canGiv`。
* `ChanHitList`: 数え上げ (`numHits` など)、次につなぐホストの選び方 (`pickHits`)、
  `clearDeadHits` / `deadHit` / `delHit` / `addHit` でどのホストをどうするかの判断。連結リスト
  (`hit`) はほかのコード (チャンネル、配信、JSON-RPC) が直接たどっているので C++ に残し、並び
  どおりの配列にして渡す。リストの付け替えは C++ 側。
* `createChannelXML` などの XML と `getState` は、中身を組み立てるだけなので段階 8 で扱う。

### C++ 版との違い

* atom をまとめて 1 回で書くので、書き先に途中までしか書けなかったとき、書けていた量が違う
  ことがある。書き先はソケットか 16KB のパケットで、書く量 (文字列は 1 つ 256 バイトまで) は
  パケットに収まるので、書き終えたときの中身は同じ。

### C++ 版と同じにしたもの (直していない)

* `getTypeFromMIME` は OGM と MP4 にならない (OGM の行は OGG と同じ MIME タイプを見ていて、MP4 の
  行がない)。
* `pickHits` は、除外するセッション ID を指定しない (0 の) とき、セッション ID が 0 のホストを
  選ばない。
* `ChanHit::init` は `direct` を true にしたあと 0 にしている (結果は false)。

## 段階7c で追加したもの (リレーツリーと帯域測定)

* `src/hostgraph.rs`: `HostGraph` (`core/common/hostgraph.cpp`) のコンストラクターで、どのホストを
  どのホストの下に置くか (トラッカー、WAN の中継、LAN の中継の順に親を探す) を決める。C++ は
  自分とリストのホストを並べて渡し、返った順 (`std::map` を回す順) と親の位置から `m_hit`、
  `m_roots`、`m_children` を作る。JSON を組み立てる `toRelayTree` / `getRelayTree` は C++ 側。
* `src/uptest.rs`: 帯域測定 (`core/common/uptest.cpp`) の通信しない部分。yp4g.xml の読み取り
  (`UptestEndpoint::readInfo`。`XML` の木の組み立てと `findNode` / `findAttr` も Rust で行う)、
  `UptestInfo::postURL`、`isReady`、状態の文字列、`addURL` で加えてよいかの判断。URL が正しいか
  (`URI`、LUrlParser) と、ダウンロード、POST、ロックは C++ 側。
* `src/reader.rs` の `SliceReader` (バイト列から読む `Reader`) をテスト以外でも使えるようにした。

### C++ 版との違い

* `readInfo` でノードか属性が見つからなかったときの例外の文言 "non-null assertion failed on
  line N in file F" の N は、C++ 版では見つからなかった項目の行、Rust 版では橋渡しの行 (どの項目
  でも同じ)。
* `isReady` は、状態が `kUntried` のときも `sys->getTime()` を呼ぶ (返り値は使わない)。

### C++ 版と同じにしたもの (直していない)

* `findAttr` は名前の先頭が一致すれば見つかったことにする (`port` で `port_open` も見つかる)。
  大文字小文字は問わない。
* 最上位の要素が 2 つ以上ある文書では、最後のものだけが根になる (C++ 版は前の根を解放しない)。

## 段階7d で追加したもの (チャンネルと ChanMgr の、スレッドやソケットに触らない部分)

`src/channel.rs` に、`Channel` (`core/common/channel.cpp`) と `ChanMgr` (`chanmgr.cpp`) のうち
次のものを移した。

* `processMp3Metadata`: ICY のメタデータ (`StreamTitle='...';StreamUrl='...';`) の解釈。値を
  `String` に入れること (`setUnquote`、`convertTo`) と `updateInfo` は C++ 側。
* atom を書く側: `writeTrackerUpdateAtom` と、`updateInfo` で中継先へ送る atom。自分の
  `ChanHit` (`initLocal`) を作るのと、パケットを送るのは C++ 側。
* `renderHexDump`、`getBufferString` の文字列。
* `checkReadDelay` の待ち時間、`ChanMgr::authToken`、`ChanMgr::closeOldestIdle` で止める
  チャンネルの選び方。
* `ChanInfo` と `ChanHit` を Rust に渡す形にする関数を `core/common/rustchan.h` にまとめた
  (`chaninfo.cpp`、`chanhit.cpp`、`hostgraph.cpp`、`channel.cpp` で使う)。

チャンネルのスレッド (`Channel::stream`、`PeercastSource::stream`、`readStream`) と、チャンネルと
ヒットリストの連結リストの管理 (`ChanMgr` の `find*`、`clearDeadHits` など) は、ソケットと
スレッドと一緒に段階 9 で扱う。`createXML` と `getState` は段階 8 で扱う。

### C++ 版との違い

* `processMp3Metadata` は、受け取った文字列を書き換えない (C++ 版は区切りの `=` と `;` を NUL に
  書き換えていた)。呼び出し元 (`mp3.cpp`) はそのあと文字列を使わない。
* `getBufferString` で、受信の速さが 0 のときの秒数は、C++ 版では x86 で `-nan`、ARM で `nan`
  (0.0 / 0.0 の NaN の符号が CPU で違う)。Rust 版は CPU によらず `-nan`。
* `getBufferString` のパケットの長さの平均は、C++ 版では合計 (int) を `size_t` で割る。Rust 版は
  64 ビットの `size_t` として計算する (32 ビットの CPU では、合計が 2GB を超えたとき C++ 版と
  違う。バッファはそれよりずっと小さい)。
* `checkReadDelay` は、C++ 版では `bitrate * 1024` が桁あふれすると未定義の動作で、2^22 の倍数の
  ビットレートでは 0 で割る。Rust 版は桁あふれを 2 の補数で扱い、割る数が 0 なら待たない。
  `readDelay` はファイルを流すときだけ使う。
* `writeTrackerUpdateAtom` などは atom をまとめて 1 回で書く (段階 7b と同じ)。

## 差分テスト

```sh
cd tests/differential
make
./diff                       # cgi/str の UTF-8・エスケープ系: 長さ0〜2バイト全網羅+乱数30万件+全コードポイント
./diff --exhaustive3         # さらに長さ3バイトを全網羅 (1,677万通り、数十秒)
./diff_strutil                # str.cpp のその他の関数: 乱数20万組 (既定)
./diff_strutil --random 300000  # 比較件数780万件相当まで増やして実行
./diff_md5_gnuid              # md5::hexdigest と GnuID: 乱数20万組 (既定)、fromStr の 2 文字の全通り
./diff_jis                     # JISConverter: 65536通り全数 (sjis/euc 各1関数)
./diff_string                  # String: 長さ0〜2全網羅+長さ3〜4を絞ったバイトで全通り+乱数30万件 (約980万件)
./diff_http                    # HTTP の行の解析と parseHttpDate: 見本の変異+乱数 (約120万件)
./diff_amf0_dechunk            # AMF0 と Dechunker: 生成した値の変異・切り詰め (約100万件)
./diff_xml                     # XML: 見本の変異+乱数 (約40万件)
./diff_url                     # URL: 短い入力の全通り+見本の変異+乱数 (約116万件)
./diff_media                   # FLV/MKV/OGG/MP4/MP3: 生成した入力とその変異 (各2万件、引数で変更)
./diff_template 20000 $(find ../../../ui/html -name "*.html")
                             # テンプレート: UI の実際のテンプレート、生成したもの、その変異
./diff_public                  # Accept-Language、formatUptime、コンソールの引数 (約80万件)
./diff_pcp                     # PCP の受け取ったパケット: 生成した atom の木とその変異 (10万件)
./diff_pcp_hs                  # PCP のハンドシェイクの helo / oleh と readVersion (20万件)
./diff_chanpacket               # ChanPacketBuffer: 操作の列 2 万本 (約290万回の比較)
./diff_chandir                  # index.txt の解釈と一覧の URL・時間の文字列 (約120万件)
./diff_chanhit                  # ChanInfo と ChanHit / ChanHitList: 乱数の一覧への操作 (約265万件)
./diff_hostgraph_uptest         # HostGraph と帯域測定の yp4g.xml の読み取りなど (約70万件)
./diff_channel                   # Channel と ChanMgr の移した部分: 乱数のチャンネルへの操作 (約40万件)
```

`diff_http` のように、C++ 版をクラスごと呼びたい差分テストは、Rust を使わずにビルドした
C++ のコア一式 (`cxxcore.a`、`make` が自動で作る) にリンクします。

`diff_media` は、同じ入力と同じ状態の `Channel` で C++ 版と Rust 版の解析器を動かし、
`Channel::newPacket` と `Channel::updateInfo` に渡ったもの (リンク時の `--wrap` で横取りする)、
`sys->sleep` の呼び出し、例外、最後の `Channel` の状態を比べます。時刻は読むたびに進むので、
時刻を読む回数と順序も比べることになります。C++ 版が初期化していないメモリを読む箇所を
比べられるように、差分テストの C++ はスタックを 0 で初期化し (`-ftrivial-auto-var-init=zero`)、
ヒープも 0 で埋めます (`diff_media.cpp` の `operator new`)。

`diff_pcp` は、同じパケットのバッファと同じ状態の `chanMgr`、`servMgr`、`PCPStream` で C++ 版の
`procAtom` と Rust 版を動かし、ログ、中継、通知、ヒットの追加と削除、`Channel::updateInfo`
(`--wrap` で横取りする) と、処理のあとのバッファ、ヒットリスト、チャンネルの状態などを比べます。
C++ 版は深い入れ子で 8MB のスタックを使い果たすので、512MB のスタックのスレッドで動かします。
C++ 版がバッファの終わりで空回りしたもの (ログが 10 万行を超えるか 0.2 秒を超えたもの) は、
比べずに数えます。

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
