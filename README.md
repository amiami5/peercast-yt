# PeerCast YT

PeerCast のフォークです。

## このフォークについて

[plonk/peercast-yt](https://github.com/plonk/peercast-yt) をベースに、
セキュリティ修正を加えたフォークです。本家は長く更新されていませんが、
PeerCast はネットワークからの入力を扱うソフトなので、脆弱性の修正だけを行っています。
新機能の追加はしません（技量的にできません）。主に Linux で使うことを想定しています。

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

Linux の `ui/linux` の Makefile では、これが既定です。C++ 版に戻すこともできます
(→ [Linuxでのビルド](#linuxでのビルド))。

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

**4. コアを少しずつ Rust に置き換えています**

ネットワークからの入力を解釈する部分から順に、C++ のコアを Rust
([`peercast-rs/`](peercast-rs/)) に置き換えています。計画と進み具合は
[`docs/rust-migration.md`](docs/rust-migration.md) にあります。

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

`ui/linux` の Makefile でビルドします (CMake でのビルドは [README_CMAKE.md](README_CMAKE.md))。

## 1. 必要なものを入れる

Ubuntu / Debian なら、次の 1 行で揃います。

```sh
sudo apt install build-essential pkg-config libssl-dev librtmp-dev ruby python3 cargo
```

| パッケージ | 何に使うか | 備考 |
|---|---|---|
| `build-essential` | C++ コンパイラ | C++11 対応なら何でも可 (GCC 4.9 以降、Clang 3.4 以降) |
| `pkg-config` `libssl-dev` | TLS (OpenSSL) | |
| `librtmp-dev` | RTMP fetch (他サーバーからの取得) | 不要なら `WITH_RTMP = no` にする |
| `ruby` | ビルド時の HTML 生成 | |
| `python3` | 実行時の CGI スクリプト | |
| `cargo` | RTMP 受信サーバー (Rust 版) のビルド | Rust 1.75 で確認。C++ 版でよければ不要 |
| `libgtest-dev` | 単体テスト | テストを動かす場合だけ |

`cargo` は `rustup` で入れてもかまいません。

## 2. ビルドしてインストールする

```sh
git clone https://github.com/amiami5/peercast-yt.git
cd peercast-yt/ui/linux
make
sudo make install
```

* `make` は**一般ユーザー**で実行し、`sudo` は `make install` だけにしてください。
  (`rustup` で入れた cargo は `sudo` 環境では見つからず、ビルドに失敗します。)
* インストール先は `/usr/local` です。変えるには `sudo make install PREFIX=/opt/peercast` のようにします。
  `peercast` と `rtmp-server` は `bin/` に一緒に入ります。PeerCast は、自分の実行ファイルと同じ
  ディレクトリにある `rtmp-server` を起動するので、別々の場所に置かないでください。

### RTMP 受信サーバーを C++ 版にする

既定では Rust 版の `rtmp-server` がビルドされます。cargo が使えない環境などでは、C++ 版を選べます。

```sh
make WITH_RUST_RTMP=no
```

毎回指定したくないときは、`ui/linux/Makefile.local` に `WITH_RUST_RTMP = no` と書いておきます。
Rust 版と C++ 版を切り替えるときは、先に `make clean` してください。

## 3. テスト (任意)

```sh
cd ui/linux/tests && make && ./test-all      # PeerCast 本体の単体テスト
cd rtmp-server-rs && cargo test --release    # Rust 版 rtmp-server のテスト
```

# 実行

peercast コマンドを起動したあと、ウェブブラウザで `http://localhost:7144/`
を開くと操作できます。なお、設定ファイル `peercast.ini` は `~/.config/peercast/`
ディレクトリに作られます。

# RTMP fetchサポート

RTMP をサポートするストリーミングサーバーからストリームを取得して配信
チャンネルを作成したい場合、PeerCast YT が RTMP fetch サポート付きでビ
ルドされている必要があります。

Linux (`ui/linux`) では既定でオンです (`Makefile` の先頭の `WITH_RTMP = yes`)。
`librtmp` をリンクする必要があるので、インストールしておいてください。
使わない場合は `WITH_RTMP = no` にしてビルドします。
