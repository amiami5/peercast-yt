# 管理ページを HTTPS で使う

VPS などに PeerCast を置いて、外から管理ページを開くときの手順です。平文の HTTP のままだと、
ログインのパスワードや Cookie がそのままネットワークを流れるので、HTTPS にしてください。

証明書は Let's Encrypt から certbot で取り、PeerCast はそのファイルを読むだけです。
HTTPS は同じポート (既定 7144) で受けるので、ポートを増やす必要はありません。

## しくみ

- フラグ `enableSSLServer` をオンにすると、同じポートで TLS の接続と平文の接続の両方を受けます
  (最初のバイトで見分けます)。受け付けるのは TLS 1.2 以上です。
- 証明書と秘密鍵は、設定ファイル (`peercast.ini`) と同じディレクトリの `server.crt` と `server.key` です。
  既定では `~/.config/peercast/` です (`-i` で ini を指定したときは、そのディレクトリ)。
- 2 つのファイルは**接続のたびに読み直します**。証明書を更新したら、ファイルを差し替えるだけでよく、
  PeerCast の再起動はいりません。
- `enableSSLServer` がオンのとき、平文で localhost 以外から管理ページ (`/`、`/html/`、`/admin`、
  JSON-RPC、`/cgi-bin/`、`/cmd?`) に来たら、GET は `https://<同じホスト>/…` にリダイレクトし、
  それ以外は 403 で断ります。
- 次のものは今までどおり平文でも使えます: ストリームの視聴、PCP (ほかのノードとの中継)、`admin.cgi`
  (配信ソフトからの操作)、`/public`、`/assets`。localhost からの接続も平文で使えます。
- HTTPS でログインしたときは、Cookie に `Secure` を付けます (平文の接続ではブラウザーが送りません)。

## 用意するもの

- ドメイン名 (例: `pc.example.com`)。VPS のグローバル IP アドレスを指す A (AAAA) レコードを作っておく。
- 80 番ポートを外から開けられること。Let's Encrypt が証明書を出すとき (更新のときも) に、80 番に
  HTTP でつないで確かめるためです。7144 番では確かめられません。
- certbot (`sudo apt install certbot`)。Ubuntu / Debian のパッケージなら、更新の timer も一緒に入ります。
- 管理ページのパスワードを設定しておくこと (設定のページの「パスワード」)。

以下では、PeerCast を一般ユーザー `peercast` で動かし、設定のディレクトリが
`/home/peercast/.config/peercast/` だとします。自分の環境に合わせて読み替えてください。

## 1. 証明書を取る

80 番で何も動いていなければ、certbot に一時的に 80 番で待ち受けさせる `--standalone` が簡単です。

```sh
sudo certbot certonly --standalone -d pc.example.com \
    --deploy-hook /usr/local/sbin/peercast-cert-hook
```

すでに nginx などが 80 番で動いているときは、`--standalone` の代わりに
`--webroot -w <そのサーバーの公開ディレクトリ>` を使います。

`--deploy-hook` は、証明書を取ったとき・更新したときに実行するスクリプトです (次の 2 で作ります)。
certbot はこの指定を覚えていて、更新のときにも実行します。**スクリプトを先に作ってから**
certbot を実行してください (先に証明書を取ってしまったときは、2 のあとで手で 1 回実行します)。

取った証明書は `/etc/letsencrypt/live/pc.example.com/` に置かれます。

| ファイル | 中身 | PeerCast の名前 |
|---|---|---|
| `fullchain.pem` | サーバーの証明書と中間証明書 | `server.crt` |
| `privkey.pem` | 秘密鍵 | `server.key` |

`cert.pem` (サーバーの証明書だけ) は使わないでください。中間証明書が送られず、ブラウザーや curl で
検証に失敗することがあります。

## 2. 証明書を PeerCast の設定のディレクトリにコピーする

`/etc/letsencrypt/` の秘密鍵は root しか読めません。PeerCast を root で動かさずに済むように、
取得・更新のたびに、PeerCast のユーザーのものとしてコピーします。

`/usr/local/sbin/peercast-cert-hook` (実行できるようにする: `sudo chmod 755 /usr/local/sbin/peercast-cert-hook`):

```sh
#!/bin/sh
set -e
DOMAIN=pc.example.com
OWNER=peercast
DIR=/home/peercast/.config/peercast

# ほかのドメインの証明書の更新では何もしない
case " $RENEWED_DOMAINS " in
  *" $DOMAIN "*) ;;
  *) exit 0 ;;
esac

# 一時ファイルに書いてから名前を変える (PeerCast が書きかけのファイルを読まないように)
install -o "$OWNER" -g "$OWNER" -m 600 "$RENEWED_LINEAGE/privkey.pem"   "$DIR/server.key.new"
install -o "$OWNER" -g "$OWNER" -m 644 "$RENEWED_LINEAGE/fullchain.pem" "$DIR/server.crt.new"
mv -f "$DIR/server.key.new" "$DIR/server.key"
mv -f "$DIR/server.crt.new" "$DIR/server.crt"
```

鍵と証明書を入れ替えるあいだのごく短い時間は、鍵と証明書が合わないので TLS の接続に失敗しますが、
つなぎ直せば使えます。

証明書を先に取ってしまったときは、手で 1 回実行してコピーします。

```sh
sudo RENEWED_LINEAGE=/etc/letsencrypt/live/pc.example.com RENEWED_DOMAINS=pc.example.com \
    /usr/local/sbin/peercast-cert-hook
```

### symlink にする場合

コピーせずに、`server.crt` と `server.key` を `fullchain.pem` と `privkey.pem` への symlink にしても動きます。

```sh
ln -s /etc/letsencrypt/live/pc.example.com/fullchain.pem ~/.config/peercast/server.crt
ln -s /etc/letsencrypt/live/pc.example.com/privkey.pem   ~/.config/peercast/server.key
```

ただし、PeerCast のユーザーが `/etc/letsencrypt/live/`・`/etc/letsencrypt/archive/` のディレクトリと
秘密鍵を読めるように、グループや権限を変える必要があります。ほかのドメインの鍵まで読めるようになりやすく、
権限の設定を誤りやすいので、上のコピーの方法をすすめます。

## 3. `enableSSLServer` をオンにする

管理ページの「フラグ」のページで `enableSSLServer` をオンにします。すぐに効きます (再起動はいりません)。

PeerCast を止めているときなら、`peercast.ini` に書いてもかまいません。真偽値は `Yes` / `No` です
(`true` と書くとオフになります)。

```ini
[Flags]
enableSSLServer = Yes
[End]
```

ファイアウォールでは、7144 番 (PeerCast) と 80 番 (certbot) を開けておきます。

それまで平文の HTTP で外からログインしていたときは、そのときの Cookie には `Secure` が付いていないので、
http:// で開くと、リダイレクトされる前の最初の要求で平文のまま送られます。オンにしたら、https:// で開いて
一度ログアウトし (サーバーの側でもそのログインが無効になります)、ログインし直してください。
設定の「クッキー期限」が「永続」のときは、ブラウザーを閉じても Cookie が残るので、特に忘れないでください。

## 4. 確かめる

ブラウザーで `https://pc.example.com:7144/` を開き、警告なしで管理ページが出れば使えています。
コマンドで確かめるなら:

```sh
# 証明書が検証できるか (失敗するとエラーで終わる)
curl -sS -o /dev/null -w '%{http_code}\n' https://pc.example.com:7144/

# 送られてくる証明書の名前と期限
openssl s_client -connect pc.example.com:7144 -servername pc.example.com -verify_return_error </dev/null 2>/dev/null \
    | openssl x509 -noout -subject -enddate

# 平文で来ると https にリダイレクトされるか (302 と Location: https://… が出る)
curl -sS -o /dev/null -D - http://pc.example.com:7144/
```

リダイレクトの確かめは、VPS の**外から**行ってください。VPS の上で実行すると、自分のアドレスからの接続は
localhost と同じ扱いになり、リダイレクトされません。また、`curl -I` は HEAD を送るので、リダイレクトではなく
403 になります (平文で GET 以外が来たら断るため)。

`s_client` の「Verify return code: 0」は、ハンドシェイクに失敗したときにも出ることがあるので、
`-verify_return_error` を付けて、証明書が取れるかで判断してください。

つながらないときは、PeerCast のログを見てください。TLS の接続を受け付けられないと
`Incoming from <接続元>: …` のエラーが出ます (平文の接続はそのまま使えます)。

| ログの末尾 | 原因 |
|---|---|
| `Certificate file` | `server.crt` がない、読めない、PEM でない |
| `Private key file` | `server.key` がない、読めない、証明書と合わない |
| `upgrade: SSL_accept: …` | ハンドシェイクの失敗。かっこの中に OpenSSL の理由が出る (`unsupported protocol` ならクライアントが TLS 1.1 以下しか使えないなど) |
| `Handshake timeout` | 要求を読み終えるまでの期限 (設定の `handshakeTimeout`、既定 15 秒) を過ぎた |

## 5. 更新

Let's Encrypt の証明書の期限は 90 日です。更新は certbot の timer (`certbot.timer`) に任せます。
期限の 30 日前になると certbot が更新し、`--deploy-hook` のスクリプトがファイルを差し替えます。
PeerCast は次の接続から新しい証明書を使うので、再起動はいりません。

```sh
systemctl list-timers | grep certbot   # timer が動いているか
sudo certbot renew --dry-run            # 更新を試す (証明書は変わらない)
```

`--dry-run` では deploy-hook は実行されません。スクリプトを試すときは、2 の手で実行する方法を使ってください。

## リバースプロキシ (nginx など) を前に置くのはすすめません

nginx などで HTTPS を受けて、PeerCast には平文で転送する形もよくありますが、PeerCast では次の問題があります。

- PeerCast から見た接続元が、すべて `127.0.0.1` (プロキシ) になります。PeerCast は、localhost から
  来て Host ヘッダーも `localhost` や `127.0.0.1` の要求はパスワードなしで通します。nginx は既定では
  Host を転送先の `127.0.0.1:7144` に書き換えるので、`proxy_set_header Host $host;` を忘れると、
  **外からパスワードなしで管理ページに入れてしまいます**。平文で来た要求を https に向ける処理も、
  localhost からの接続には効きません。
- 接続元で判断する機能が効かなくなります: IP アドレスの許可、フィルター (禁止・中継の制限など)、
  パスワードを何度も間違えたときのロックアウト。
- PeerCast は `X-Forwarded-For` などのヘッダーに対応していません。

PeerCast が直接 TLS を受ける、この文書の方法を使ってください。
