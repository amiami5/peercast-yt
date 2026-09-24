# C++ 版の既知の不具合 (メモ)

C++ のコードは Rust への移行が終わったら消すので、移行の途中で見つけた C++ 版の不具合は C++ 側では
直さず、ここに書き留めておく。Rust 版での扱い (直したか、同じ動きにしたか) は
`peercast-rs/README.md` の各段階の「C++ 版との違い」を見ること。

## まだ Rust に移していない部分 (移すときに扱う)

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

* `XML::read` は、最上位の要素が 2 つ以上あると前の根を解放しない (`XML::setRoot` が上書き
  するだけ)。Rust 版の帯域測定の読み取り (段階 7c) も、最後の要素だけを根にする。
* `XML::Node::findAttr` は名前の先頭が一致すれば見つかったことにする (`port` で `port_open` も
  見つかる)。Rust 版も同じにした (段階 7c)。

* `LogBuffer::write` は、途中までの UTF-8 (例えば `"a\xe3\x81"`) で終わる文字列を渡すと、99 バイトずつ
  切り分けるループが進まなくなり、ログのロックを持ったまま終わらない。今はログを書くのが `ADDLOG`
  だけで、不正な UTF-8 を先に置き換えるので起きない (段階 9)。
* `PublicController::createChannelIndex` は `getChannels` の結果を `LOG_DEBUG` に出すために `dump()` する
  ので、配信元の URL などに不正な UTF-8 があると `type_error` が飛び、index.txt を返さずに接続が切れる
  (`GeneralException` でないので捕まえられない)。Rust 版 (段階 8c) も同じく例外にしている。接続の
  処理を移す段階 9 で、例外をどう扱うか決める。

* `Servent::handshakeGET` などが `handshakeAuth` に渡す引数は `http.cmdLine` の中を指していて、
  `handshakeAuth` の中の `readHeaders` で書き換えられる。このため `/html/`、`/cmd?`、`/cgi-bin/` の
  `?pass=` は、最後のヘッダーの行の残りから読まれ、ほぼ効かない (段階 9)。
* `/admin.cgi` (ShoutCast の曲名の更新) は、`pass=` があるかだけを見て、パスワードの中身を確かめない。
* `CMD_stop_servent` などの `std::stoi` は、`isDecimal` を通った大きすぎる数で `std::out_of_range` を
  投げ、捕まえられずにスレッドの一番上まで飛ぶ (応答を返さずに接続が切れる)。

* `Channel::getBufferString` は、受信の速さが 0 のとき 0.0 / 0.0 を `%.2f` で書くので、x86 では
  `-nan`、ARM では `nan` になる。Rust 版は CPU によらず `-nan` にした (段階 7d)。

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
* 段階 7d: `Channel::checkReadDelay` は `info.bitrate * 1024` を int で計算するので、ビットレートが
  大きいと桁あふれ (未定義の動作) し、2^22 の倍数では 0 で割って落ちる。Rust 版は割る数が 0 なら
  待たない。
* 段階 8b: `handshakeSOURCE` は、ICE/1.0 でない `SOURCE` の行に `/` がないと、行の先頭より前の
  メモリを読み、`/` の値のバイトがあればその 1 つ前に NUL を書く (範囲外の読み書き)。
* 段階 8a: JSON-RPC の要求に `1e400` のような `double` に収まらない数があると、nlohmann の
  `out_of_range` を捕まえず (`parse_error` だけを捕まえている)、応答を返さずに接続を切る。Rust 版は
  Parse error を返す。
* 段階 8c: `FileSystemMapper::toLocalFilePath` のディレクトリトラバーサルの確認は、解決したパスの
  先頭が文書のディレクトリと一致するかだけを見る。このため、名前の先頭が同じ隣のディレクトリ
  (`.../public` に対する `.../public2`) の中のファイルを、`/public` の下のシンボリックリンクなどから
  返してしまう。Rust 版は、続きがパスの区切りであることも確かめる。
* 段階 8b: `/api/1` の Content-Length が `int` に収まらないと、`atoi` の切り詰めで負の数になり 411 を
  返す (値によっては正の数になり、違う長さを読む)。Rust 版は端の値にするので 413。
* 段階 9a: `HTTP::getResponse` の `if (contentLengthStr.empty())` は条件が逆。Content-Length があるときに
  接続が閉じるまで読み、ないときに 0 バイトだけ読む。
* 段階 9a: `ServFilter` の IPv4 のネットマスクは `(uint32_t)-1 << (32 - netmask)` なので、/0 は 32 ビット
  ずらす未定義の動作で、x86 ではずらさない (`0.0.0.0/0` が 0.0.0.0 にしか一致しない)。
* 段階 9a: `Environment::set` は、ある変数を置き換えるときに `名前=` を付けずに値だけを入れる。
* 段階 9a: `Host::fromStrName` と `ServFilter::setPattern` は、IPv6 の正規表現に合うが `inet_pton` で読めない
  もの (`fe80::1%eth0` などのスコープ付きのアドレス) で `FormatException` を投げる。設定ファイルのフィルター
  なら `loadSettings` がそこで止まる。Rust 版は `::` として続ける。
* 段階 9a: `LogBuffer::write` が途中までの UTF-8 で終わらない件 (上) も、Rust 版は進まなくなったらやめる。
* 段階 9b: `RTMPClientStream::read` は `RTMP_Read` の -1 (エラー) を読んだ量として扱い、残りが増えて
  バッファーの前に書き戻す。Rust 版はエラーにする。
* 段階 9b: `PlayList::readSCPLS` と `readPLS` は空の行で読むのをやめる (`readLine` が 0 を返すため)。
  プレイリストの途中の空の行より後の URL は読まれない。Rust 版も同じ。
* 段階 9c: コンソールの `get` コマンドは、位置引数でなく `argv[0]` を URL として使う (`get -- URL` で "--" を
  取りに行く)。Rust 版も同じ。
* 段階 9c: `POST /admin` に Content-Length がないと、`HTTP::getRequest` の `GeneralException` を
  `incomingProc` が捕まえず (捕まえるのは `HTTPException` と `StreamException` だけ)、スレッドの外側で
  ログに書かれるだけで、応答を返さずに切る。Rust 版も応答は返さない。
* 段階 9c: `CMD_stop_servent` などの `std::stoi` は、`int` に収まらない番号で `std::out_of_range` を投げ、
  応答を返さずに切る。Rust 版は、`stop_servent` は見付からない (404)、`*_speedtest` は同じく切る。
* 段階 9d: ui/linux/main.cpp は `-i` などの引数を `String::setFromString` で読むので、空白を含むパスは
  空白の手前で切れる (引用符も外す)。Rust 版は引数をそのまま使う。
* 段階 9d: ui/linux/main.cpp のシグナルハンドラーはログを書く (メモリの確保を伴うので、シグナル
  ハンドラーの中では安全でない)。Rust 版はフラグを立てるだけにして、ログは後で書く。
* 全体: `char` の符号や `double` から `int` への変換など、CPU によって結果が変わる箇所
  (Rust 版は CPU によらず x86 と同じ結果にしている)。
