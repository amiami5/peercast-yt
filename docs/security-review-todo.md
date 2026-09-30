# Rust 版セキュリティレビューの残り

2026-09-24 のレビュー (#1〜#17、重さの順) のうち、#1〜#12 は直し済み。
ここには #13 以降を残す。直したら「済み」とコミットを書き足す。

- [x] #13 設定のポート番号を、数字の値そのまま 16 ビットの数に切り詰めています。たとえば 70000 を入れると、エラーにならずに 4464 番になります。
  - 済み: 1〜65535 の外は受け付けない。設定画面はポートを変えずに警告をログに残し、`control_rtmp` は 400 を返し、ini の `serverPort` は既定の 7144 にする。
- [x] #14 平文のパスワードが入った ini と、ログインのトークン一覧のファイルを、既定の権限で作っています。umask によっては 0644 になり、ほかのユーザーから読めます。
  - 済み: どちらも一時ファイルを 0600 で作ってから名前を変える (`stream::open_private`)。前からあった一時ファイルも開いたあとに 0600 にするので、umask や古いファイルの権限によらない。0644 で残っていた ini も、次に設定を保存したときに 0600 になる。
- [x] #15 portcheck が平文の HTTP で受け取った JSON を、入れ子の深さを確かめずに解析しています。途中で通信を書き換えられると、プロセスごと落とされます。
  - 済み: jrpc にあった深さの判定を `json::nesting_is_too_deep` に移し、portcheck (IPv6) の応答も 16 段を超えたら解析せずにエラーにする。ほかの `json::parse` は自分のトークンファイルと内部の状態だけなので対象外。
- [x] #16 ログインの Cookie に HttpOnly が付いていません。
  - 済み: `cmd=login` の Set-Cookie (期限なし・あり両方) に `HttpOnly` を付けた。UI の JS は Cookie を読んでいないので影響はない。
- [x] #17 `pass=` の効き方がパスごとに違います。`/html/…?pass=` は効かず、`&pass=` なら効き、`/admin?pass=` は効きます。
  - 済み: `/html/`・`/cgi-bin/`・`/cmd?` は、パス全体ではなく最初の `?` より後ろ (`servhs::query_part`) を認証に渡す。`/admin?` や `POST /api/1?` と同じく `?pass=` が効き、パスに続けた `&pass=` は効かない。

## 2026-09-30 に見つけたもの (UI の JavaScript)

- [x] #18 チャンネル一覧の「フィルタ作成」ボタンが、チャンネル名を `onclick` の JavaScript の文字列に埋めていました (名前に `"` を入れると XSS)。
  - 済み: 名前は `data-name` 属性に置き、`this.dataset.name` を `encodeURIComponent` して渡す。グループのタブも `Groups` の番号で引くようにした。C++ 版に残ることは `docs/cpp-known-issues.md` に書いた。
- [x] #19 視聴ページの掲示板の欄が、掲示板から届いたレスの本文・名前・メール・日付を HTML のままページに入れていました。掲示板のサーバーはコンタクト URL で配信者が好きに指定できるので、自分のサーバーに `<img onerror=…>` などを入れたスレッドを置けば、視聴ページを開いた人の管理画面で JavaScript を動かせました (XSS)。
  - 済み: `ui/html-master/bbs.js` は、本文を `<template>` の中で解釈し、文字と改行 (`<br>`) と `http(s)://` のリンクだけを DOM の API で作り直して入れる。名前などは文字だけを取り出して `title` に入れる。URL や題名を属性に入れるところも `h()` を通す。C++ 版に残ることは `docs/cpp-known-issues.md` に書いた。

## 2026-10-01 に見つけたもの (全体の見直し、重さの順)

コードを読んで判断したもので、実際に動かしての確認はまだしていない。

- [x] #20 `/stream/` の応答の `Content-Type` に、PCP の `styp` (配信者やほかのノードが送ってくる MIME タイプ) をそのまま使っています (`servent.rs` の `return_stream_headers` → `ChanInfo::mime`)。`text/html` にされると、配信の中身が管理画面と同じオリジンの HTML として開かれます。管理コマンドの `fetch` は `pipe:` で外部のプログラムを起こせるので、ここで JavaScript が動くと PC の上でのコマンドの実行まで届きます。
  - 案: `/stream/` の `Content-Type` は、種類 (`type`) から決まる既知のメディアの MIME タイプだけにする (`styp` は表にあるものだけ受け付けるか、使わない)。あわせて `X-Content-Type-Options: nosniff` と `Content-Security-Policy: sandbox` を付ける。
  - 済み: `chaninfo::stream_mime` で、`styp` は `audio/*`・`video/*` (サブタイプは英数字と `.+-_` だけ) と `application/x-ogg`・`application/ogg`・`application/octet-stream` のときだけ使い、それ以外は種類の表から決める。`/stream/` の HTTP の応答 (ICY の形も) には `X-Content-Type-Options: nosniff` と `Content-Security-Policy: sandbox` を付けた。bvt の `relay`・`sources` でヘッダーを確かめる。`styp` に `text/html` を入れて送ってくるノードを立てての確認はしていない (単体テストで `stream_mime` を確かめた)。
- [x] #21 同じ応答で、チャンネル名・ジャンル・説明・URL (`icy-name:` や `x-audiocast-*:`) を、改行などを除かずにヘッダーに書いています。PCP から届く文字列 (`pcpstream.rs` の `chan_info_string`) は制御文字を落としていないので、応答のヘッダーを書き足せます。
  - 案: PCP で受け取るときに制御文字を除き、ヘッダーに書くときにも除く。
  - 済み: 制御文字 (0x00〜0x1f と 0x7f) を除く `pcstr::strip_controls` を作った。PCP で受け取る文字列 (`chan_info_string`、トラックの情報も) は除いてから持ち、`/stream/` の応答の `icy-*:`・`x-audiocast-*:` にも除いてから書く (HTTP Push や設定の変更など、ほかの経路から入った値のため)。同じ文字列を 1 行ずつ書くプレイリスト (`PlayList::add_url` の URL と題名) でも除く。bvt の `stream_header_injection` で、名前に `%0D%0A` を入れた配信を直接と PCP の中継で見て、ヘッダーを書き足されないことを確かめる (直す前は失敗することも確かめた)。
- [x] #22 リレー一覧 (`relays.html`) の `<a href="/stream/{$this.id}{$this.ext}">` の `ext` が、PCP の `sext` (ほかから届く値) そのままです。`/../` などを入れると、リンク先を同じオリジンの管理コマンド (`/admin?cmd=...`) に変えられます。クリックは同じオリジンからの要求になるので、CSRF の判定も通ります。
  - 案: `ext` は種類から決まる固定の表 (`type_ext`) だけを使う。`sext` を使うなら英数字とドットだけの短いものに限る。
  - 済み: `chaninfo::effective_ext` (`ChanInfo::type_ext`、リレー一覧の `ext` とプレイリストの URL が使う) は、`sext` が `.` と英数字 1〜7 文字のときだけ使い、それ以外は種類の表 (`type_ext`) から決める。PCP の経路でも ini (`streamExt`) の経路でも同じところを通る。受け取った `sext` はそのまま持ち、ほかのノードへもそのまま送る。bvt の `relays_stream_ext` で、ini に `streamExt = /../../admin?cmd=stop&x=` のリレーを書いて relays.html のリンクが `.flv` になることを確かめる (直す前は失敗することも確かめた)。
- [x] #23 ShoutCast 形式の放送 (1 行目がパスワード) の判定 (`servhs::request_kind` の `line.starts_with(password)`) が、パスワードの締め出し (`auth_lockout` / `auth_record`) を通っていません。当たれば `OK2`、外れれば 400 と応答で分かるので、管理パスワードを締め出されずに総当たりできます。
  - 案: この形の行も、localhost 以外からは締め出しの判定と記録を通す (外れたことを数える)。
  - 済み: `handshake_http` で、パスワードが設定されていて localhost 以外からのとき、1 行目が ShoutCast の形 (パスワードで始まる) か、どれにも当たらない行 (`Bad`) なら、締め出しを先に判定する (締め出している間は当たっても 429)。`Bad` の行は外れとして数える。ただし HEAD・OPTIONS など受け付けないメソッドの HTTP の要求 (`servhs::is_other_http_method`) は数えない (パスワードがこれらのメソッド名と空白で始まるときだけ、数えられずに試せる余地が残る)。当たったことは、これまでどおり `handshake_icy` で記録する。bvt の `shoutcast_password_lockout` で、このマシンのループバックでないアドレスからつなぎ、HEAD は数えないこと、5 回外すと正しいパスワードでも 429 になることを確かめる (直す前は失敗することも確かめた)。
- [x] #24 PCP の PUSH (`pcpstream.rs` の `push` → `servent::init_giv`) で、届いた宛先 (ループバックや LAN のアドレスも含む) へ、数の上限なく接続とスレッドを作ります。サーバント (`ServMgr::alloc_servent`) の数にも上限がありません。中継の相手や CIN の相手なら誰でも送れます。
  - 案: GIV のための接続は同時に動かす数に上限を設け、ループバック・プライベート・リンクローカルなどの宛先は断る。サーバントの数にも上限を設ける。
  - 済み: GIV のためにつなぎに行き、相手の要求を読み終えるまでのものは、同時に全体で 8 つ、宛先の IP アドレスごとに 2 つまでにした (`servent::acquire_giv`。札は要求を読み終えるか接続が切れたら返す。要求を読み終えるまでの期限 `handshakeTimeout` もかける)。つなぐのは GIV のスレッドの中で行い、PUSH を受け取った PCP の接続を待たせない。宛先のポートが 0 のもの、空・マルチキャスト・0.0.0.0/8・240.0.0.0/4 は断る。ループバック・プライベート・リンクローカル・自分のアドレスは、PUSH を届けた接続の相手 (`PcpStream::peer`) も LAN の中のときだけつなぐ (LAN の中だけで使う場合のため) (`servent::giv_dest_allowed`)。サーバントは空いたものを使い回すので、同時に動く数は、受け付ける接続 (`maxServIn`。ループバックは除く)、GIV (上の 8)、中継 (`maxRelays` など) で抑えられる。`alloc_servent` 自体には上限を設けていない。bvt の `push_giv_limit` で、CIN から BCST で PUSH を 13 回送って 8 本 (127.0.0.1 へは 2 本) しかつながらないこと、切れればまた受け付けることを確かめる (直す前は 13 本つながって失敗することも確かめた)。宛先を断るほうは単体テスト (`giv_dest`) で確かめた。
- [x] #25 認証なしの `/stream/`・`/channel/`・`/pls/` に `?ip=` や `?tip=` を付けると、ヒットやトラッカーを足せます (`ServMgr::proc_connect_args`)。そのたびに名前解決も走ります。また公開ディレクトリを有効にしていると、`/public/play.html?id=` から認証なしで任意のチャンネルの中継を始めさせられます (`public.rs` の `get_channel(.., true)`)。
  - 済み: `ip=` / `tip=` は private な接続 (localhost を含む) と `auth` トークンのある要求だけで受け付け (`proc_connect_args` の `hints`)、IP アドレス (ポート付きも) だけを読んで名前は引かない (`Host::from_str_addr`)。`/html/` の再生ページは認証済みなのでこれまでどおり。公開ディレクトリの再生ページは中継を始めず、自分が配信しているチャンネルだけを出す (一覧と同じ)。bvt の `connect_args_untrusted` で確かめる。
- [ ] #26 PCP の root atom (ホスト情報の更新間隔 `uint`、ルートのメッセージなど) を、YP でなくどのノードから届いても受け付けます (`pcp/mod.rs` の `read_root_atoms`)。プロトコルの作りによるもの。
  - 案: root atom は YP (rootHost) への COUT で受け取ったものだけ使い、更新間隔には下限と上限を設ける。
- [ ] #27 (未確認) `enableSSLServer` が有効なとき、TLS のハンドシェイク (`tls.rs` の `SSL_accept`) の間は、要求を読み終えるまでの期限が十分には効いていないかもしれません (少しずつ送る接続で居座れる)。
  - 案: TLS のハンドシェイクにも期限を設ける。確かめてから決める。
- [ ] #28 パスワードの比較 (`handshake_auth` の `sent_pass == password` など) が定数時間ではありません。締め出しがあるので影響は小さい。
  - 案: 定数時間で比べる関数を使う。

まだ深くは見ていないところ: メディアのパーサー (FLV、MKV、OGG、MP4)、XML と JSON のパーサー、uptest、正規表現、ini の読み書き。
