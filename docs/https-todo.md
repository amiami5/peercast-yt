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
- [x] 3. TLS の接続では、ログインの Cookie に `Secure` を付ける
  - 済み: `servhs::login_cookie` で `Set-Cookie` の値を組み立てる (単体テストあり)。平文の接続では付けない
    (localhost の平文や、`enableSSLServer` がオフのときにログインできなくならないように)。
- [x] 4. bvt に TLS のテストを足す
  - 済み: bvt の `tls` (Unix のみ。openssl コマンドを使う)。テストのサーバーを起こす前に、ポートに
    ほかのプロセスが待ち受けていないかを確かめるようにした (`Server::start_with`)。
  - テストで見つかったこと: TLS の接続を閉じるときに close_notify を送っていなかった。TCP の書き込みを
    先に閉じ、TcpStream を捨てて記述子を閉じてから `SSL_shutdown` していた (閉じた記述子、または同じ番号を
    使った別の接続に書くおそれがあった)。`ClientSocket::close` で Session を先に捨てるように直した。
  - テストのときに openssl コマンドで、ルート CA → 中間 CA → サーバーの証明書を作る。
  - 確かめること: 中間証明書まで送ること、ルートだけを信頼して検証が通ること、TLS 1.1 を断ること、
    合わない鍵に差し替えると断り、戻すとまたつながること、平文の HTTP も使えること、Cookie の `Secure`。
  - 注意: テストのサーバーが起動できたか (ポートを別のプロセスが使っていないか) を確かめてから接続する。
    s_client の「Verify return code: 0」はハンドシェイクに失敗しても出るので、送られた証明書の数などで判定する。
- [x] 5. 手順書 `docs/https.md` (README からリンク)
  - 済み: 鍵は root しか読めないので、symlink よりも deploy-hook で PeerCast のユーザーのものとして
    コピーする方法をすすめている (一時ファイルに書いてから mv)。ログのエラー (`Certificate file` など) と
    その原因の表も載せた。リバースプロキシでは、nginx の既定で Host が `127.0.0.1:7144` になり、
    パスワードなしで入れてしまうことを書いた。
  - certbot での取得 (80 番を開ける。standalone か webroot)。
  - `server.crt` と `server.key` を fullchain.pem と privkey.pem への symlink にする。鍵を読めるようにする権限
    (または deploy-hook でコピー)。
  - `enableSSLServer` の有効化と、接続の確かめ方。更新は certbot の timer に任せる (再起動はいらない)。
  - リバースプロキシ (nginx など) を前に置くのはすすめない理由: 接続元がすべて 127.0.0.1 になり、
    Host を転送しないと認証なしで入れる。IP の許可やフィルター、ロックアウトも効かなくなる。

- [x] 6. 最終チェックで見つかったこと
  - TLS の接続では、要求を読み終えるまでの期限 (`handshakeTimeout`) が効いていなかった。`SSL_accept` と
    `SSL_read` の中の recv のたびに読む待ち時間をまるごと使えるので、ハンドシェイクや要求のレコードを
    1 バイトずつ送ると、いつまでも居座れた (平文は期限で切れる)。期限のある間はソケットをノンブロッキングに
    して、`poll` で期限までの残りだけ待つようにした (`tls::Session::run`、`socket::with_deadline`)。
    bvt の `tls_slow_client` で確かめる。
  - `SSL_accept` の失敗のログに OpenSSL の理由を出すようにし、SSL の関数を呼ぶ前にエラーのキューを空にする。
  - TLS の `read_upto` が受信量の統計に数えていなかった。
  - 手順書: リダイレクトの確かめを `curl -I` (HEAD なので 403) から GET にし、VPS の外から行うことを書いた。
    平文のころのログインの Cookie をログアウトで無効にすることを書いた。

## やらないこと

- ACME (Let's Encrypt の取得と更新) をサーバーに組み込むこと。certbot があるので使う。
- リバースプロキシ用の X-Forwarded-For の対応。要望が出たら考える。

- [x] 7. Let's Encrypt の証明書の取得から有効化までを一括して行うスクリプト `tools/peercast-https-setup`
  - 済み: 動いている peercast のプロセス (/proc の cmdline・environ) から、ユーザーと設定のディレクトリを
    調べる。deploy-hook は `--deploy-hook` ではなく `/etc/letsencrypt/renewal-hooks/deploy/` に置く
    (すでにある証明書にも効く)。フラグは、動いていれば localhost の `/cmd?q=flag set …` で、止まって
    いれば peercast.ini を書き換えてオンにする。HTTPS はオプションのまま (既定はオフ、`make install` にも
    入れない)。
  - WSL では root で通しては試せていない (certbot を動かせないため)。プロセスからの設定のディレクトリの
    検出、`/cmd` でのフラグの設定と ini への保存、ini の書き換え、HTTPS の確かめは一般ユーザーで試した。
