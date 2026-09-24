# サーバー全体のテスト

Rust のサーバー (`src/bin/peercast.rs`) を実際に起動して試すスクリプト。先に `ui/linux` で `make` して
`ui/linux/peercast-yt` (html などの配布用のディレクトリ) を作っておく。C++ 版と比べるものは、
`ui/linux` で `make WITH_RUST_SERVER=no TARGET=peercast-cxx peercast-cxx` で C++ 版も作っておく
(C++ 版を消すまで)。サーバーは `/tmp/pcyt-servertest` (環境変数 `PCYT_TEST_DIR` で変えられる) の下で
起こす。設定は `bvt/peercast.ini.master` の YP を空にしたもので、外のホストにはつながない。

* `relay_test.py SRC_BIN RELAY_BIN`: HTTP Push で配信し、直接の視聴と、別のサーバーでの PCP の中継を
  試す。Rust 同士のほか、C++ 版との組み合わせ (両方向) も試せる。
* `source_test.py BIN`: HTTP の取得 (fetch)、ShoutCast と Icecast の放送、ICY のメタデータ付きの視聴。
* `server_diff.py`: C++ 版と Rust 版を同じ設定で起こし、同じ要求 (HTML の各ページ、JSON-RPC、管理画面と
  コンソールのコマンド、不正な要求など) を送って応答を比べる。時刻やポート番号など起動ごとに変わる
  ものは伏せて比べる。違ったものは `$PCYT_TEST_DIR/serverdiff/out` に書き出す。
