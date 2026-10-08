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
- [x] #26 PCP の root atom (ホスト情報の更新間隔 `uint`、ルートのメッセージなど) を、YP でなくどのノードから届いても受け付けます (`pcp/mod.rs` の `read_root_atoms`)。プロトコルの作りによるもの。
  - 案: root atom は YP (rootHost) への COUT で受け取ったものだけ使い、更新間隔には下限と上限を設ける。
  - 済み: root atom は、rootHost (YP) につなぎに行った COUT (`best.yp`。ハンドシェイクで自分のアドレスなどを信じるのと同じ条件) で受け取ったものだけ使う (`PcpStream::from_root`、`pcp::Host::root_trusted`)。ほかの接続 (CIN、トラッカーへの COUT、中継) で届いたものは読み飛ばす (更新間隔、ルートのメッセージ、新しい版の URL、`next` による NOROOT の判定、トラッカーの更新の要求のどれも使わない)。BCST の中に入っていたときも使わないが、中継はこれまでどおりする。YP から届いた更新間隔も 30〜3600 秒に収める。単体テスト (`root_atoms`) と、bvt の `root_atoms_from_yp_only` (rootHost を偽の YP にして、COUT で届いたものは使い、CIN からそのままと BCST の中で送ったものは使わないことを確かめる。直す前は失敗することも確かめた) で確かめる。
- [x] #27 `enableSSLServer` が有効なとき、TLS のハンドシェイク (`tls.rs` の `SSL_accept`) の間は、要求を読み終えるまでの期限が十分には効いていません (少しずつ送る接続で居座れる)。
  - 案: TLS のハンドシェイクにも期限を設ける。確かめてから決める。
  - 済み: develop-rs-tls の cb03da5 のうち、期限の部分を持ってきた。要求を読み終えるまでの期限 (`handshakeTimeout`) のある間は、ソケットをノンブロッキングにし、`SSL_accept`・`SSL_read` が読み書きを待つたびに、期限までの残りだけ `poll` で待つ (`tls::Session::run`、`socket::with_deadline`)。これまでは OpenSSL の中の recv のたびに読む待ち時間をまるごと使えたので、ハンドシェイクや要求のレコードを 1 バイトずつ送る接続に居座られていた。SSL_accept の失敗のログに OpenSSL の理由を出すこと、TLS の read_upto を受信量の統計に数えることも一緒に持ってきた。bvt の `tls_slow_client` で、平文・ハンドシェイク・ハンドシェイクのあとの要求を 1 バイトずつ送り、期限で切られることを確かめる (直す前は失敗することも確かめた)。
- [x] #28 パスワードの比較 (`handshake_auth` の `sent_pass == password` など) が定数時間ではありません。締め出しがあるので影響は小さい。
  - 案: 定数時間で比べる関数を使う。
  - 済み: 長さが同じならどこで違っても同じだけ時間をかけて比べる `strutil::ct_eq` と `strutil::ct_starts_with` を作り (結果は `black_box` を通して、途中で打ち切る最適化をさせない)、管理パスワード (`?pass=` と Basic 認証)、ShoutCast の 1 行目 (`servhs::request_kind`)、ShoutCast・Icecast の放送のパスワード (`servhs::icy_password_ok`)、ログインの Cookie の ID (`CookieList::contains`)、`?auth=` のトークン (`valid_auth_token`・`flv_valid_auth_token`) の比較に使う。長さが違うことは分かってしまう (パスワードの長さは隠さない)。rtmp-server のストリームキーは前から `same_bytes` で定数時間に比べている。単体テスト (`ct_eq_cases`) と、これまでの bvt (`pass_in_query`・`shoutcast_password_lockout`・`sources` など) で確かめた。時間の差を測っての確認はしていない。

## 2026-10-01 に見つけたもの (残っていたところの見直し、重さの順)

前の見直しで「まだ深くは見ていないところ」としていた、メディアのパーサー (FLV、MKV、OGG、MP4、MP3)、XML と JSON のパーサー、uptest、正規表現、ini の読み書きを見た。コードを読んで判断したもので、実際に動かしての確認はまだしていない。

- [x] #29 速度測定 (`directory.rs` の `UptestEndpoint::take_speedtest` → `post_random_data`) が、yp4g.xml に書かれた `uptest_srv` の `addr`・`port`・`object` をそのまま POST の宛先とパスに使っています。宛先に制限がなく (ループバックや LAN も可)、値の改行も除かず、`Http::send_request` も要求の行やヘッダーの値の CR/LF を確かめずに書くので、要求を書き足せます。localhost からの要求は管理画面で認証なしに通るため、yp4g.xml を書き換えられる者が、管理者が「速度測定」を押したときに、このノード自身へ管理の要求を送らせられるおそれがあります。既定の登録先 (`http://bayonet.ddo.jp/sp/yp4g.xml`) は平文の HTTP なので、通り道で書き換えられます。あわせて、`post_size` に上限がなく (値 × 1000 バイトを一度に確保する)、大きな値でメモリを確保できずにプロセスごと落ちます。
  - 案: `addr`・`port`・`object` は、空白・制御文字・`"` などがあれば断り、`port` は 1〜65535 の数だけにする。宛先がループバック・プライベート・リンクローカル・自分のアドレスなら断る (yp4g.xml を取ったホストと同じものに限ることも考える)。`post_size` に上限を設ける (正当な値は 250 なので、例えば 10 MB まで)。さらに、出ていく HTTP の要求を書くところ (`Http::send_request`) で、メソッド・パス・ヘッダーの名前と値に CR・LF・NUL があれば送らずに誤りにする (ほかの外向きの要求もまとめて守る)。
  - ほか: 登録先の一覧のロックを持ったまま yp4g.xml を取りに行き POST もする (`update`・`force_update`・`take_speedtest`) ので、相手が遅いと、その間は設定画面の状態の取得 (`getState` の `uptestServiceRegistry`) なども待たされる。あわせて直すなら、ロックの外で通信する。
  - 済み: yp4g.xml の `addr` はホスト名か IPv4 アドレスに使う文字 (英数字・`.`・`-`) だけか、IPv6 アドレス (`[]` で囲んでも囲まなくてもよい。スコープの `%` は不可。名前は引かずに使い、Host ヘッダーには `[]` で囲んで書く)、`port` は 1〜65535 の数だけ、`object` は `/` で始まり、空白・制御文字・`"` `<` `>` `\` `^` `` ` `` `{` `|` `}` `#` を含まない 1024 バイトまでのものだけにし、ほかは送らずに誤りにする (`uptest::check_srv`)。宛先は、名前を引いた結果が空・マルチキャストなら断り、ループバック・プライベート・リンクローカル・自分のアドレスには、yp4g.xml を取ったのと同じアドレスのときだけ送る (`directory::uptest_dest_allowed`。LAN の中の YP で、同じホストに測定のサーバーを置く場合のため)。`post_size` は 10000 (10 MB) までにし、超えたら送らずに誤りにする (`uptest::post_size`)。POST の要求は、URL を組み立てて読み直さず、確かめた値をそのまま使う。`Http::send_request` は、メソッド・パス (クエリも)・版・ヘッダーの名前と値に CR・LF・NUL (名前は `:` も) があれば、何も書かずに誤りにする (ほかの外向きの要求もまとめて守る)。あわせて、yp4g.xml の取得と POST は登録先の一覧のロックの外で行い、結果は URL で探して書き戻す (待つ間に消されていたら捨てる)。単体テスト (`srv_checks`・`post_sizes`・`uptest_dest`・`send_request_rejects_crlf`) と、bvt の `uptest_srv_checked` (偽の速度測定のサーバーで、正当な値なら送ること、`object`・`addr` の改行で要求を書き足せないこと、127.0.0.2 へは送らないこと、IPv6 の addr は [::1] から取った yp4g.xml なら ::1 へ送れて 127.0.0.1 から取ったものでは送らないこと、`post_size` が大きくても落ちないことを確かめる。直す前は失敗することも確かめた) で確かめる。
- [x] #30 `/admin?cmd=speedtest_cached_xml` が、外から取った yp4g.xml を `Content-Type: application/xml` で、管理画面と同じオリジンからそのまま返しています (speedtest.html にリンクがある)。XML の中の XHTML の名前空間の `script` 要素はブラウザーで動くので、yp4g.xml を書き換えられる者 (#29 と同じく既定の登録先は平文の HTTP) が、管理者がキャッシュの XML を開いたときに、管理画面のオリジンで JavaScript を動かせるおそれがあります。
  - 案: `text/plain; charset=utf-8` で返し、`X-Content-Type-Options: nosniff` と `Content-Security-Policy: sandbox` を付ける (#20 の `/stream/` と同じ)。
  - 済み: 案のとおり、`text/plain; charset=utf-8` で返し、`X-Content-Type-Options: nosniff` と `Content-Security-Policy: sandbox` を付けた (`servent_http.rs` の `speedtest_cached_xml`)。中身はそのまま文字として見える。bvt の `speedtest_cached_xml_as_text` で、XHTML の `script` を入れた yp4g.xml をキャッシュさせて、ヘッダーを確かめる (直す前は失敗することも確かめた)。ブラウザーで開いての確認はしていない。
- [x] #31 `/admin?cmd=viewxml` (index.html にリンクがある) と、`chanLog` を設定したときに書くチャンネルの記録 (`pcpstream.rs` の `chan_end`) の XML で、ほかのノードから届く値のエスケープが不完全です。`Content-Type` は `text/xml`。
  - (a) `channel` の `type` 属性 (`ChanInfo::channel_xml` の `content_type`) は、PCP の `type` で届いた文字列 (制御文字を除いただけ) をエスケープせずに書いている。`"`・`<`・`>` を入れられる。
  - (b) ほかの属性 (名前・ジャンル・説明・URL・コメント・曲の情報) は `StrType::UnicodeSafe` (`pcstring::unknown_to_unicode` の `safe`) で `&` `"` `'` `<` `>` を実体参照にしているが、UTF-8 の先頭バイトに見えるバイト (2 バイト目が 0x80〜0xBF) のあとは、1 バイト目の上位の 1 の数だけ、続きのバイトかを確かめずにそのまま出すので、そこに入った `"` や `<` はエスケープされない (C++ 版と同じ)。
  - XML の構造 (属性や要素) を書き換えたり、`XmlNode::write` の属性の解析を失敗させて viewxml を出せなくしたりできる。viewxml を読むほかのツールに偽の値を渡せる。管理者がブラウザーで開いたときに script まで動かせるかは確かめていない ((b) は不正な UTF-8 になるのでブラウザーでは読めなくなるが、(a) は正しい UTF-8 のまま入れられる)。
  - 案: `XmlNode` を、名前と生の値の組で持ち、書き出すときに値を XML のエスケープ (`&` `<` `>` `"` `'` と制御文字) をするように変える (`channel_xml`・`track_xml` は `UnicodeSafe` でなく `Unicode` で変換して渡す)。`unknown_to_unicode` も、続くバイトがすべて 0x80〜0xBF のときだけ UTF-8 の 1 文字とみなし、そうでなければ先頭バイトを 1 バイトの文字として扱う。PCP の `type` は受け取るときに英数字だけ (短いもの) に限ることも考える。
  - 済み: `XmlNode` は、タグ名と、属性の名前と生の値の組で持つようにし (`XmlNode::new("channel").attr("name", ..)`)、書き出すときに値の `&` `<` `>` `"` `'` を実体参照にし、制御文字 (0x00〜0x1f と 0x7f) を除き、正しい UTF-8 でないところは U+FFFD にする。書式付きの文字列を組み立てて解析し直すことはやめたので、値しだいで書き出せなくなることもない。`channel_xml`・`track_xml` は `UnicodeSafe` でなく `Unicode` で変換して渡し、`type` もそのまま渡してエスケープさせる。viewxml と `chanLog` のどちらもここを通る。`unknown_to_unicode` は、先頭バイトが 2〜4 バイトの文字のもので、続くバイトがすべてあって 0x80〜0xBF のときだけ UTF-8 の 1 文字とみなし、そうでなければ Shift_JIS などとして読む (`pcstring::utf8_seq_len`。`UnicodeSafe` のほか、JSON などに使う `Unicode` の変換もここを通る)。PCP の `type` を受け取るときに限ることはしていない (書き出すところで守る)。単体テスト (`values_are_escaped`・`unknown_to_unicode_checks_continuation_bytes`) と、bvt の `viewxml_escapes_values` (ini の `contentType`・名前・ジャンルに `"` や `<` を入れたリレーで、viewxml に属性や要素を書き足せないことを確かめる。直す前は `type="FLV" evil="<script>"` となって失敗することも確かめた) で確かめる。(b) の不正な UTF-8 は ini で入れられないので単体テストだけ。ブラウザーで開いての確認はしていない。
- [x] #32 メディアのパーサーが、配信元の入力しだいで大きなメモリを使います。MKV は 1 つの要素を 256 MB (`mkv::MAX_SIZE`) までまるごと読み、Cluster はさらに `send_cluster` の中で要素ごとに複製するので、1 本の配信で数百 MB になりうる。ヘッダーに入る要素 (Cluster より前) も、`MAX_DATALEN` を超えて送れないと分かっていても最後まで読む。MP4 はボックスを 64 MB まで、FLV はタグを 16 MB まで受け付ける (先に長さ分のバッファを作る)。配信元は管理者が選ぶもの (HTTP Push は LAN からだけ、ICY はパスワード、URL は管理者が入力) なので影響は小さいが、URL の配信元が平文の HTTP なら通り道で書き換えられる。
  - 案: MKV の Cluster より前の要素は、ヘッダーに入りきらない長さなら読まずに誤りにする。Cluster と MP4 のボックスの上限を下げる (例えば 16 MB)。`send_cluster` は複製せずに区切りの位置だけで送る。
  - 済み: MKV の Cluster より前の要素 (EBML・Info・Tracks など) は、ID とサイズを読んだところで、ヘッダーに足すと `MAX_DATALEN` を超えるなら中身を読まずに誤りにする (`mkv::read_head_element`)。`mkv::MAX_SIZE` を 16 MB に下げた (ヘッダーの要素は `MAX_DATALEN` までなので、実際には Cluster の上限)。`send_cluster` は、要素を複製して溜めずに、送っていない部分の始まりの位置だけを覚えて Cluster の一部をそのまま送る (`MemStream::read_slice`)。Cluster は、ID とサイズのあとに中身を届いた分だけ足して読み、つなぐときの複製もやめた。MP4 は `MAX_BOX_SIZE` を 16 MB に下げ、ftyp と moov は `MAX_DATALEN` を超えるなら中身を読まずに誤りにし、moof のあとに mdat を同じバッファに続けて読む。MP4 のボックスと FLV のタグは、先に長さ分のバッファを作らず、4096 バイトずつ届いた分だけ確保する (`media::read_into_vec`。足りない分を 0 で埋めるのは前と同じ)。FLV のタグの上限 (サイズの欄が 24 ビットなので 16 MB) はそのまま。単体テスト (`mkv::header_too_big_is_not_read`・`mkv::huge_element_rejected`・`mp4::bad_boxes`) と、既存のパケットの分け方のテスト (`header_and_clusters` など、送るパケットは変わらない) で確かめる。メモリの使用量を測ってはいない。

見て、問題がなかったもの:

- JSON のパーサー (`json::parse`): ネットワークから受け取るもの (JSON-RPC の要求、portcheck の応答) は、解析の前に入れ子の深さを確かめている (#15)。ほかはこのノードが書いたファイルと内部の状態だけ。
- XML のパーサー (`xml::read`): 使うのは uptest の yp4g.xml だけで、タグ 1 つ・内容 1 つの長さに上限があり (100 KB)、応答の本体も 32 MB まで。木は再帰せずにたどる。
- 正規表現 (`server/regex.rs`): パターンはコードの定数と同梱のテンプレート (`navbar.html` の `request.path =~ "..."`) だけで、外から指定できない。照合は明示的なスタックで行う。相手のエージェント名や IP アドレスの文字列への照合も、パターンが単純なので時間はかからない。
- ini の読み書き (`server/ini.rs`): 書くときに値の制御文字を除き、1 行 255 バイトに収まるように切るので、ネットワークから受け取った名前などで行を書き足せない。読むときも 255 バイトを超えた分は捨てる。`sourceURL` (`pipe:` などで起動しうる) は管理者が入力した URL (`Channel::start_url`) だけが入る。
- AMF0 (FLV のメタデータ): 入れ子の深さと値の数に上限がある。
- OGG・MP3: 長さはどれも `MAX_DATALEN` などで確かめている。時刻の計算で大きな値や NaN になっても、待つ時間は 60 秒までに収まる (`Channel::sleep_until`)。MP3 の ICY メタデータが 1024 バイトを超えると残りを読み捨てずにずれる (C++ 版と同じ。セキュリティの問題ではない)。

## 2026-10-01 に見つけたもの (#1〜#32 を直したあとの見直し、重さの順)

コードを読んで判断したもので、実際に動かしての確認はまだしていない。

確かめ方について: #33 を WSL で動かして確かめようとしたところ、「認証を回り込んで外部のプログラムを起動するまで」を一続きにした手順とスクリプトを書く段階で、Claude の安全の仕組みに止められた。ローカルで試すこと自体が禁止されているのではなく、そのまま攻撃の手順として使える形のものを書き出すのが止められたと考えている。このため、確かめるときは挙動を分けて、それぞれを bvt のテストにする (例: #33 は「ループバックへのリダイレクトを追うか」と「その要求を管理画面が localhost として認証なしで通すか」を別々に、害のない管理コマンドで確かめる)。直したあとも同じテストで、直す前は失敗し、直したあとは通ることを確かめる。

- [x] #33 外から取った URL のリダイレクト先やプレイリストの中身で、このノード自身の管理画面に要求を送らされます (重)。
  - `server/http.rs` の `http::get` (YP のチャンネル一覧 index.txt の取得、コンソールの `get`) と、`sources.rs` の `stream_url` (配信元の URL のリダイレクト先、HTTP で取ったプレイリストの中身) は、宛先がループバックや LAN でもそのままつなぎに行く。`stream_url` の `is_remote_safe_source` はスキームしか見ていない。
  - 管理画面 (`handshake_auth`) は、ループバックからの接続で Host がループバックの名前か IP アドレスなら認証を省く。そのため、取得先 (または通り道) を握った者が、管理コマンドを認証なしで実行させられる。`cmd=fetch` は `pipe:` で外部のプログラムを起こせるので、そこまで届きうる。
  - 既定のフィード `http://yp.pcgw.pgw.jp/index.txt` は平文の HTTP で、5 分ごとに自動で取りに行く。要求に `host=localhost:<port>` を付けているので、ポート番号も相手に分かる。
  - 案: `http::get` と `stream_url` のリダイレクト先・プレイリストの中身 (管理者が入力したものでない URL) は、`bbs_http.rs` の `is_public` と同じく、名前を引いた結果が公開アドレスのときだけつなぐ (LAN の中の配信元を使う場合のため、もとの URL と同じホストなら許すことも考える)。あわせて、管理画面が localhost を信じる条件に、このノード自身が送った要求でないこと (例: User-Agent が PeerCast でないこと) を足すことも考える。
  - 済み: 管理者が入力したのでない URL (`http::get` のリダイレクト先、`stream_url` のリダイレクト先と HTTP で取ったプレイリストの中身) は、名前を引いたアドレスが公開アドレス (`bbs_http::is_public`) か、管理者が入力した URL と同じアドレスのときだけつなぐ (`server/http.rs` の `allowed_untrusted_ip`)。確かめたアドレスにそのままつなぐので、引き直しで変えられない。同じアドレスなら許すので、LAN の中の配信元が同じホストの別のポートへリダイレクトするのはこれまでどおり使える。管理画面 (`handshake_auth` と admin.cgi) は、User-Agent がこのノードのもの (`PCX_AGENT`) なら localhost として認証を省かない (`servent_http.rs` の `trust_localhost`)。bvt の `no_fetch_into_internal` (コンソールの `get`・配信元のリダイレクト・プレイリストで、127.0.0.2 の待ち受けにつながないこと、同じホストへのリダイレクトは追うこと) と `admin_distrusts_own_agent` で確かめる。直す前は、4 つのケースがそれぞれ失敗することも確かめた。`rtmp://` のリダイレクト先は確かめていない (HTTP の要求にならないので管理画面には届かない)。
- [x] #34 ほかのサイトのページから、localhost の利用者のノードに配信や中継を始めさせられます (CSRF と DNS リバインディング)。
  - 次のものは、private (localhost を含む) からなら、Origin・Sec-Fetch-Site も Host ヘッダーも見ずに受け付ける。
    - `POST /` (HTTP Push、`handshake_http_push`): 送った本体をそのまま配信できる。既定の rootHost は実在の YP (yp.pcgw.pgw.jp:7146) なので、利用者の IP で YP に載る。
    - `/stream/`・`/pls/` の `?ip=` / `?tip=`: 任意の宛先 (LAN を含む) からの中継を始めさせられる。
    - `/html/…/play.html?id=` (`handshake_auth` を `reject_cross_origin = false` で呼ぶ): `id` に `%3Fip%3D…` を入れると、同じく中継を始めさせられる。
  - 案: 配信ソフトやプレイヤーは Origin などを付けないので、`is_cross_origin_request` に当たるものは断る (`/stream/`・`/pls/` は、配信中のチャンネルを返すのはそのままにし、中継を始めることと `ip=`/`tip=` だけを断る)。localhost として信じるときは、`handshake_auth` と同じく Host がループバックの名前か IP アドレスであることも確かめる。
  - 済み: private からの要求を信じるのは、Host が手元の名前で、ほかのサイトのページから送らされたものでないときだけにした (`servent_http.rs` の `trust_private`)。手元の名前は、ループバックの名前と IP アドレスのほかに、ドットのない名前と `.local`・`.lan`・`.home.arpa`・`.internal` で終わる名前 (`http::is_lan_host_header`。LAN のほかの PC から名前で配信・視聴する場合のため。こうした名前はほかのサイトが握れない)。ほかのサイトからかは `is_cross_origin_request` (Sec-Fetch-Site、なければ Origin) で見る。ただし、利用者がリンクを押して開いたもの (`Sec-Fetch-Mode: navigate` と `Sec-Fetch-User: ?1`、`http::is_user_navigation`) は、YP のサイトの再生のリンク (`/pls/…?tip=`) が使えるように信じる (`peercast://` のリンクと同じ程度)。HTTP Push はこの例外なしで、当たれば 403。`/stream/` (と `/channel/`) は、チャンネルを探す前にヘッダーを読むようにし (`servent::read_stream_headers`)、当たれば中継を始めず `ip=`/`tip=` も使わない (配信中のチャンネルはそのまま返す)。`/pls/` も同じ。play.html は、ほかのサイトからなら (リンクを押したものは除く) 中継を始めず `id` の `?ip=` も使わない (認証は前のとおり `handshake_auth`)。ヘッダーのない要求 (配信ソフトやプレイヤー) と古いブラウザーはこれまでどおり。bvt の `cross_site_cannot_start_relay` (`/stream/`・`/pls/`・play.html で、Sec-Fetch-Site・Origin・Host がほかのドメインのときは中継を始めないこと、ヘッダーなし・same-origin・`.local` の Host・リンクを押したものは始めること、HTTP Push はほかのサイトからのもの (フォームの送信も) と Host がほかのドメインのものは 403 で、ヘッダーのないものは受け付けること) と単体テストで確かめる。直す前は、中継を始めないはずの 6 つのケースがすべて中継を始め、HTTP Push も受け付けて失敗することも確かめた。ブラウザーで開いての確認はしていない。
- [x] #35 PCP の接続なら、どれからでもこのノードのチャンネルのストリームにパケットを書けます。
  - `pcpstream.rs` の `chan_packet` は、届いた接続がそのチャンネルの中継元 (上流) かどうかを見ていない。下流の中継先や CIN からでも、自分が配信しているチャンネルを含めてデータを書き込め、`pos=0` の head でストリームを入れ替えられる。チャンネルの情報の更新 (`chan_end` の `update_info`) も上流以外から受け付ける。C++ 版と同じ。
  - 案: `PcpStream` に、どのチャンネルの上流か (`Option<チャンネル ID>`) を持たせ、`pkt` と情報の更新はそのチャンネルについてだけ受け付ける。ほかの接続で届いた `pkt` は読み飛ばす。
  - 済み: `PcpStream` に、どのチャンネルの上流への接続か (`upstream_of`) を持たせた。設定するのは中継の配信元 (`SourceStream::create`) だけ。`pkt` はそのチャンネルについてだけ書き込み、ほかの接続 (下流の中継先、CIN、COUT、ほかのチャンネルの上流) で届いたものは読み飛ばす (切断はしない)。チャンネルの情報の更新 (`update_info`) も同じ。ヒットリストの情報の更新はこれまでどおり (YP やトラッカーから届くため)。bvt の `pcp_data_from_upstream_only` (配信しているノードへの CIN と、中継しているノードの下流から、`pos=0` の head と名前の更新を送っても、ストリームと名前が変わらないこと) で確かめる。直す前は、配信側と中継側のそれぞれで失敗することも確かめた。
- [x] #36 引数のない GIV で、YP への接続 (COUT) を乗っ取れます。
  - 誰でも `GIV` を送れば、そのソケットが COUT の `push_sock` に置かれ (`servmgr::accept_giv`)、COUT が次につなぎ直すとき、YP の代わりにそのソケットが使われる。トラッカーの更新 (`tracker_update_atom`) には放送 ID (BCID) が入るので相手に渡り、BCID があればどのチャンネルの `?auth=` も作れる。相手がつないだままにすれば、その間 YP にチャンネルが載らない。C++ 版と同じ。
  - 案: 引数のない GIV は、こちらが PUSH を頼んだ直後だけ受け付ける (置いたソケットにも期限を付ける)。BCID は YP (`best.yp`) への COUT のときだけ送る。
  - 済み: 引数のない GIV への PUSH は、COUT を断った YP (やトラッカー) がほかのトラッカーに頼むもので、こちらからは誰が来るか分からない。そこで、COUT が相手から QUIT を受けてから 30 秒の間 (`servent.rs` の `GIV_WINDOW`) だけ受け付け、それ以外は 503 にする。置いたソケットも、期限から 30 秒を過ぎたら使わずに閉じる。1 つ使ったら、次に QUIT を受けるまで受け付けない。放送 ID は、トラッカーの更新でも helo と同じく YP (`rootHost`) への COUT (`Servent::to_yp`) にだけ送り、ヒットリストから選んだトラッカーや GIV で来た相手には送らない (C++ 版はどの COUT にも送る)。bvt の `giv_cout_only_after_quit` (YP につないでいる間の GIV は 503、YP への helo とトラッカーの更新には放送 ID があること、YP が QUIT を送ったあとの GIV では COUT を始め、helo にもトラッカーの更新にも放送 ID がないこと、そのあとの GIV は 503) で確かめる。直す前は、GIV を受け付けるところと、GIV の相手への更新に放送 ID を入れるところでそれぞれ失敗することも確かめた。QUIT のあとの 30 秒の間は、まだ誰でも GIV で COUT を握れる (放送 ID は渡らないが、つないでいる間は YP に載らない)。チャンネル ID の付いた GIV (`Channel::accept_giv`) にも同じ問題があり、#41 に残す。
- [x] #41 チャンネル ID の付いた GIV で、中継の配信元を乗っ取れます。
  - 誰でも `GIV /<チャンネル ID>` を送れば、そのソケットがチャンネルの `push_sock` に置かれ (`Channel::accept_giv`)、中継しているチャンネルが次に配信元を探すとき (つなぎ直しを待っている間ならすぐに) それが配信元に使われる。#35 で `pkt` は上流からだけ受け付けるようにしたが、GIV で来た相手は上流になるので、偽のストリームを流せる。C++ 版と同じ。
  - 案: #36 と同じく、こちらが PUSH を頼んだ (配信元から QUIT を受けた) 直後だけ受け付け、置いたソケットにも期限を付ける。あわせて、受け付けるのは中継しているチャンネルだけにする (配信しているチャンネルは `push_sock` を使わない)。
  - 済み: チャンネル ID の付いた GIV は、中継しているチャンネル (`src_type` が `SRC_PEERCAST`) で、配信元から QUIT を受けてから 30 秒の間 (`channel::GIV_WINDOW`、`Channel::giv_until`) だけ受け付け、それ以外は 503 にする (満員の配信元は 503 のあとに PCP でほかのホストと QUIT を送り、ほかのノードにこちらへの PUSH を頼むことがある)。置いたソケットも、期限から 30 秒を過ぎたら使わずに閉じる。1 つ使ったら、次に QUIT を受けるまで受け付けない。bvt の `giv_channel_only_after_refusal` (配信しているチャンネルと、配信元から受け取っている間の中継への GIV は 503、満員の配信元に断られたあとの GIV は受け付けてそのソケットで `GET /channel/` を送ること、そのあとの GIV は 503) で確かめる。直す前は、配信しているチャンネルへの GIV を受け付けて失敗することも確かめた。QUIT のあとの 30 秒の間は、まだ誰でも GIV でそのチャンネルの配信元になれる (#36 と同じ。#35 により、そのあいだ偽のストリームを流せる)。
- [x] #37 ヒットリストとヒットの数に上限がありません。
  - PCP の相手は、チャンネル ID やアドレスを変えていくらでもヒットリスト (`chanmgr::add_hit_list`、`add_hit`) とヒットを足せる。足すたびに一覧を頭から探すので、メモリも CPU も使い、ロックも長く持つ。C++ 版と同じ。
  - 案: ヒットリストの数と、1 つのリストのヒットの数に上限を設ける (超えたら古いものから捨てるか、足さない)。
  - 済み: ヒットリストは 1000 (`chanmgr::MAX_HIT_LISTS`)、1 つのリストのヒットは 500 (`chanhit::MAX_HITS_PER_LIST`) までにした。超えるときは、ヒットリストは最後にヒットが来たのが古いもの (このノードのチャンネルのものは除く) から、ヒットは時刻の古いものから消す (同じ時刻なら後ろのもの)。足さないのでなく古いものを消すので、相手が送り続ければ YP から届いたヒットリストも押し出される (メモリと CPU は上限で止まる)。このノードのチャンネル ID は、ヒットリストのロックの外で調べる。単体テストの `hit_lists_capped` と `hits_capped`、bvt の `hit_lists_capped` (配信しているノードへの CIN から、チャンネル ID の違うヒットを 1200 送ると、ヒットリストは 1000 で、配信しているチャンネルのものと新しいものが残ること) で確かめる。直す前は bvt が 1201 で失敗することも確かめた。
- [x] #38 管理画面をほかのサイトの枠 (iframe) に入れられます (クリックジャッキング)。
  - `html::write_ok` などの応答に `X-Frame-Options` も CSP の `frame-ancestors` もない。
  - 案: 管理画面の応答に `X-Frame-Options: DENY` と `Content-Security-Policy: frame-ancestors 'none'` を付ける。
  - 済み: `html::write_ok` (テンプレートのページ、ログインのページ、`/html/` のそのままのファイル) と、管理コマンドの誤りのページ・`cmd=redirect` のページに、`X-Frame-Options: DENY` と `Content-Security-Policy: frame-ancestors 'none'` を付けた (`html::NO_FRAME_HEADERS`)。UI は自分のページも枠に入れないので、同じオリジンも断る。公開ディレクトリ (`/public/`) と `/assets/` は操作がないので付けていない。bvt の `admin_not_framed` で確かめる。直す前は失敗することも確かめた。ブラウザーで枠に入れての確認はしていない。
- [x] #39 パスワードの締め出しを IP アドレスの単位で数えています (軽)。
  - IPv6 では /64 の中でアドレスを変えれば締め出しを逃れられる。覚えている数 (4096) が締め出し中の IP アドレスで埋まると、新しい IP アドレスは数えない (`servhs::AuthThrottle::failed`)。
  - 案: IPv6 は /64 ごとに数える。埋まったときは数えないのでなく、全体の数で締め出す。
  - 済み: 締め出しは `servhs::auth_key` のキーごとに数える。IPv6 (IPv4 射影アドレスを除く) は /64 ごと、IPv4 はアドレスごと。覚えている数が締め出し中のもので埋まったら、覚えていないキーはまとめて 1 つ (`AuthTable::rest`) として数え、設定の回数で締め出す (そのあいだは、覚えていないキーからは誰も入れない)。単体テストの `auth_key_v6_per_64` と `auth_throttle_full` で確かめる。bvt では複数の IPv6 のアドレスから試せないので、動かしての確認はしていない。
- [x] #40 `--enable-notify-send` のとき、チャンネル名やコメントをそのまま notify-send に渡しています (軽)。
  - 本文は多くの通知のデーモンでマークアップとして解釈される。
  - 案: `&` `<` `>` を実体参照にしてから渡す。
  - 済み: notify-send に渡す本文の `&` `<` `>` を実体参照にした (`server/app.rs` の `markup_escape`)。要約 (通知の種類) はマークアップにならず、こちらで決めた文字列なのでそのまま。単体テストの `markup_escape_tags` で確かめる。通知のデーモンで表示しての確認はしていない。

見て、問題がなかったもの:

- テンプレートと UI の JS: `{!...}` は `COLUMNS` とエスケープ済みのログだけ。リンクの URL は `link_url` で http(s) に限っている。コンソールの出力は文字として入れている。
- 公開ディレクトリ: URL を消毒していて、中継も始めない。
- JSON-RPC のストレージ: キーを `channelFilters` だけに限っている。
- 乱数: Cookie・セッション ID・放送 ID は /dev/urandom から作る。
- TLS のクライアント: 証明書とホスト名を確かめている。
- flv.cgi: トークンの確かめと ffmpeg の引数で、同じ `id` を使っている。
- 掲示板の取得 (`bbs_http.rs`): 公開アドレスだけにつなぎ、リダイレクト先も確かめている。
- rtmp-server: 既定でこの PC からの接続だけを受け付ける。
- ソケット: 要求を読み終えるまでの期限が効いている。

## clippy の警告 (2026-10-01、2026-10-03 に片付けた)

`cargo clippy --workspace --all-targets` (clippy 0.1.98) の warning 82 件を 0 件にした。これ以降は、変更のたびに警告が増えていないことを確かめる。

- [x] 性能に少し関わるもの
  - `accept_giv` が断ったときに返すソケット (136 バイト) を `Box` にした (`server/channel.rs`・`servent.rs`・`servmgr.rs`)。`Error` そのものは大きくない。
  - enum の大きい要素を `Box` にした (`chandir.rs` の `Line::Entry`、`server/regex.rs` の `Esc::Class`)。
  - ヒットリストの最後の一致を `rev().find()` で探すようにした (`server/servent_http.rs`)。
- [x] 間違いにつながりうるもの
  - `if` の両方の枝が同じ (`server/channel.rs` の `readStream` の誤り): どちらもログを書いて -1 にするだけなので、一つにまとめた。develop-old の C++ 版は `StreamException` だけを捕まえ、ほかの例外は後始末を飛ばして外へ抜けるが、Rust 版はどの誤りも捕まえて後始末をする (動きは変えていない)。C++ 版の問題は docs/cpp-known-issues.md に書いた。
  - 引き算の下限を `saturating_sub` にした (`chanpacket.rs`)。
  - `xml::Builder` が `Result<_, ()>` を返していた: 失敗の理由は属性の読み取りの誤りだけなので `AttrError` を返すようにし、`xml::Error::Callback` を `Attr(AttrError)` にした。`uptest.rs` の誤りの横流し (`attr_error`) はなくなった。
- [x] 書き方だけのもの
  - `cargo clippy --fix` で直したもの (`map_or` → `is_some_and` など、`repeat_n`、`io::Error::other` ほか)。
  - 型に名前を付けた (`commands::Options`、`log::Listener`・`AuxFunc`、`channel::Span`、ui-gen の `Catalog`)。
  - `bbs::post_message` の名前・メール・本文を `Message` にまとめた (引数が 8 つ)。
  - `sys::Random::next` を `next_u32` にした (`Iterator::next` と紛らわしい)。
  - rtmp-server-rs の `flv::State` の要素の名前から `Expect` を外した。
  - `server/tls.rs` の `SSL` は OpenSSL の名前に合わせているので、`clippy::upper_case_acronyms` を許した。

## 2026-10-08 に見つけたもの (#1〜#41 を直したあとの見直し、重さの順)

#42 と #43 は一時的なテスト (コミットしていない) で動かして確かめた。ほかはコードを読んで判断したもの。

- [x] #42 rtmp-server が、publish を受け付ける前のメタデータと音声・映像も出力先 (PeerCast) に流します (重)。
  - `session.rs` の `on_message` は、publish を受け付けたかを見ずに 0x12・0x08・0x09 を `FlvWriter` に書き、そこで出力先を開く。このため、ストリームキー (`rtmpStreamKey`) を設定していても、publish を送らない接続はキーを確かめられずに配信できる (配信開始までの期限で切れるが、つなぎ直せば続く)。C++ 版にはキーがないので Rust 版だけの問題。
  - 確かめたこと: キーを `secret` にした rtmp-server に、publish を送らずにメタデータと映像を送ると、出力先に `POST /?name=t` と FLV のヘッダー・映像が届いた。既存のテスト (`wrong_stream_key_is_rejected`) は publish の名前が違う場合だけを見ている。
  - 案: publish を受け付けるまでは、メタデータと音声・映像のメッセージは捨てる (か誤りにして切る)。キーを設定していないときも同じにする。robustness に「publish なしのデータは出力先に届かない」テストを足す。
  - 済み: publish を受け付けるまでに届いたメタデータ (0x12) と音声・映像 (0x08・0x09) は、出力先に書かずに誤り (`media before publish`) にして切る。キーを設定していないときも同じ。OBS や ffmpeg は publish の応答を待ってから送るので影響はない。session のテスト `media_before_publish_is_rejected` と robustness の `data_without_publish_does_not_reach_sink` で確かめる。直す前は robustness のテストが失敗することも確かめた。
- [x] #43 `randomizeBroadcastingChannelID` がオフのとき、チャンネル ID から放送 ID (BCID) を戻せます (中)。
  - チャンネル ID は、放送 ID に名前・ジャンル (ICY はマウント)・ビットレートを XOR しただけのもの (`gnuid::encode`、`set_broadcast_id_channel_id`、`handshake_icy`)。これらの値は YP の一覧や PCP で公開されているので、同じ計算をもう一度すれば放送 ID になる。放送 ID があれば、どのチャンネル ID についても `?auth=` のトークンを作れる (#36 と同じ影響)。既定はオン (ランダム) なので、オフにした人だけ。C++ 版と同じ。
  - 確かめたこと: 単体の一時的なテストで、`encode` を同じ値でもう一度かけると放送 ID に戻ることを確かめた。
  - 案: オフのときのチャンネル ID を一方向の関数 (放送 ID と名前などをまとめたもののハッシュ) で作る (同じ名前なら同じ ID になるのは保てる。ID は今と変わる)。あわせて、`auth` のトークンを放送 ID とは別の秘密から作ることも考える。
  - 済み: オフのときのチャンネル ID は、放送 ID・名前・ジャンル (ICY はマウント)・ビットレートを長さつきでつないだものの MD5 にした (`channel::derived_channel_id`。HTTP Push・管理画面・JSON-RPC の配信と ICY の両方)。同じ名前などなら同じ ID になるのは今までどおりで、ID の値は前と変わる。放送 ID が戻せなくなったので、`auth` のトークンはそのままにした。単体テスト `derived_channel_id_is_one_way` と bvt の `channel_id_hides_broadcast_id` で確かめる。直す前は bvt が失敗することも確かめた。
- [x] #44 `/cgi-bin/flv.cgi` (トランスコード) が、ほかのサイトのページから送らされた要求と DNS リバインディングを断っていません (中。トランスコードを有効にしたときだけ)。
  - private (localhost を含む) からならトークンなしで受け付け、Sec-Fetch-Site・Origin と Host を見ていない (#34 で `/stream/` などに入れた `trust_private` を通っていない)。localhost からは同時に動かす数の上限 (`maxTranscodes`) も掛からない。さらに ffmpeg が `/stream/` を localhost から取るので、#34 で断ったほかのサイトからの中継の開始が、ここを通ると起きる。
  - 案: トークンでなく private で通すときは `trust_private` を通す。localhost からも上限に数える (か別の上限を設ける)。あわせて、ffmpeg に入力の形式 (`-f`) を種類から決めて渡し、使うプロトコルを絞ることも考える。
  - 済み: トークンがないときは `is_private` でなく `trust_private` で通す (`/stream/` と同じく、利用者がリンクを押して開いたものは通す)。localhost からは `maxTranscodes` とは別に、同時に 4 つまで (`MAX_LOCAL_TRANSCODES`。設定の説明「localhost は数えない」はそのまま)。ffmpeg には `type` から決めた入力の形式を `-f` で渡し (MKV・WEBM は matroska、ほかに FLV・MP3・OGG)、知らない種類は 400 にする。bvt の `flv_cgi_cross_site` (ffmpeg がない環境では 403 かどうかだけを見る) と単体テスト `flv` で確かめる。直す前は bvt が失敗することも確かめた。localhost の上限は ffmpeg がないので動かして確かめていない。
- [x] #45 IDLE スレッドが、外部の HTTP の応答を期限なしに待ちます (中〜軽)。
  - 速度測定の yp4g.xml の取得 (`UptestRegistry::update` → `download`) は IDLE スレッドの中で行い、チャンネルフィードの取得 (`ChannelDirectory::update`) は IDLE スレッドがスレッドの終わりを待つ。読むたびの待ち時間 (30 秒) はあるが全体の期限がないので、少しずつ返す相手に止められる。既定の登録先 (`http://bayonet.ddo.jp/sp/yp4g.xml`、`http://yp.pcgw.pgw.jp/index.txt`) は平文の HTTP。
  - 止まると、配信を YP に載せる COUT (`connect_broadcaster` でしか始めない)、ヒットの掃除、rtmp-server の再起動、`cmd=shutdown`、フィードの更新なども止まる。
  - 案: 外向きの HTTP (`http::get`、uptest の `download`) に全体の期限を設ける。uptest の取得も別のスレッドで行い、IDLE スレッドは待たない。
  - 済み: チャンネルフィードと速度測定の yp4g.xml の取得を、IDLE から新しい FEEDS のスレッド (`servmgr::feed_proc`) に移した。`ClientSocket` に書いても外れない全体の期限 (`set_total_timeout`) を足し、`http::get` と速度測定の `download` は 1 回の要求を 60 秒まで (`http::FETCH_TIMEOUT_MS`) にした。bvt の `slow_feed_does_not_block_idle` (フィードの相手が 1 秒に 1 バイト返す間に `cmd=shutdown` で終わる) と単体テスト `total_timeout` で確かめる。直す前は bvt が失敗することも確かめた。
- [x] #46 通知に間隔の制限がなく、`--enable-notify-send` のときは通知ごとに notify-send を起動します (軽)。
  - 中継しているチャンネルのコメントが変わるたびに通知する (`Channel::update_info`) ので、中継元がコメントを変え続けると、1 秒に数十回 notify-send を起こす (通知のデーモンがない環境では 1 つが数十秒残る)。
  - 案: notify-send は同時に 1 つ (か数秒に 1 回) までにし、間の通知はまとめるか捨てる。コメントの変更の通知もチャンネルごとに間隔を空ける。
  - 済み: コメントが変わった通知は、同じチャンネルでは 10 秒に 1 回まで (`COMMENT_NOTIFY_INTERVAL`)。notify-send は同時に 1 つ、3 秒に 1 回まで (`app::NotifyGate`) にし、その間の通知はログと通知の一覧にだけ残す。bvt の `comment_notifications_throttled` (JSON-RPC の `setChannelInfo` でコメントを 5 回変える) と単体テスト `notify_gate` で確かめる。直す前は bvt が失敗する (通知が 5 つ) ことも確かめた。
- [ ] #47 フィルターの `.` で始まる名前は、逆引き (PTR) の結果だけで判定しています (軽)。
  - `ServFilter::matches` の `Suffix` は `dnscache::name_of` の名前の終わりを見るだけで、その名前を正引きして相手のアドレスに戻るかを確かめていない。PTR は相手のアドレスの持ち主が決められるので、private や許可に使うと、その扱いを受けられる。C++ 版と同じ。
  - 案: 逆引きした名前を正引きし、相手のアドレスが含まれるときだけ使う (forward-confirmed reverse DNS)。
- [ ] #48 RTMP の設定 (rtmp.html) は GET で送るので、ストリームキーが URL に入ります (軽)。
  - ブラウザーの履歴に残り、ログの伏せ字 (`redact_query`) も `pass`・`passnew` だけなので、デバッグのログにも残る。rtmp-server でキーをコマンドラインやログに出さないようにしたのと合わない。
  - 案: フォームを POST にし、`streamkey` も伏せる。
- [ ] #49 設定のページと JSON-RPC の `getState` に、管理パスワードが平文で入ります (軽)。
  - settings.html はパスワードの欄の `value` に `servMgr.password` を入れ、`servMgr` の状態にも `password` がある。管理画面に XSS があると読めるので、Cookie を HttpOnly にした (#16) 意図と合わない。
  - RTMP のストリームキーも同じ形 (2026-10-08 の続きで見つけた): rtmp.html の欄の `value` に `servMgr.rtmpStreamKey` を入れ、`servMgr` の状態 (`servmgr.rs` の `state`) にも `rtmpStreamKey` がある。
  - 案: 欄は空で出し、空のまま保存したら変えない。状態からは除く。ストリームキーも一緒に直す。

見て、問題がなかったもの:

- HTTP の要求の読み方 (行の長さとヘッダーの数の上限、CR の扱い)。`/html/`・`/public/`・`/assets/` のパスは、realpath のあとで文書のディレクトリの中かを確かめている。
- テンプレートの出力と UI の JS (チャンネル一覧、接続一覧、リレー一覧、掲示板、コンソール): 外から届く値は文字として入れるか `h()` を通し、リンクは http(s) に限っている。新しい版の URL (`upgradeURL`) は `http://www.peercast.org/` で始まるものしか入らない。
- PCP の atom とハンドシェイクの読み取り (入れ子と長さの上限)、パケットのバッファ。
- 掲示板の取得と書き込み (公開アドレスだけ、リダイレクト先も確かめる、書き込みは CSRF の確かめのあと)。
- TLS (証明書とホスト名の検証、ハンドシェイクの期限)、ソケットの期限、Cookie (IP と結び付け、32 個まで)。
- 外部のプログラムの起動 (引数は配列で渡し、シェルを通さない)。外向きの HTTP の User-Agent はどれも `PCX_AGENT` (#33 の判定が効く)。
- rtmp-server の AMF0 (入れ子と値の数の上限) とチャンクの保持量。
- ui/linux/scripts の URL ハンドラー (インストールされない。`\w` しか通さない)。

- #45・#46 は C++ 版と同じ (C++ 版も IDLE の中で `channelDirectory->update()` (ワーカーを `join` で待つ) と `uptestServiceRegistry->update()` を呼び、`http::get` に全体の期限がない。通知ごとに `system("notify-send … &")` を起こし、コメントが変わるたびに通知する)。docs/cpp-known-issues.md に書いた。
- #44 と #45〜#49 は動かしての確認をしていない。直すときに bvt で「直す前は失敗、直したあとは通る」を確かめる。

## 2026-10-08 に見つけたもの (続き。角度を変えた見直し、重さの順)

前の見直しが止まったので、コードを順に追うのをやめて、(1) 変異を強めた乱数のテストを一時的に書いて回し (入力ごとにパニック・処理時間・メモリの確保量を測る)、(2) 長く動くスレッド (IDLE・待ち受け) を外からの入力で止められるかという観点で読んだ。

- [x] #50 YP のチャンネル一覧 (index.txt) の行数と誤りの行の数に上限がなく、1 回の取得で数 GB のメモリを使います (中)。
  - `directory.rs` の `get_feed` は、応答の本体 (32 MB まで) を `ChannelEntry::text_to_entries` で行ごとに読み、欄が 19 の行はすべてチャンネルにし、そうでない行はすべて「Parse error at line N.」を作って 1 行ずつ `log_error!` する。その間のログは `log::capture` が上限なく溜め、最後に `feed.log` にもつなげる。一覧は getState (`ChannelDirectory::state`) でさらに複製して JSON にする。
  - 確かめたこと (一時的なテストで測った): 32 MB の最小の行 (`<>` を 18 個) で 906,876 チャンネル・最大 418 MB。32 MB の空行で誤りが 33,554,432 件になり、ログを先頭の 200 万行に絞っても最大 2,228 MB (実際は全部ログに書くので、さらに数 GB 増える。ログのファイルにも 3,300 万行書く)。
  - 一覧を書き換えられるのは、YP と、通り道 (既定のフィード `http://yp.pcgw.pgw.jp/index.txt` は平文の HTTP) の者。5 分ごとに自動で取りに行く。メモリの少ない機械 (ARM の小さいものなど) では落ちうる。C++ 版と同じ。
  - 案: チャンネルの数に上限を設ける (例えば 10,000)。誤りの行は数だけ数え、ログは最初の数行と件数だけにする。`log::capture` が溜める行数にも上限を設ける。フィードの本体の上限を下げる (例えば 4 MB)。
  - 済み: 1 つのフィードから読むチャンネルは 10,000 個まで (`MAX_FEED_CHANNELS`)。誤りの行の文言は 10 個まで書き、残りは「N more parse errors.」の 1 行にする。`log::capture` が集めるのは 1,000 行まで。本体の上限 (32 MB、取ったあとすぐ捨てる) はほかの取得と共通なのでそのままにした。単体テスト `feed_limits`・`capture_is_bounded` で確かめる。
- [ ] #51 PCP のヒットの宛先を確かめずに、中継元としてつなぎに行きます (軽)。
  - ヒット (`pcpstream.rs` の `hit` → `chanmgr::add_hit`) の `rhost` はどの相手からでも好きなアドレスにでき、中継元を探すとき (`sources.rs` の `peercast_stream` → `pick_from_hit_list` → `connect_fetch`) は、ループバック・LAN・自分のアドレスでもそのままつなぐ。`chanhit::pick` は WAN 側がこちらのグローバル IP と同じなら LAN 側 (`rhost[1]`) を選ぶので、LAN の中のアドレスも選ばせられる。送るのは決まった形の `GET /channel/<ID>` なので害は小さい。PUSH の宛先 (#24) とそろっていない。C++ 版と同じ。
  - 案: #24 の `giv_dest_allowed` と同じく、ループバック・LAN・自分のアドレスのヒットは、届けた接続の相手も LAN の中のときだけ受け付ける (か、つなぐ前に断る)。
- [ ] #52 `chanLog` を設定していると、どの PCP の相手からでも、`chan` の atom を送るたびにチャンネルの記録を書き足させられます (軽)。
  - `pcpstream.rs` の `chan_end` は、ヒットリストの情報を更新するたびに XML を 1 件 (1 KB ほど) 追記し、記録の大きさに上限がない。小さな atom で何倍もの書き込みになる。既定は空なので、設定した人だけ。C++ 版と同じ。
  - 案: 上流 (`is_upstream`) からの更新だけ記録するか、記録の大きさか頻度に上限を設ける。

乱数のテストの結果 (問題なし):

- 対象: PCP のパケットの処理 (`pcp::proc_packet`、400 万回)、PCP のハンドシェイク (`read_hello`・`read_version`、300 万回)、HTTP の要求と応答の読み取り (`server::http::Http`、各 300 万回)、チャンク転送 (`dechunk`、300 万回)、文字列の関数 (`http`・`url`・`cgi`・`strutil`・`pcstring`・`utf8`・`strtod`・`inspect`・`gnuid`・`jis`・`chaninfo`・`uptest`・`servhs`・`pcstr`・`servfilter`、200 万回)、JSON・yp4g.xml・AMF0 (各 300 万回)、index.txt (200 万回)、ini (100 万回)、掲示板 (設定・スレッドの一覧・dat の解釈と文字コード、150 万回)、メディア (FLV 30 万回、MKV・OGG・MP4 各 40 万回、MP3 20 万回)。
  - 変異は、1 バイトの書き換えなどのほかに、長さの欄に境界の値 (0x7fffffff・0x80000000・16384 など) を 16/24/32/64 ビットで入れるもの、切り貼り、繰り返し (深い入れ子) を足した。
- パニック・止まる入力はなかった。メモリの最大は、FLV のタグと MP4 のボックスの上限どおり 1 回 16 MB (#32 で決めたもの)、HTTP の応答は Content-Length の分 (32 MB まで) を先に確保する。
- ハンドシェイクで 20 秒以上返らない入力 (子の数が約 21 億の `oleh`) が出たが、テスト用の読み手が終わりで空を返すためだった。実際のソケットは相手が切ると誤り (`Closed on read`) を返すので止まらない (相手が送り続ける間だけ回る)。

見て問題がなかったもの:

- パニックの影響: スレッドはどれも `catch_panic` で包まれ、ロックは毒化を無視して取り直すので、ほかのスレッドへは広がらない。待ち受けのスレッドが落ちても SERVER のスレッドが待ち受けを始め直す。IDLE が直接読む外からの値は速度測定の yp4g.xml くらいで、乱数のテストで落ちなかった。
- `chanpacket.rs`・`server/packetbuf.rs`: 位置の計算は `wrapping`/`saturating`、添字は 64 で割った余り。
- `chanhit.rs` (数え上げ・選び方・追加): 折り返しの演算だけ。
- `pcpstream.rs`: PUSH (#24)、`pkt` と情報の更新 (#35)、root atom (#26) の制限が効いている。受け取る atom は 16 KB と入れ子 64 段まで。

止まったところ (この見直しで 3 回止まった):

- 1 回目: 単体のテストでは通らないつなぎのコード (`servent.rs`・`pcpstream.rs` など) も試すため、権限のないネットワークの名前空間 (外へは出られない) で実際のサーバーを起こし、乱数の要求を送る準備をしていた (サーバーが自分のアドレスや LAN をどう扱うかを調べていたところ)。Claude の安全の仕組みに止められたので、この方式はやめた。動いているサーバーに外から要求を送る形の確かめは、今後もしない。
- 2 回目: BCST の中継で、各接続の送る待ち行列 (64 個) が埋まると「Send too slow」で接続を切る作りなので、送り出しのループの速さしだいで、ほかの下流の接続に影響しうるかを `servent.rs` の中継のループで読んでいたところで止まった。結論は出していない (C++ 版も同じ作り)。確かめるなら、待ち行列が埋まったときの振る舞いを単体のテストで見る形にする。
- 3 回目: 2 回目の続きとして `servent.rs`・`pcpstream.rs`・`packetbuf.rs` を読んでいたところで、また止まった。結論は出さず、この件 (BCST の中継と送り待ちの列) は打ち切った。これ以上は調べない。
- まだ深く読んでいないもの: `server/chanmgr.rs` (ヒットリストの操作の残り)、`server/servmgr.rs` の設定の読み込みと IDLE・待ち受け以外の部分、`jrpc.rs` (認証のあとだけ)。→ 次の「続きの続き」で読んだ。

## 2026-10-08 に見たもの (続きの続き。残っていたところ)

前の見直しで「まだ深く読んでいないもの」とした `server/chanmgr.rs`・`server/servmgr.rs`・`jrpc.rs` と、`server/servent_http.rs` の認証のまわりを、コードで読んだ。動かしての確認はしていない。

- 見つけたもの: RTMP のストリームキーが、設定のページと `getState` に平文で入る。#49 と同じ形なので、#49 に書き足した。

見て、新しい問題は見つけていないもの:

- `server/chanmgr.rs`: チャンネルとヒットリストの一覧の操作。ヒットリストの数の上限 (#37) が効いている。
- `server/servmgr.rs`: 設定 (ini) の読み込みと書き出し、ホストのキャッシュ (100 個で固定)、`proc_connect_args` (#25)。
- `jrpc.rs`: メソッドの振り分けと引数の変換 (前半のメソッドまで)。`POST /api/1` は認証 (`handshake_auth`。ほかのサイトからの要求は断る) のあとだけで、認証なしの `GET /api/1` は `getVersionInfo` だけ。
- `server/servent_http.rs`: 管理画面・JSON-RPC・admin.cgi の認証と、ほかのサイトからの要求の判定 (`trust_localhost`・`trust_private`・`is_cross_origin_request`)。

止まったところ:

- 4 回目: 続けて `servhs.rs` (要求の振り分けと判断) を読んでいたところで、Claude の安全の仕組みに止められた。結論は出していない。この件もここで打ち切った。

## 2026-10-08 に見つけたもの (守りの突き合わせ、重さの順)

4 回止まったので、「どう攻めるか」を順に追う読み方をやめ、(1) 入口の種類ごとに、これまでに入れた守り (`trust_private`・`trust_localhost`・締め出し・`ct_eq`・`allowed_untrusted_ip`・`strip_controls` など) が同じように付いているかを grep で突き合わせ、(2) `unsafe` と FFI、(3) 依存するクレートを、一覧にして確かめた。コードと librtmp の説明書き (`man 3 librtmp`) とヘッダーを読んで判断したもので、動かしての確認はしていない。この見直しでは止まらなかった。

- [x] #53 未確認の URL (配信元のリダイレクト先、HTTP で取ったプレイリストの中身) の `rtmp://` を、librtmp のオプションごと渡しています (中〜重。`rtmp` の機能つきのビルド (Makefile の既定) で、URL の配信元を使うとき)。
  - `url::is_remote_safe_source` は `rtmp://` を通し、`sources.rs` の `stream_url` は、#33 の宛先の確かめ (`allowed_untrusted_ip`) をせずに URL (255 バイトまで) をそのまま `RtmpStream::open` → `RTMP_SetupURL` に渡す。`Location:` の値も、プレイリストの行も、途中の空白を残している。
  - librtmp は URL の空白の後ろをオプションとして読む。説明書きにあるものだけでも、`socks=ホスト:ポート` (そこを通してつなぐ) と `swfUrl=… swfVfy=1` (その URL から SWF を HTTP で取り、`$HOME/.swfinfo` に書く) があり、どちらの宛先も #33 の確かめを通らない。`rtmp://` のホスト自身も確かめていない。librtmp は 2015 年のスナップショットのまま保守されていない C のライブラリで、中継元が選んだ RTMP のサーバーの応答をそのまま解釈することにもなる。
  - #33 では「`rtmp://` は HTTP の要求にならないので管理画面には届かない」としていたが、オプションで HTTP の取得をさせられるので、この前提は成り立たない。
  - C++ 版も同じ (リダイレクト先を区別せず、URL をそのまま librtmp に渡す)。
  - 案: 未確認の URL では `rtmp://` を受け付けない (`is_remote_safe_source` から外す。リダイレクトで RTMP に移る配信元はまれなので)。受け付けるなら、空白とタブを含まないことと、名前を引いたアドレスが `allowed_untrusted_ip` であることを確かめ、librtmp が引き直さないように IP アドレスの URL にして渡す。単体テスト (`is_remote_safe_source` に `rtmp://` を渡すと false) と、bvt (偽の配信元が `Location: rtmp://…` を返したら、つなぎに行かずにログに残すこと) で確かめる。
  - 済み: `is_remote_safe_source` から `rtmp://` を外し、未確認の URL の `rtmp://` は「Refusing a non-network URL given by the source」でつながない。管理者が入力した `rtmp://` は今までどおり。単体テスト `remote_safe_sources` と、bvt の `no_fetch_into_internal` (`Location: rtmp://127.0.0.2:…` を返す配信元。`--features rtmp` で回す) で確かめる。直す前は bvt が失敗することも確かめた。
- [x] #54 ICY の放送 (`SOURCE`) は、localhost からならパスワードなしで受け付け、そのときに Host ヘッダーとほかのサイトからの要求かを見ていません (中)。
  - `handshake_icy` の localhost の判定は、ソケットの相手のアドレス (`is_localhost`) だけ。#34 で HTTP Push (`POST /`) には `trust_private` (Host が手元の名前で、ほかのサイトのページから送らされたものでない) を、`/admin.cgi` には `trust_localhost` とほかのサイトからの要求の判定を入れたが、`SOURCE` の行 (`handshake_source` → `handshake_icy`) には入っていない。
  - 影響は HTTP Push と同じ形 (利用者の IP で配信を始め、既定の rootHost の YP に載る。同じ ID の放送があれば止める)。ShoutCast の形 (1 行目がパスワード) はパスワードが要るので対象外。ブラウザーから `SOURCE` の要求を送れるか (DNS リバインディングで同じオリジンになったときなど) は確かめていない。
  - C++ 版は、パスワードが空 (既定) ならどこからでも受け付ける (前に書いたもの)。パスワードがあっても、localhost からの要求の Host などは見ない。
  - 案: localhost からパスワードなしで受け付けるときは、HTTP の形の行 (`is_http`) なら `trust_private` と同じく、Host がループバックか LAN の名前 (ないものは今までどおり受け付ける) で、ほかのサイトからの要求でないことを確かめる。HTTP の形でない古い ICY の行 (ICE/1.0 など) はヘッダーを付けないので今までどおり。bvt (`cross_site_cannot_start_relay` と同じ形で、`SOURCE` の要求に Sec-Fetch-Site・Origin・Host を付けたものは 403、付けないものは受け付ける) で確かめる。
  - 済み: `handshake_icy` で、localhost から HTTP の形の行でパスワードなし (空の Basic 認証を含む) のときは、`trust_private` (HTTP Push と同じく、利用者がリンクを押したものも通さない) を通らなければ 403 にする。パスワードを送ったものと、HTTP の形でない古い行は今までどおり。ブラウザーの `fetch` は `SOURCE` のメソッドと `Authorization: Basic source:` を送れるので、パスワードなしの形も実際に作れる。bvt の `icy_source_cross_site` で確かめる。直す前は bvt が失敗する (ほかのサイトからの要求に 200 を返す) ことも確かめた。
- [ ] #55 要求を読み終えていない接続の IP ごとの上限 (`servent.rs` の `HANDSHAKES`、`maxHandshakesPerIp`) を、IPv6 でもアドレスごとに数えています (軽)。
  - パスワードの締め出しは #39 で IPv6 を /64 ごとにしたが、こちらは `cs.host.ip.str()` をそのままキーにしている。/64 を持つ相手はアドレスを変えて上限を越え、受け付ける接続の数 (`maxServIn`) を `handshakeTimeout` の間埋められる (ループバックからの接続は数えないので管理画面は開ける)。IPv6 で待ち受けているときだけ。C++ 版にはこの上限自体がない。
  - 案: キーを `servhs::auth_key` (IPv6 は /64) にそろえる。単体テストで、同じ /64 の別のアドレスが同じキーになることを確かめる。
- [x] #56 `RtmpStream::open` が、librtmp が書き込む URL の文字列を `CString::as_ptr()` (書き込まない前提のポインタ) で渡しています (軽。Rust 版だけ)。
  - librtmp の `RTMP_SetupURL(RTMP *r, char *url)` は、`RTMP_ParseURL` (`const char *`) と違って `char *` を取り、空白などに NUL を書き込んでオプションを切り分ける。共有の参照から得たポインタを通して書くのは Rust の決まりの外 (いまのコンパイラーで困ることはないと思われる)。
  - 案: NUL で終わる `Vec<u8>` を持ち、`as_mut_ptr()` を渡す (`CString::into_raw` は、中に NUL を書かれると `from_raw` で長さが変わって解放を誤るので使わない)。#53 を直すときに一緒に直せる。
  - 済み: URL は NUL で終わる `Vec<u8>` で持ち、`as_mut_ptr()` を渡す。中に NUL を含む URL は今までどおり断る。`--features rtmp` のビルドと clippy で確かめた (librtmp につなぐ動かしての確認はしていない)。
- [ ] #57 PCP の相手が送るエージェント名 (`agnt`) と `mesg` の文字列を、改行などを除かずにデバッグのログに書きます (軽)。
  - `log::add_log` は正しい UTF-8 ならそのまま書くので、ログのファイルや標準出力に偽の行を作れる (管理画面のログの表示はエスケープしている)。#21 で `chan_info_string` には `strip_controls` を入れたが、ほかの PCP の文字列は通っていない。C++ 版と同じ。
  - 案: `add_log` で、改行を含む制御文字を `[0A]` のように書き換える (`log_escape` と同じ形)。

見て、問題がなかったもの:

- 依存するクレート: `peercast-rs` と `rtmp-server-rs` は外部のクレートに依存しない (Cargo.toml)。
- `unsafe`: `lib.rs` は `deny(unsafe_code)`、rtmp-server は `forbid(unsafe_code)`。使うのは `server::os`・`server::tls`・`server::rtmp` だけで、ほかの `unsafe` という語は関数の名前と説明だけ。`os.rs` の構造体 (`struct tm`・`struct passwd`・`sockaddr_in(6)`) は glibc と musl の Linux の並びどおり (ARM でも同じ)。`tls.rs` は長さを `c_int` に収めてから渡し、作ったものは `Drop` でだけ解放する。
- TLS のクライアント: 証明書を確かめないのはホスト名が空のときだけで、呼ぶ側 (`http::get`・掲示板) はどちらも名前を引いてからつなぐので、空や NUL を含む名前で確かめを飛ばすことはない。
- `servhs.rs` (要求の振り分け): 入口ごとの守りは次のとおりで、上の #54 のほかに抜けはない。
  - `/admin`・`POST /admin`・`POST /api/1`・`/cmd?`・`/cgi-bin/` (flv.cgi 以外): `handshake_auth` (ほかのサイトからの要求は断る)。`/html/`: `handshake_auth` (ページを開くだけなので断らない。play.html は #34)。
  - `/stream/`・`/channel/`・`/pls/`: `trust_private` かトークンがなければ中継を始めない (#25・#34)。flv.cgi は #44。
  - `/admin.cgi`: ほかのサイトからの要求を断り、`trust_localhost`、締め出し。
  - HTTP Push: private で `trust_private`。GIV: #36・#41 の時間の窓。PCP: `ALLOW_NETWORK` とフィルター。
  - パスワードを比べるところ (`?pass=`・Basic 認証・ShoutCast の 1 行目・ICY・admin.cgi) は、どれも `ct_eq` で比べ、localhost 以外は締め出しを通る。Cookie の ID と `?auth=` のトークンは推測できない長さの乱数とハッシュ。
- 応答のヘッダーに書く値: 要求の行を読むときに CR を捨て LF で切るので、要求から来た値 (Referer など) に改行は入らない。`requested_path` はデコードしたあとで `cgi::is_safe_local_path` (制御文字と `//`・`/\` を断る) を通す。`cmd=redirect` のページは `http(s)://` に限り、HTML のエスケープをする。`customizeAppearance` の値は、ページでは決まった文字列と比べるだけ。
- 外向きの接続 (`ClientSocket::connect` を呼ぶ 12 か所): YP (rootHost)・管理者の入力した URL・速度測定の登録先・コンソールの `helo` は管理者が決めた宛先、portcheck は決まった名前。ping は相手のアドレスそのもの。GIV (#24)・速度測定の POST (#29)・リダイレクト先 (#33)・掲示板は宛先を確かめている。確かめていないのは、ヒットリストから選ぶ宛先 (中継元 #51 と、COUT が ID 0 のヒットリストからトラッカーを選ぶとき。送るのは決まった形の PCP の helo で、#51 と同じ程度) と、上の #53 (librtmp が自分でつなぐもの)。
- アドレスの分類: `bbs_http::is_public` (#33 の判定) は予約済みの範囲、CGN、NAT64、IPv4 射影まで見ている。GIV と速度測定の判定は 0.0.0.0/8 とマルチキャストを `is_unconnectable` で断る。
- `jrpc.rs` の後半: 引数の数は `dispatch` で先に確かめるので、メソッドの中の添字で落ちない。中継ツリーの再帰は、親が 1 つの木をたどり、ヒットの数の上限 (#37) で深さも抑えられる。`setSettings` の負の数は上限なしになるが、管理者の操作で C++ 版と同じ。
