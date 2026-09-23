# Rust への段階的移行計画

PeerCast YT の C++ 実装を、動く状態を保ったまま少しずつ Rust に置き換えていくための計画書です。
段階が終わるたびに、下の「進捗」を更新します。

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
| 3 | 入力パーサー (`cgi`, `url`, `uri`, `xml`, `http`, `amf0`, `atom`, `dechunker`) | 3,800 行 |
| 4 | メディアコンテナの解析 (`flv`, `mkv`, `ogg`, `asf`, `mp3`, `nsv`, `mp4`) | 2,600 行 |
| 5 | テンプレートエンジン、HTML、管理画面コマンド (`template`, `html`, `commands`, `public`) | 3,300 行 |
| 6 | PCP プロトコル (`pcp`, `chanpacket`, `atom`) | 1,400 行 |
| 7 | チャンネルとホストの管理 (`channel`, `chanmgr`, `chaninfo`, `chanhit`, `chandir`, `hostgraph`, `uptest`) | 5,600 行 |
| 8 | JSON-RPC と HTTP ハンドラ (`jrpc`, `servhs`) | 4,000 行 |
| 9 | 接続と並行処理 (`servent`, `servmgr`, `socket`, `sys`, スレッド)、`main`。ここで C++ を全部外す | 6,900 行 |

段階 4 の前に、解析器が `Stream` (ブロッキング読み出し) にどう依存しているかを整理して、
Rust 側の設計 (バイト列を受け取る形にするか、コールバックで読むか) を決めます。
段階 9 は影響範囲が最も大きいので、それまでの段階で C++ 側の依存を減らしておきます。

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
| 3 以降 | 未着手 |

### 確認環境についての注記

* gtest は Ubuntu 26.04 (g++ 15、googletest 1.12.1) で 738/740 件成功。失敗する 2 件
  (`ServentFixture.handshakeStream_returnResponse_channelReady_direct`、`ServMgrFixture.writeVariable`)
  は移行前から失敗している。`IniFixture.parse` は、g++ 15 で既定になった
  `_GLIBCXX_ASSERTIONS` が `ini.cpp` の `trim` の範囲外アクセス (空文字列で `str[-1]`) を検出して
  止まるので、除外して走らせている (移行前からある C++ 版のバグ)。
* `bvt/` は 00〜03 が成功。`02-html` は UI から消えた `bcid.html` を見ているので、その 1 件を
  除いて確認している。`04-helo` は移行前から失敗している。
* Rust のコードは CPU に依存しない書き方にする (ARM でも同じ結果になる)。C++ 版の結果が
  `char` の符号 (x86 は符号付き、ARM の Linux は符号なし) で変わっていた箇所は、Rust 版では
  どちらかに決めて `peercast-rs/README.md` に書く。
