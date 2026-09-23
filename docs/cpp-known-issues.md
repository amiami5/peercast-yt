# C++ 版の既知の不具合 (メモ)

C++ のコードは Rust への移行が終わったら消すので、移行の途中で見つけた C++ 版の不具合は C++ 側では
直さず、ここに書き留めておく。Rust 版での扱い (直したか、同じ動きにしたか) は
`peercast-rs/README.md` の各段階の「C++ 版との違い」を見ること。

## まだ Rust に移していない部分 (移すときに扱う)

* `HTTP::getResponse` の `if (contentLengthStr.empty())` は条件が逆。Content-Length があるときに
  接続が閉じるまで読み、ないときに 0 バイトだけ読む (段階 9)。
* `URLSource::streamURL` は、プレイリストの中の URL を再帰呼び出しで読むので、プレイリストを指す
  プレイリストが続くと再帰が深くなる (段階 7〜9)。
* `GeneralException` (と派生クラス) はコピーすると、`msg` がコピー元の `msgbuf` を指したままになる。
  コピー元が消えると `msg` は壊れた文字列になる (`std::make_exception_ptr` などで起きる)。
* `ini.cpp` の `trim` は、空文字列で `str[-1]` を読む (g++ 15 の `_GLIBCXX_ASSERTIONS` で止まるので、
  gtest の `IniFixture.parse` は除外して走らせている)。
* `ChanPacketBuffer::findPacket` などのループは、`lastPos` が `UINT_MAX` のとき終わらない
  (4G 個のパケットを送った場合。元のコードのコメントにもある)。段階 6c で Rust 版は直した。
* `ChanPacketBuffer::copyFrom` (使われていない) は `packets[writePos++]` を 64 で割らずに書く
  (配列の外に書く)。段階 6c で Rust 版は直した。
* gtest の `ServentFixture.handshakeStream_returnResponse_channelReady_direct` と
  `ServMgrFixture.writeVariable` は、移行前から失敗している。
* `bvt/04-helo.rb` は、返した oleh のエージェント名の形の確認で、移行前から失敗している。
  `bvt/02-html.rb` は UI から消えた `bcid.html` を見ている。

* `ChanInfo::getTypeFromMIME` は OGM と MP4 にならない (OGM の行は OGG と同じ `MIME_XOGG` を見て
  いて、MP4 の行がない)。Rust 版も同じにした (段階 7b)。
* `ChanHitList::pickHits` は、除外するセッション ID が 0 のとき、セッション ID が 0 のホストを
  選ばない。Rust 版も同じにした (段階 7b)。
* `ChanHit::init` は `direct` を true にしたあと 0 にしている。

## Rust 版で直したもの (C++ 版には残っている)

段階ごとの詳しい説明は `peercast-rs/README.md`。主なもの:

* 段階 5a: テンプレートの `nth` の範囲外読み出し、入れ子の上限がない (スタックを使い果たす)、
  例外のあとに破棄したスコープへのポインタが残る。
* 段階 5b: Accept-Language のタグが 17 個以上で `q=nan` があると、`std::sort` が配列の外を読む。
* 段階 6a: PCP の `bcst` の入れ子に上限がなく、数百段でスタックを使い果たして落ちる。NUL で
  終わらない文字列の atom で、初期化されていないスタックの中身を文字列として使い、ほかのノードへ
  中継することもある。子の数を偽った atom で最大約 21 億回空回りする。
* 段階 4: OGG Vorbis のコメント数で最大約 21 億回空回りする、ちょうど 8192 バイトのコメントで
  スタックのバッファの外に 1 バイト書く、など。
* 全体: `char` の符号や `double` から `int` への変換など、CPU によって結果が変わる箇所
  (Rust 版は CPU によらず x86 と同じ結果にしている)。
