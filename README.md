# PeerCast YT

PeerCast のフォークです。

## このフォークについて

[plonk/peercast-yt](https://github.com/plonk/peercast-yt) をベースに、
セキュリティ修正を加えたフォークです。本家は長く更新されていませんが、
PeerCast はネットワークからの入力を扱うソフトなので、脆弱性の修正だけを行っています。
新機能の追加はしません。主に Linux で使うことを想定しています。

### 本家との違い

**1. RTMP 受信サーバー (`rtmp-server`) が Rust 製になりました**

OBS などから RTMP で配信を受け付ける `rtmp-server` を、C++ から Rust に書き直しました
([`rtmp-server-rs/`](rtmp-server-rs/))。コマンドラインも、PeerCast 本体との接続方法も従来と
同じなので、使い方は変わりません。外部から最初に入力を受ける部分なので、メモリ安全性が言語で
保証される Rust にしています。C++ 版で問題になっていた次の場面も、Rust 版では直っています。

| 場面 | C++ 版 | Rust 版 |
|---|---|---|
| エンコーダーが Shift_JIS などの非 UTF-8 文字列を送ってきた | 配信が切れる | 配信を続ける |
| 出力先 (PeerCast 本体) に接続できない | プロセスごと異常終了する | その接続だけ切って待ち受けを続ける |
| 巨大な未完了メッセージを大量に送りつけられた | 最大で約 1 GiB のメモリを確保する | 合計 64 MiB で打ち切る |

**2. セキュリティ修正**

* 管理画面・API: 同一オリジン検証 (CSRF 対策)、ログイン Cookie に `SameSite=Strict`、
  リダイレクト先や `htmlPath` の検証
* 入力の検証: HTTP・チャンネル情報の URL は http(s) のみ許可 (SSRF 対策)、HTTP ヘッダー数と
  チャンクサイズの上限、JSON・AMF0・atom のネストの深さと値の数の上限
* メモリ安全: `strcpy` / `sprintf` によるバッファオーバーフローの修正、FLV・MP4・OGG・MKV・
  MP3 の各パーサーの長さ検査と未初期化バッファの修正
* TLS: SNI の送信と証明書のホスト名検証 (Unix 系ビルドのみ)
* HTML テンプレート: JavaScript 文字列内の `<` `>` `&` をエスケープ

**3. 使われなくなった形式のサポートを外しました**

ここ数年使われていない次の形式は、配信・視聴ともにサポートをやめ、コードを削除しました。

* NSV (Nullsoft Streaming Video)
* Windows Media 系: WMA / WMV (ASF)、MMS (MMSH) での視聴、Windows Media HTTP Push 配信、
  ASX プレイリスト、`mms://` の配信元、設定の「WMV プロトコル」(`wmvProtocol`)

これらの形式のチャンネルを他のノードから受け取った場合、種類は UNKNOWN として扱われ、
中身は解析せずにそのまま流れます。古い `peercast.ini` に `wmvProtocol` が残っていても、
読み飛ばされるだけで問題ありません。

**4. PeerCast 本体も Rust で書き直しました**

C++ のコアを段階的に Rust ([`peercast-rs/`](peercast-rs/)) に移し、このブランチ (`develop-rs`) では
C++ のコードをすべて取り除きました。ネットワークとのやりとりや設定ファイル、HTML の管理画面、
JSON-RPC などのふるまいは C++ 版と同じにしてあります (C++ 版と Rust 版に同じ要求を送って応答を
比べて確かめました)。移行の記録は [`docs/rust-migration.md`](docs/rust-migration.md)、移行中に
見つかった C++ 版の不具合は [`docs/cpp-known-issues.md`](docs/cpp-known-issues.md) にあります。

このブランチは Linux 用です。Windows 版 (GUI) と macOS 版は C++ で書かれていたので、ここにはありません。
C++ 版のコードは `develop-old` ブランチにあります。

個々の変更は `git log` で確認できます。ライセンスは本家と同じ GPL です。

## ブラウザインターフェイス

ブラウザインターフェイスは、YPブラウザ、動画プレーヤー、したらば掲示板
ビューワを実装しており、ユーザーはウェブブラウザさえあれば ローカル/リ
モートを問わず PeerCast が視聴できます。

## 多種のエンコーダーに対応

* RTMP に対応しており、OBS などのエンコーダーで配信できます。
  →[RTMPプロトコル対応エンコーダーでの配信のやり方](https://github.com/plonk/peercast-yt/wiki/RTMP%E3%83%97%E3%83%AD%E3%83%88%E3%82%B3%E3%83%AB%E5%AF%BE%E5%BF%9C%E3%82%A8%E3%83%B3%E3%82%B3%E3%83%BC%E3%83%80%E3%83%BC%E3%81%A7%E3%81%AE%E9%85%8D%E4%BF%A1%E3%81%AE%E3%82%84%E3%82%8A%E6%96%B9)
* HTTP Push に対応しており、ffmpeg で配信できます。
  →[HTTP Push 配信のやり方](https://github.com/plonk/peercast-yt/wiki/HTTP-Push-%E9%85%8D%E4%BF%A1%E3%81%AE%E3%82%84%E3%82%8A%E6%96%B9)
## 多種の動画フォーマットに対応

* FLV、MKV、WebM、MP4、OGG、MP3 の配信に対応しています (それ以外は RAW として、中身を
  解析せずにそのまま流します)。

## その他

* [継続パケット機能](docs/continuation-packets.md)により、キーフレーム
  からの再生ができます。
* PeerCastStation 互換の JSON RPC インターフェイス。
  →[JSON RPC API](https://github.com/plonk/peercast-yt/wiki/JSON-RPC-API)
  ([epcyp](https://github.com/mrhorin/epcyp)、
  [ginger](https://github.com/plonk/ginger/) などで使えます)
* <del>公開ディレクトリ機能。チャンネルリストやストリームをWebに公開できます。</del>
* HTML UI をメッセージカタログ化。各国語版で機能に違いがないようにしました。
* Ajax による画面更新。

# Linuxでのビルド

リポジトリの一番上の Makefile でビルドします。中では `cargo` で Rust のコードをビルドし、管理画面の
HTML も作ります (これも Rust の小さなツール `tools/ui-gen`)。

## 1. 必要なものを入れる

Ubuntu / Debian なら、次の 1 行で揃います。

```sh
sudo apt install cargo pkg-config libssl-dev librtmp-dev python3
```

| パッケージ | 何に使うか | 備考 |
|---|---|---|
| `cargo` | Rust のコンパイラとビルド | Rust 1.70 以降 (1.70、1.75、1.85 で確認)。`rustup` で入れてもよい |
| `pkg-config` `libssl-dev` | TLS (OpenSSL) | |
| `librtmp-dev` | RTMP fetch (他サーバーからの取得) | 不要なら `make WITH_RTMP=no` |
| `python3` | 実行時の CGI スクリプト | |

外部のクレート (Rust のライブラリ) は使っていないので、ビルド中にネットワークからは何も取ってきません。

## 2. ビルドしてインストールする

```sh
git clone https://github.com/amiami5/peercast-yt.git
cd peercast-yt
make
sudo make install
```

* `make` は**一般ユーザー**で実行し、`sudo` は `make install` だけにしてください。
  (`rustup` で入れた cargo は `sudo` 環境では見つからず、ビルドに失敗します。)
* `make` で `build/peercast-yt/` に、実行ファイル (`peercast`、`rtmp-server`) と HTML などをまとめた
  ものができます。インストールせずに `cd build/peercast-yt && ./peercast -P .` で動かすこともできます。
* インストール先は `/usr/local` です。変えるには `sudo make install PREFIX=/opt/peercast` のようにします。
  `peercast` と `rtmp-server` は `bin/` に一緒に入ります。PeerCast は、自分の実行ファイルと同じ
  ディレクトリにある `rtmp-server` を起動するので、別々の場所に置かないでください。
  消すときは `sudo make uninstall` です。
* ほかに `make dist` (tar.gz)、`make appimage`、`make clean` があります。変数は `Makefile.local` に
  書いておくこともできます。
* Rust のコードだけなら `cargo build --release` でもビルドできます (出力は `build/target/release/`)。

## 3. テスト (任意)

```sh
make check    # 単体テストと、サーバーを実際に起動して試すテスト (cargo test --release --workspace)
```

中継や配信元の種類ごとの確認 (Python) は [`peercast-rs/tests/server/`](peercast-rs/tests/server/) にあります。

# 実行

peercast コマンドを起動したあと、ウェブブラウザで `http://localhost:7144/`
を開くと操作できます。なお、設定ファイル `peercast.ini` は `~/.config/peercast/`
ディレクトリに作られます。

# RTMP fetchサポート

RTMP をサポートするストリーミングサーバーからストリームを取得して配信
チャンネルを作成したい場合、PeerCast YT が RTMP fetch サポート付きでビ
ルドされている必要があります。

既定でオンです (`librtmp` をリンクするので、インストールしておいてください)。
使わない場合は `make WITH_RTMP=no` でビルドします。
