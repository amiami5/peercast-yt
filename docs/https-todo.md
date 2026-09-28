# 管理ページの HTTPS 化の工程表

VPS などに置いたときに、管理ページを HTTPS で使えるようにする。証明書の取得と更新は
certbot に任せ、peercast は証明書のファイルを読むだけにする (ACME をサーバーに組み込まない)。
作業ブランチは `develop-rs-tls`。済んだら「済み」とコミットを書き足す。

## 前提 (今あるもの)

- フラグ `enableSSLServer` をオンにすると、同じポート (7144) で最初のバイトが 0x16 の接続を TLS として受ける
  (`servent_http::handshake_incoming`)。
- 証明書と鍵は、設定のディレクトリの `server.crt` と `server.key`。接続のたびに読み直すので、
  更新してファイルを差し替えれば再起動はいらない。
- ini の真偽値は `Yes` / `No` (`true` は偽になる)。

## 工程

- [x] 1. 証明書の読み込み (`server/tls.rs`)
  - 済み (1eef75d): 証明書をチェーンのファイルとして読む (`SSL_CTX_use_certificate_chain_file`)。
    前は 1 枚目しか送らず、fullchain.pem の中間証明書が届かないので、curl などで検証に失敗していた。
    受け付ける TLS を 1.2 以上にした。
  - 鍵が証明書と合わないときは、もともと `SSL_CTX_use_PrivateKey_file` で失敗するので、確認は足していない。
- [x] 2. 平文で外から管理ページに来たら断る
  - 済み: 判定は `servhs::plain_admin` (Host は `valid_host_header` で確かめる)、接続側は
    `servent_http::require_tls`。`handshake_auth` の最初と、`/` などの振り分け (`K::Other`) で呼ぶ。
    HEAD も GET ではないので 403 にしている。
  - `enableSSLServer` がオンで、平文の接続で、localhost 以外から来たとき。
    GET は `https://<Host>/…` に 302 でリダイレクトする (Host の文字を確かめる)。それ以外は 403。
  - 対象: 認証を通るもの (`handshake_auth`)。`/`、`/html/`、`/admin`、JSON-RPC、`/cgi-bin/`、`/cmd?`。
  - 対象外: admin.cgi (配信ソフトは平文でしか来られない)、ストリーム、PCP、`/public`、`/assets`。
  - 判定とリダイレクト先の組み立ては関数に分けて単体テストする (bvt は localhost から来るので対象外になる)。
- [ ] 3. TLS の接続では、ログインの Cookie に `Secure` を付ける
- [ ] 4. bvt に TLS のテストを足す
  - テストのときに openssl コマンドで、ルート CA → 中間 CA → サーバーの証明書を作る。
  - 確かめること: 中間証明書まで送ること、ルートだけを信頼して検証が通ること、TLS 1.1 を断ること、
    合わない鍵に差し替えると断り、戻すとまたつながること、平文の HTTP も使えること、Cookie の `Secure`。
  - 注意: テストのサーバーが起動できたか (ポートを別のプロセスが使っていないか) を確かめてから接続する。
    s_client の「Verify return code: 0」はハンドシェイクに失敗しても出るので、送られた証明書の数などで判定する。
- [ ] 5. 手順書 `docs/https.md` (README からリンク)
  - certbot での取得 (80 番を開ける。standalone か webroot)。
  - `server.crt` と `server.key` を fullchain.pem と privkey.pem への symlink にする。鍵を読めるようにする権限
    (または deploy-hook でコピー)。
  - `enableSSLServer` の有効化と、接続の確かめ方。更新は certbot の timer に任せる (再起動はいらない)。
  - リバースプロキシ (nginx など) を前に置くのはすすめない理由: 接続元がすべて 127.0.0.1 になり、
    Host を転送しないと認証なしで入れる。IP の許可やフィルター、ロックアウトも効かなくなる。

## やらないこと

- ACME (Let's Encrypt の取得と更新) をサーバーに組み込むこと。certbot があるので使う。
- リバースプロキシ用の X-Forwarded-For の対応。要望が出たら考える。
