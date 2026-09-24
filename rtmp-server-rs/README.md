# rtmp-server (Rust 版)

PeerCast YT 付属の `rtmp-server` です。もとは C++ 版 (`develop-old` ブランチの `rtmp-server/`) の Rust 移植で、
コマンドラインも出力も同じです。

    rtmp-server [-p PORT] URL...      # 既定のポートは 1935

- `http://host:port/path?query` : 接続して `POST path?query HTTP/1.0` を送り、以降 FLV をそのまま流す (PeerCast 本体へ)
- `file://...` または URL でない文字列 : ファイルに書き出す

PeerCast 本体との境界は今までと同じ (別プロセス + ループバック HTTP) で、FFI はありません。

## ビルド

リポジトリの一番上の Makefile が、`peercast` と一緒にビルドしてインストールします (ルートの README.md を参照)。

    make                      # build/peercast-yt/ に peercast と rtmp-server ができる
    sudo make install         # /usr/local/bin に peercast と rtmp-server が入る

単体でビルドする場合は `cargo build --release -p rtmp-server` (出力は `build/target/release/rtmp-server`)。
PeerCast は自分の実行ファイルと同じディレクトリの `rtmp-server` を起動します。

Rust 1.70 以降 (1.70、1.75、1.85 で確認)。外部クレートには依存せず、ビルド中のネットワークアクセスも不要です。

## 検証

下の表の C++ 版との比較は、C++ 版があったときに `tests/differential.py` (`develop-old` ブランチ) で行ったもの。

    cargo test --release                                   # 単体テスト + 結合テスト (23 件)
    python3 tests/robustness.py RUST_BIN                   # タイムアウト、出力先切断など

| 項目 | 結果 |
|---|---|
| ffmpeg (h264 + aac, 5 秒) → Rust 版 → ffprobe | 映像 75 / 音声 217 パケット、長さ 5.000 秒で欠落なし |
| 手組みの 22 シナリオ (正常系・異常系) を C++ 版と比較 | FLV、サーバー応答、生存状況がすべてバイト単位で一致 |
| 変異ファズ 400 件を C++ 版と比較 | 395 件一致、5 件差異 (下記の (a) が原因)。Rust 版は 0 件クラッシュ |
| 黙って居座るクライアント | 約 30 秒で切断し、次の配信を処理 |

## C++ 版との違い

意図的な差異です。ここに挙げたもの以外は同じ振る舞いに揃えてあります。

- (a) 文字列が UTF-8 として不正でもセッションが落ちない。C++ 版はログ出力用の `inspect()` が
  `std::invalid_argument` を投げ、コマンド名・パラメータ・メタデータのどれかに非 UTF-8 (例: Shift_JIS) が
  含まれるとその配信全体が終了する。
- (b) 出力先 (PeerCast) に接続できなくてもサーバーが落ちない。C++ 版は `openUri()` が
  `try` の外にあり、接続拒否で `terminate` する。Rust 版はそのクライアントだけ切って続行する。
- (c) 未完了メッセージの保持量の合計に上限 (64 MiB) を設けた。C++ 版は 24 ビット長を名乗るメッセージを
  62 本のチャンクストリームで同時に始められ、最大で 1 GiB 近く確保する。
- (d) 送信メッセージが 1 チャンクに収まらない場合のヘッダー生成を修正 (現状の応答は全て収まるため実害なし)。

## C++ 版から引き継いだ制限 (機能修正はしない方針のため、そのまま)

- 拡張タイムスタンプ (0xFFFFFF 以上のヘッダー値) は未対応でセッション終了
- チャンクストリーム ID 64 以上 (基本ヘッダー 2〜3 バイト形式)、AMF3 コマンド (type 0x11) は未対応
- `@setDataFrame` なしの `onMetaData` を直接送るエンコーダーでは、壊れた FLV になる
- メタデータを 2 回送るとセッション終了
- fmt 3 で次のメッセージが始まるとき、タイムスタンプの差分を再適用しない

## ライセンス

元のプログラムが GPL (v2 以降) なので、この移植も GPL-2.0-or-later です。
