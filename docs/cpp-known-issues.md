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
* 移行後のセキュリティの見直し: `URLSource::streamURL` は、中継元の HTTP の応答の `Location` (302) と、
  HTTP で取ったプレイリスト (`text/plain` など) の中の URL を、管理者が入力した URL と区別せずに読む。
  このため中継元が `pipe:コマンド` を返すと外部のプログラムが起動し、スキームのない文字列 (`/etc/passwd`
  など) を返すとローカルのファイルを読んで配信する。Rust 版は、ネットワークから受け取った URL は
  `http://`、`pcp://`、`rtmp://` だけを受け付ける (`url::is_remote_safe_source`)。
* 移行後のセキュリティの見直し: `Servent::handshakeICY` のパスワードの確認は `loginPassword != password` の
  ときだけ localhost かを見るので、サーバーのパスワードが空 (既定) だと、どこからでもパスワードなしで
  ShoutCast / Icecast の放送を始められ、同じ ID の放送があれば止めてしまう。Rust 版は、localhost 以外からは
  パスワードが設定されていて一致するときだけ受け付ける (`servhs::icy_password_ok`)。HTTP Push
  (rtmp-server の経路) は変わらない。
* 移行後のセキュリティの見直し: `ini::Document` の書き出しは値をそのまま書くので、改行を含む値 (PCP で
  受け取った中継チャンネルの名前など。keep にしたものは保存される) で行を足せ、次に起動したときに
  `[End]` で `[RelayChannel]` を抜けて `password` などを書き換えられた。`IniFileBase::readNext` は 255 バイト
  から後ろを次の行として読むので、長い値でも同じことができた。Rust 版は、書くときに制御文字を除いて
  1 行を 255 バイトに収め、読むときも 255 バイトを超えた分を捨てる。最後の行に改行がなくても読む
  (C++ 版は読めなかった)。
* 移行後のセキュリティの見直し: `GnuID::generate` / `GnuID::random` は `sys->rnd()` で ID を作る。`rnd` の
  数列は /dev/urandom から読んだ 32 ビットの種だけで決まる (2 つの状態を同じ値で始める) ので、PCP で
  ほかのノードに送るセッション ID から種を総当たりで割り出せ、そのあとに作るログインの Cookie や
  放送 ID (FLV の `auth` のトークンの元) を予測できた。Rust 版は ID を毎回 /dev/urandom から作る
  (`sys::secure_random`)。
* 移行後のセキュリティの見直し: `/cgi-bin/flv.cgi` (MKV のチャンネルを ffmpeg で FLV に変換してブラウザーで
  見せる) は、設定のトランスコードが無効でも動き、同時に動かす ffmpeg の数に上限がない。`auth` のトークンは
  公開ディレクトリの再生ページ (パスワードなしで見られる) にも出るので、手に入れた人がいくつも開いて
  CPU を使い切れた。Rust 版は、トランスコードが無効なら 403 を返し、localhost 以外からは同時に動かす数を
  設定の `maxTranscodes` (既定 2、設定画面の「トランスコード」で変えられる) までにして、超えたら 503 を返す。
* 移行後のセキュリティの見直し: `URLSource::stream` とプレイリストのループは、入力元がエラーなしですぐに
  終わると待たずにつなぎ直す (エラーのときだけ 1 秒待つ)。このため、すぐに終わる外部のプログラム
  (`pipe:true` で 12 秒に約 2 万回起動した) や、自分自身へのリダイレクト、中身のない応答で休みなく繰り返す。
  `PeercastSource::stream` も、つないですぐに正常に切るノードや 503 を返すトラッカーに、約 10 ミリ秒ごとに
  つなぎ直す。Rust 版は、10 秒未満で終わるのが続くと、2 回目から 1、2、4 秒と待ち (最大 30 秒)、長く続いた
  あとなら戻す。中継では、失敗して別の候補を探すときと、再接続やプッシュの接続が来たときは待たない。
* 移行後のセキュリティの見直し: `/cgi-bin/` (掲示板ビューワーの board.cgi / thread.cgi / post.cgi) は
  `handshakeAuth` でほかのサイトのページからの要求 (CSRF) を断っていない。localhost はパスワードなしで
  通るので、ブラウザーで開いた悪意のあるページが裏で `post.cgi` を呼ぶと、PeerCast がその人の IP から
  掲示板へ書き込んだ。Rust 版は、3 つとも `Sec-Fetch-Site` / `Origin` を見てほかのサイトからの要求を 403 で
  断る (管理画面の掲示板の欄は同じサイトからの要求なので変わらない)。
* 移行後のセキュリティの見直し: チャンネルのコンタクト URL は、PCP で届く中継チャンネルの情報や配信ソフトの
  `icy-url` など他人から届く値なのに、管理画面 (relays.html / play.html) と公開ディレクトリ (index.html /
  play.html) でそのままリンク先 (`href`) にしている。`javascript:…` を入れたチャンネルのリンクを踏むと、
  PeerCast の画面の中でスクリプトが動き、管理画面の操作ができた。Rust 版は、テンプレートに渡すときに
  `http://` / `https://` で始まるものだけを残し、それ以外は空にする (`cgi::link_url`。受け取った値や
  中継先へ送る値、JSON-RPC の出力は変えない)。
* 移行後のセキュリティの見直し: `/admin.cgi` (ShoutCast の配信ソフトが曲名を知らせる口) は `pass=` が
  あるかだけを見て、中身を確かめない。このため、ポートに届く人なら誰でも、配信中の MP3 チャンネルの
  曲名とコンタクト URL を書き換えられ、PCP で視聴者全員に広がった。ブラウザーで開いたほかのサイトからも
  呼べた。Rust 版は、放送を受け付けるときと同じ規則 (`servhs::icy_password_ok`。localhost 以外は
  設定したパスワードと一致するときだけ) で確かめ、違えば 403 を返す。ほかのサイトのページからの要求は
  断り、Host がループバックの名前でない localhost からの要求 (DNS リバインディング) は localhost として
  扱わない。
* 移行後のセキュリティの見直し: 管理画面・API・放送 (ShoutCast / Icecast)・`/admin.cgi` のパスワードは、
  何度間違えても制限がなく、パスワードを設定して外から入れるようにしていると総当たりで破られうる。
  また、デバッグ/トレースのログに要求の行 (`?pass=…`、ShoutCast の放送ではパスワードそのもの) や
  ICY の `Authorization` ヘッダー、Cookie をそのまま書いていた。Rust 版は、同じ IP アドレスから
  続けてパスワードを間違えると、しばらくそのアドレスを締め出して 429 を返す (既定は 5 回で 60 秒、
  そのあとも間違えるたびに倍で最長 1 時間。`[Privacy]` の `authFailLimit` / `authLockSeconds` と
  設定画面で変えられ、`authFailLimit = 0` で締め出さない。localhost は締め出さない)。ログでは
  パスワードを `***` に置き換え、Cookie の値は書かない。
* 移行後のセキュリティの見直し: 受け付けた接続の要求を読む待ち時間は「何も届かない時間」(30 秒) だけで、
  1 文字ずつゆっくり送られると (Slowloris) いつまでも切れなかった。さらに待ち受けのポートの接続数が
  上限 (`maxServIn`、既定 50) に達すると受け付けること自体をやめるので、1 台の PC がゆっくり送る接続を
  50 本張るだけで、視聴者・中継・管理画面 (localhost からも) が誰もつなげなくなった。Rust 版は、
  つないでから要求を読み終えるまでの合計の期限 (既定 15 秒。返事を書き始めるか、HTTP Push・GIV で
  ソケットを渡したら外すので、始まった配信や中継は切らない) を設け、同じ IP アドレスからの読み終えて
  いない接続の数を抑える (既定 8 本。ループバックは数えない)。接続数が上限でも受け付けたうえで
  ループバック以外をすぐ切るので、localhost からは管理画面を開ける。`[Server]` の `handshakeTimeout` /
  `maxHandshakesPerIP` と設定画面で変えられ、0 で制限しない。
* 移行後のセキュリティの見直し: `rtmp-server` (OBS などからの RTMP 配信を受ける口) は、すべてのアドレスで
  待ち受け、ストリームキーも確かめなかったので、ポート (既定 1935) に届くなら誰でも勝手に配信を始められた。
  また配信は 1 本ずつ順に処理するのに、読み取りの待ち時間は「何も届かない時間」(30 秒) だけだったので、
  何も送らない接続や少しずつ送り続ける接続で、ほかの配信をいつまでもふさげた。Rust 版は、既定で
  この PC (`127.0.0.1` と `::1`) だけで待ち受け (`[Server]` の `rtmpLocalOnly`、管理画面の「この PC からだけ」)、
  ストリームキーを設定できるようにし (`rtmpStreamKey`、既定は空で確かめない。キーはコマンドラインに
  載せず環境変数で渡し、ログにも出さない)、接続してから publish が始まるまでの期限 (10 秒) を設けた。
  出力先 (PeerCast への HTTP Push) は最初に FLV を書くときに開くので、断った接続では PeerCast につながない。
* 移行後のセキュリティの見直し: `Host::isLocalhost` (接続がこの PC からかの判定) は、呼ばれるたびに自分の
  ホスト名を名前解決 (`gethostbyname`) していた。接続のたびに何度も呼ばれるので、DNS の応答が遅いと
  接続の処理がそこで数秒ずつ待たされ、大量に接続されるとスレッドが詰まった (ソケットを読んでいない間の
  待ちなので、要求を読み終えるまでの期限でも切れない)。また localhost と判定された接続はパスワードの
  締め出しや接続数の上限から外れるので、その時々の DNS の答えで判定が変わるのも好ましくない。Rust 版は、
  自分のアドレスを起動時に一度引いて覚えておき、そのあとは別のスレッドで決まった間隔ごとに引き直す
  (`[Server]` の `selfIPCheckInterval`、既定 60 秒、0 で起動時だけ。設定画面でも変えられる)。判定の中身
  (ループバックか、自分のホスト名のアドレスか) は変えていない。
* 移行後のセキュリティの見直し: 接続を許す・禁じるフィルター (`ServFilter::matches`) も、ホスト名の
  パターンなら判定のたびに正引きし、`.` で始まる名前のパターンなら接続してきた相手のアドレスを判定の
  たびに逆引きしていた。逆引きの応答は相手の側の DNS サーバーが返すので、相手がわざと遅くすれば、
  接続の処理を好きなだけ待たせられた。Rust 版は、名前解決の結果を 60 秒覚えておき (引けなかったことも
  覚える)、古くなったら古いものを使いつつ別のスレッドで引き直す。まだ覚えていないものは 2 秒まで待ち、
  間に合わなければ一致しないものとする。覚える数 (1024) と同時に走らせる問い合わせの数 (16) にも
  上限を設けた。ホスト名のパターンは、フィルターを設定したときに前もって引き始める。覚えておく秒数と
  待つ時間は ini の `[Server]` の `dnsCacheSeconds` (0 で毎回引き直す) と `dnsWaitMillis` (0 で待たない)
  で変えられる (設定画面にはない)。
* 移行後のセキュリティの見直し: チャンネル一覧 (`ui/html-master/channels.html`) の情報の窓にある
  「フィルタ作成」ボタンは、チャンネル名を `onclick` の JavaScript の文字列にそのまま埋めていた
  (`window.location = "chanfilters.html?fav=<名前>"`)。HTML のエスケープはしているが、ブラウザーは属性の
  `&quot;` を `"` に戻してから JavaScript として実行するので、名前に `"` を入れたチャンネルを YP に
  載せれば、ボタンを押した人の管理画面で好きな JavaScript を動かせた (XSS。管理画面の権限で `/admin` や
  `/api/1` を呼べる)。グループのタブの `setSelectedGroup('<フィードの URL>')` も同じ形だった (こちらは
  自分で設定した YP の URL なので、他人からは使えない)。UI は C++ 版と共通だったので、C++ 版
  (`develop-old`) には残っている。Rust 版は、名前を `data-name` 属性に置いて `this.dataset.name` から読み、
  `encodeURIComponent` して渡す (`&` や `#` を含む名前も正しく渡る)。タブは `Groups` の番号で引く。
* 移行後のセキュリティの見直し: 視聴ページ (`play.html`) の掲示板の欄 (`ui/html-master/bbs.js`) は、
  thread.cgi が返すレスの本文を HTML のままページに入れ、名前・メール・日付もエスケープせずに `title`
  属性に入れていた。読む掲示板はチャンネルのコンタクト URL で決まり、配信者が自分のサーバーを指定できる
  ので、レスに `<img src=x onerror=…>` などを入れておけば、そのチャンネルの視聴ページを開いた人の
  管理画面で好きな JavaScript を動かせた (XSS)。UI は C++ 版と共通だったので、C++ 版 (`develop-old`) には
  残っている。Rust 版の UI は、本文を `<template>` の中で解釈し、文字と `<br>` と `http(s)://` のリンク
  だけを作り直して入れる (ほかのタグは捨てて中身の文字だけを残す)。
* 移行後のセキュリティの見直し (2026-10-01。`docs/security-review-todo.md` の #20〜#26。Rust 版は #20 を直した):
  `Servent::handshakeStream` は、`/stream/` の応答の `Content-Type` に `ChanInfo::getMIMEType()` (PCP の
  `styp` で配信者やほかのノードが送ってきた値) をそのまま書き、`icy-name:` や `x-audiocast-*:` にも
  チャンネル名などを改行を除かずに書く (MIME タイプを `text/html` にされると、配信の中身が管理画面と
  同じオリジンの HTML として開かれる。改行でヘッダーを書き足せる)。UI の `relays.html` の
  `/stream/{$this.id}{$this.ext}` のリンクも、`ext` が PCP の `sext` そのまま。ShoutCast 形式の放送は
  1 行目がパスワードで始まるかだけを見て、応答で合否が分かる (C++ 版にはもともとパスワードの締め出しが
  ない)。PCP の PUSH は、届いた宛先 (ループバックや LAN を含む) へ数の上限なく接続する (`initGIV`)。
  認証なしの `/stream/`・`/channel/`・`/pls/` の `?ip=` や `?tip=` でヒットを足せる (`procConnectArgs`)。
  PCP の root atom をどのノードから届いても受け付ける。管理パスワード・Basic 認証・放送のパスワード・
  ログインの Cookie の ID の比較も定数時間ではない (#28。C++ 版は締め出しもないので、応答までの時間から
  1 文字ずつ当てる余地がより大きい)。
* 移行後のセキュリティの見直し (2026-10-01。`docs/security-review-todo.md` の #29〜#32。Rust 版はすべて直した):
  速度測定 (`UptestEndpoint::takeSpeedtest` → `postRandomData`) は、yp4g.xml の `uptest_srv` の `addr`・`port`・
  `object` をそのまま POST の宛先とパスに使い、宛先の制限も改行の確認もない (既定の登録先は平文の HTTP)。
  `post_size` にも上限がない。`CMD_speedtest_cached_xml` は、取ってきた yp4g.xml を `application/xml` で
  管理画面と同じオリジンから返す。viewxml (`ChanInfo::createChannelXML`) は、`type` 属性に PCP で届いた
  `contentType` をエスケープせずに書き、ほかの属性も `T_UNICODESAFE` の変換が UTF-8 の先頭バイトのあとの
  バイトを確かめずに通すので、`"` や `<` が残りうる。メディアのパーサーも、MKV の要素 (256 MB まで) などを
  まるごとメモリに読む。
* 移行後のセキュリティの見直し (2026-10-01。`docs/security-review-todo.md` の #33〜#41。Rust 版はすべて直した):
  `http::get` と `URLSource` は、リダイレクト先や HTTP で取ったプレイリストの中身の URL がループバックでも
  つなぎに行き、管理画面はループバックからの要求を認証なしで通すので、取得先を握った者が管理コマンドを
  実行させられる (既定のフィードは平文の HTTP)。HTTP Push (`POST /`) と、`/stream/`・`/pls/` の `?ip=`・`?tip=`
  は、localhost からならほかのサイトのページからの要求 (CSRF) も受け付ける。PCP の `readPktAtoms` は、
  届いた接続が上流かどうかを見ずにチャンネルにパケットを書く。引数のない GIV のソケットは COUT に使われ、
  トラッカーの更新で BCID が渡る (トラッカーの更新は YP 以外への COUT にも BCID を入れる)。チャンネル ID の
  付いた GIV のソケットは、PUSH を頼んでいなくても中継の配信元に使われる (#41)。ヒットリストとヒットの数に
  上限がない。管理画面の応答に `X-Frame-Options` も CSP の `frame-ancestors` もなく、ほかのサイトの枠に
  入れられる (クリックジャッキング)。`--enable-notify-send` (ui/linux/main.cpp) は、チャンネル名やコメントを
  エスケープせずに通知の本文に渡すので、通知のデーモンがマークアップとして解釈する。
