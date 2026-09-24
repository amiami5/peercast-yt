# Rust への段階的移行計画

PeerCast YT の C++ 実装を、動く状態を保ったまま少しずつ Rust に置き換えていくための計画書です。
段階が終わるたびに、下の「進捗」を更新しました。

**移行は終わりました。** `develop-rs` ブランチでは C++ のコードをすべて取り除き、ビルドはリポジトリの
一番上の `Makefile` と cargo のワークスペースで行います。C++ と Rust が混ざった途中の版は `develop-old`
ブランチにあります。この文書の「進め方」などは、移行中の記録として残しています (ここに出てくる
`tests/` の gtest、`peercast-rs/tests/differential/`、`ui/linux/Makefile` などは `develop-old` にあります)。

## 目的と前提

* 目的は、信頼できない相手からの入力を最初に受ける部分から順に、メモリ安全性が言語で保証される
  Rust にすることです。機能の追加はしません (このフォークの方針どおり)。
* どの段階でも、`make` して `make install` した成果物がそのまま使える状態を保ちます。
  半分だけ動かない状態のコミットは作りません。
* 挙動は **C++ 版と同じ** にします。違いが出るときは、理由を README か各段階のコミットメッセージに
  書きます (`rtmp-server-rs/README.md` の「C++ 版との違い」と同じ扱い)。

## 進め方

`rtmp-server` は別プロセスだったので、実行ファイルを差し替えるだけで済みました。それ以外の部分は
1 つのプロセスの中でつながっているため、**Rust の静的ライブラリ (`peercast-rs`) を C++ 側にリンクし、
関数単位で C++ の実装を Rust 呼び出しに置き換えていきます**。

* C++ との境界は C ABI (`extern "C"`) にします。渡すのは、平たいバイト列と単純な構造体だけです。
  C++ のクラスを Rust から直接いじる設計はしません。
* `unsafe` は境界のコードだけに閉じ込めます。パーサーなどの本体は `#![forbid(unsafe_code)]` にします。
* 外部クレートは既定で使いません。必要になったら、その段階で個別に判断します。

### 各段階の完了条件

1. 既存の単体テスト (`tests/` の gtest) が、Rust 実装に差し替えた状態で全部通る。
2. `bvt/` のブラックボックステスト (Ruby) が通る。
3. C++ 版と Rust 版に同じ入力を与えて出力を比べる差分テストがある (`rtmp-server-rs/tests/differential.py` と同じ考え方)。
4. 入力を受ける部分には、変異ファズのテストがあり、panic やハングがない。
5. 置き換えた C++ のコードは、その段階のうちに消す。
6. コミットメッセージは日本語で書く。

## 段階

行数は `core/common` の該当ファイルの概算です。

| 段階 | 内容 | 概算 |
|---|---|---|
| 0 | `rtmp-server` を Rust 化 (完了) | 950 行 |
| 1 | 基盤 (Cargo ワークスペース、静的ライブラリ、Makefile と gtest からの呼び出し) と、境界が単純な関数 (エスケープ、URL エンコード、パス検証、UTF-8 検証) | 〜1,000 行 |
| 2 | 文字列・文字コード・ハッシュ (`str`, `_string`, `jis`, `md5`, `sha1`, `base64`, `gnuid`) | 3,000 行 |
| 3 | 入力パーサー (`cgi`, `url`, `uri`, `xml`, `http`, `amf0`, `dechunker`)。`atom` は段階 6 で `pcp` と一緒に扱う | 3,800 行 |
| 4 | メディアコンテナの解析 (`flv`, `mkv`, `ogg`, `mp3`, `mp4`)。NSV と Windows Media 系 (`asf`, `mms`, `wmhttp`) は Rust 化せずにサポートをやめた | 1,600 行 |
| 5 | テンプレートエンジン、HTML、管理画面コマンド (`template`, `html`, `commands`, `public`) | 3,300 行 |
| 6 | PCP プロトコル (`pcp`, `chanpacket`, `atom`) | 1,400 行 |
| 7 | チャンネルとホストの管理 (`channel`, `chanmgr`, `chaninfo`, `chanhit`, `chandir`, `hostgraph`, `uptest`) | 5,600 行 |
| 8 | JSON-RPC と HTTP ハンドラ (`jrpc`, `servhs`) | 4,000 行 |
| 9 | 接続と並行処理 (`servent`, `servmgr`, `socket`, `sys`, スレッド)、`main`。ここで C++ を全部外す | 6,900 行 |

解析器が `Stream` (ブロッキング読み出し) から読む必要がある場合は、コールバックで読む形に
決めました (段階 3b)。Rust 側が C++ のコールバックを呼んで `Stream` から読み、C++ の例外は
コールバックの中で捕まえて、Rust から戻ったあとで投げ直します。データがメモリ上にそろっている
ものは、バイト列を受け取る形にします。詳しくは `peercast-rs/README.md` の段階 3b を参照。
段階 9 は影響範囲が最も大きいので、それまでの段階で C++ 側の依存を減らしておきます。

## 段階 9 の進め方

段階 8 までは、C++ の関数を 1 つずつ Rust の呼び出しに置き換えた。残る部分 (`Servent`、`ServMgr`、
`ChanMgr`、`Channel` の状態とスレッド、ソケット、`main`) は、グローバルなポインタと共有ポインタで
互いの欄を直接読み書きしているので、クラスごとに境界を作ると、消す予定の C++ 側の書き換えが
大きくなる。そこで段階 9 は次のように進める。

* `peercast-rs` の中に、Rust だけで動くサーバー (`src/server/`) と実行ファイル (`src/bin/peercast.rs`)
  を作る。段階 1〜8 で移した解析と判断 (PCP、HTTP、テンプレート、JSON-RPC など) はそのまま使い、
  それぞれの `Host` トレイトをサーバーの状態に対して実装する。
* スレッドとブロッキングのソケット (タイムアウト付き) という C++ 版の作りはそのままにする
  (非同期のランタイムは使わない)。外部クレートは使わない。TLS (HTTPS の取得と SSL での受け付け)
  と RTMP の取得は、C++ 版と同じく OpenSSL と librtmp を C ABI で呼ぶ (`unsafe` はそのモジュール
  だけ)。正規表現 (`std::regex` の ECMAScript の文法) も Rust で書く。
* 途中のコミットでは、C++ 版のビルド (`WITH_RUST_CORE`) はそのまま動く。Rust のサーバーは、全部
  そろったところで Linux のビルドの実行ファイルとして切り替え、そこで C++ を外す。
* 段階を 9a (土台: ログ、時刻と乱数、ソケットと TLS、`Stream`、正規表現、ini、統計、通知、Cookie、
  フィルター、フラグなど)、9b (チャンネル: ヒットリスト、`ChanMgr`、`Channel` と配信元、YP)、
  9c (接続: `Servent` の各ハンドシェイクと中継、`ServMgr`、JSON-RPC とテンプレートの状態、管理画面の
  コマンド)、9d (`main`、ビルドの切り替え、C++ の削除) に分ける。
* 確認: 移した部品ごとの単体テストと、C++ がある間は C++ 版の部品との差分テスト (ini、正規表現、
  ホストの文字列など)。サーバー全体は、C++ 版と Rust 版の両方を起動して同じ要求を送り、応答を
  比べるテストと、`bvt/` で確かめる。

## 今は対象にしないもの

次のものは、C++ を Rust に置き換える作業とは別に、あとで判断します。

* Windows 版 (`core/win32`, `ui/mingui`, MSYS2 のビルド)。段階 9 までは C++ のまま残します。
* CMake ビルド。Rust 版を組み込むかどうかは、段階 1 のあとで決めます。
* Python の CGI スクリプト (`ui/cgi-bin`)、ビルド時の Ruby、HTML と JavaScript。

## 進捗

| 段階 | 状態 |
|---|---|
| 0 | 完了 |
| 1 | 完了 (peercast-rs: cgi/str の一部関数、C ABI 境界、Makefile 統合)。差分テストは長さ 0〜3 バイトの入力を全網羅 (1,677 万通り) して確認 |
| 2 | 完了 (str の残り、jis、md5、gnuid の純粋な部分、`String` の変換関数)。`String` クラス自体と `setFromTime`、`GnuID::generate` など状態や OS に依存する部分は C++ に残る。差分テストは String だけで約 980 万件、違いなし |
| 3 | 完了。3a (HTTP の行の解析、`parseHttpDate`)、3b (AMF0、chunked 転送)、3c (XML)、3d (URL)。`atom` は段階 6 に回した。`cgi::Query` と `HTTP::parseRequestLine` は、Rust 化済みの `str::split` などを呼ぶだけなので C++ のまま |
| 4 | 完了。相談の結果、NSV と Windows Media 系 (WMA/WMV、ASF、MMS、WMHTTP、ASX) のサポートを削除し、残る FLV、MKV/WebM、OGG、MP3、MP4 の解析器を Rust 化した (`peercast-rs/src/media`)。チャンネルとのやりとりは `pcrs_media_host` のコールバックで行う。差分テストは各形式 10 万件で、説明のつかない違いなし |
| 5 | 完了。5a (テンプレートエンジンの式とディレクティブ)、5b (Accept-Language の解釈、コンソールの引数の解釈)。差分テストは UI の実際のテンプレートとその変異などで 20 万件、5b は約 80 万件で、説明のつかない違いなし。テンプレートのスコープ (変数) と正規表現、`html.cpp`、`commands.cpp` の各コマンドの本体、`public.cpp` の HTTP の処理は、入力を解釈せず `servMgr` や `chanMgr` を呼ぶだけなので、段階 7〜9 で扱う |
| 6 | 完了。6a (受け取ったパケットの処理: `procAtom` 以下と `ChanInfo::readInfoAtoms` / `readTrackAtoms`)、6b (ハンドシェイクで受け取る `helo` / `oleh` と `readVersion`)、6c (`ChanPacketBuffer`)。チャンネルやサーバーの状態は `pcrs_pcp_host` のコールバックで触り、返事を書くことと読んだ値を使った処理は C++ に残る。差分テストは 6a が 10 万件、6b が 20 万件、6c が約 290 万回の比較で、説明のつかない違いなし。atom を書く側 (`AtomStream` の write 系、`writeInfoAtoms` など) と `PCPStream` のソケットの読み書きは、それを使うチャンネルや接続の処理と一緒に段階 7〜9 で扱う |
| 7 | 完了。7a (イエローページの index.txt の解釈、`chatUrl` / `statsUrl` など)、7b (`ChanInfo` と `TrackInfo`、`ChanHit` と `ChanHitList` の状態を持たない部分。atom を書く側の `writeInfoAtoms` / `writeTrackAtoms` / `ChanHit::writeAtoms` を含む)、7c (`HostGraph` の親子の決め方、帯域測定の yp4g.xml の読み取りなど)、7d (`Channel` と `ChanMgr` の、ICY のメタデータの解釈、トラッカーへの更新などの atom、状態の画面の文字列、`authToken` など)。差分テストは 7a が約 120 万件、7b が約 265 万件、7c が約 70 万件、7d が約 40 万件で、説明のつかない違いなし。チャンネルのスレッドと、チャンネルとヒットリストの連結リストの管理は段階 9、`createXML` と `getState` は段階 9 で扱う (状態を書き出すだけなので、状態を持つクラスと一緒に移す)。7a の差分テストで、段階 1c の `GnuID::fromStr` が `strtoul` の空白と符号の読み方を再現していなかったことがわかり、Rust 版を直した |
| 8 | 完了。8a (JSON-RPC の `JrpcApi`。JSON は nlohmann::json 3.7.3 と同じふるまいのものを `src/json` に作った)、8b (HTTP の要求の解釈と判断: 要求の行とパスの振り分け、認証の Cookie、CGI の引数、`CMD_apply`、ICY のヘッダー、ローカルのファイルのパスなど)、8c (`/public` と `/assets`: `FileSystemMapper`、振り分けと MIME タイプ、If-Modified-Since、index.txt、`HTTPRequest` の URL の分割)。差分テストは 8a が 12.5 万件、8b が約 220 万件、8c が約 160 万件と index.txt 2.5 万件で、説明のつかない違いなし。8c で、`FileSystemMapper` が隣のディレクトリ (`public2` など) へのトラバーサルを通していたことを Rust 版で直した。状態を書き出すだけの `getState`、`createXML`、`CMD_dump_hitlists` は、状態を持つクラスと一緒に段階 9 で扱う |
| 9 | 完了。9a (サーバーの土台: 正規表現、IP アドレスとホスト、ログ、時刻と乱数、ソケットと TLS、`Stream`、HTTP のやりとり、ini、統計、通知、Cookie、フィルター、旗、外部のプログラム)。差分テストは正規表現が約 42 万件、ホストとフィルターが約 110 万件で、説明のつかない違いなし。9a で、`HTTP::getResponse` の Content-Length の条件が逆なこと、/0 のフィルター、`Environment::set` を Rust 版で直した。9b と 9c (チャンネル、ヒットリスト、`ChanMgr`、配信元、YP、`Servent` の各ハンドシェイクと中継、`ServMgr`、JSON-RPC とテンプレートの状態、`/public` と `/assets`、管理画面のコマンド、プレイリスト、RTMP の取得 (feature `rtmp`))と、実行ファイル `peercast-rs/src/bin/peercast.rs` (ui/linux/main.cpp)。bvt の 00〜03 が Rust のサーバーで通り、HTTP Push の配信、直接の視聴、PCP の中継を Rust 同士と、C++ 版との間 (両方向) で確かめた。9d の前半: ui/linux の `make` が Rust のサーバーを `peercast` として作るようにした (`WITH_RUST_SERVER`。`make WITH_RUST_SERVER=no` で C++ 版)。C++ 版と Rust 版を同じ設定で起こして同じ要求を送るテスト (`peercast-rs/tests/server`、125 件) で、違いは時刻やポート番号など起動ごとに変わるものだけ。HTTP の取得、ShoutCast と Icecast の放送、ICY のメタデータ付きの視聴も両方で確かめた。C++ のソースの削除は、Windows 版 (C++ のまま) の扱いを決めてから行う。9d の後半 (`develop-rs` ブランチ): Windows 版はこのブランチでは扱わないことにし、C++ のコード (core、C++ 版の rtmp-server、gtest、ui の Windows・macOS 版、CMake、C++ との橋渡しの `src/ffi.rs` と差分テスト) を取り除いた。ビルドはリポジトリの一番上の `Makefile` と cargo のワークスペースにした (`make`、`make install`、`make check` など)。C ABI で呼ばれることがなくなったので、panic はプロセスを止めずに、そのスレッド (接続やチャンネル) の処理だけを終わらせてログに書くようにした (C++ 版のスレッドが例外を捕まえてログに書いていたのと同じ) |

### 確認環境についての注記

* 移行の途中で見つけた C++ 版の不具合は、C++ を最後に消すので C++ 側では直さず、`docs/cpp-known-issues.md` にメモとして残す。

* gtest は Ubuntu 26.04 (g++ 15、googletest 1.12.1) で 724/726 件成功 (段階 4 で C++ 版の内部のクラスのテスト 5 件、段階 6 で `readPktAtoms` のテスト 2 件、段階 8a で `JrpcApi` の中身のテスト 2 件を、C++ 版のビルドでだけ動くようにした)。失敗する 2 件
  (`ServentFixture.handshakeStream_returnResponse_channelReady_direct`、`ServMgrFixture.writeVariable`)
  は移行前から失敗している。`IniFixture.parse` は、g++ 15 で既定になった
  `_GLIBCXX_ASSERTIONS` が `ini.cpp` の `trim` の範囲外アクセス (空文字列で `str[-1]`) を検出して
  止まるので、除外して走らせている (移行前からある C++ 版のバグ)。
* `bvt/` は 00〜03 が成功。`02-html` は UI から消えた `bcid.html` を見ているので、その 1 件を
  除いて確認している。`04-helo` は移行前から失敗している。
* Rust のコードは CPU に依存しない書き方にする (ARM でも同じ結果になる)。C++ 版の結果が
  `char` の符号 (x86 は符号付き、ARM の Linux は符号なし) で変わっていた箇所は、Rust 版では
  どちらかに決めて `peercast-rs/README.md` に書く。
