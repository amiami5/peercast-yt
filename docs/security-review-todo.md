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
- [ ] #34 ほかのサイトのページから、localhost の利用者のノードに配信や中継を始めさせられます (CSRF と DNS リバインディング)。
  - 次のものは、private (localhost を含む) からなら、Origin・Sec-Fetch-Site も Host ヘッダーも見ずに受け付ける。
    - `POST /` (HTTP Push、`handshake_http_push`): 送った本体をそのまま配信できる。既定の rootHost は実在の YP (yp.pcgw.pgw.jp:7146) なので、利用者の IP で YP に載る。
    - `/stream/`・`/pls/` の `?ip=` / `?tip=`: 任意の宛先 (LAN を含む) からの中継を始めさせられる。
    - `/html/…/play.html?id=` (`handshake_auth` を `reject_cross_origin = false` で呼ぶ): `id` に `%3Fip%3D…` を入れると、同じく中継を始めさせられる。
  - 案: 配信ソフトやプレイヤーは Origin などを付けないので、`is_cross_origin_request` に当たるものは断る (`/stream/`・`/pls/` は、配信中のチャンネルを返すのはそのままにし、中継を始めることと `ip=`/`tip=` だけを断る)。localhost として信じるときは、`handshake_auth` と同じく Host がループバックの名前か IP アドレスであることも確かめる。
- [ ] #35 PCP の接続なら、どれからでもこのノードのチャンネルのストリームにパケットを書けます。
  - `pcpstream.rs` の `chan_packet` は、届いた接続がそのチャンネルの中継元 (上流) かどうかを見ていない。下流の中継先や CIN からでも、自分が配信しているチャンネルを含めてデータを書き込め、`pos=0` の head でストリームを入れ替えられる。チャンネルの情報の更新 (`chan_end` の `update_info`) も上流以外から受け付ける。C++ 版と同じ。
  - 案: `PcpStream` に、どのチャンネルの上流か (`Option<チャンネル ID>`) を持たせ、`pkt` と情報の更新はそのチャンネルについてだけ受け付ける。ほかの接続で届いた `pkt` は読み飛ばす。
- [ ] #36 引数のない GIV で、YP への接続 (COUT) を乗っ取れます。
  - 誰でも `GIV` を送れば、そのソケットが COUT の `push_sock` に置かれ (`servmgr::accept_giv`)、COUT が次につなぎ直すとき、YP の代わりにそのソケットが使われる。トラッカーの更新 (`tracker_update_atom`) には放送 ID (BCID) が入るので相手に渡り、BCID があればどのチャンネルの `?auth=` も作れる。相手がつないだままにすれば、その間 YP にチャンネルが載らない。C++ 版と同じ。
  - 案: 引数のない GIV は、こちらが PUSH を頼んだ直後だけ受け付ける (置いたソケットにも期限を付ける)。BCID は YP (`best.yp`) への COUT のときだけ送る。
- [ ] #37 ヒットリストとヒットの数に上限がありません。
  - PCP の相手は、チャンネル ID やアドレスを変えていくらでもヒットリスト (`chanmgr::add_hit_list`、`add_hit`) とヒットを足せる。足すたびに一覧を頭から探すので、メモリも CPU も使い、ロックも長く持つ。C++ 版と同じ。
  - 案: ヒットリストの数と、1 つのリストのヒットの数に上限を設ける (超えたら古いものから捨てるか、足さない)。
- [ ] #38 管理画面をほかのサイトの枠 (iframe) に入れられます (クリックジャッキング)。
  - `html::write_ok` などの応答に `X-Frame-Options` も CSP の `frame-ancestors` もない。
  - 案: 管理画面の応答に `X-Frame-Options: DENY` と `Content-Security-Policy: frame-ancestors 'none'` を付ける。
- [ ] #39 パスワードの締め出しを IP アドレスの単位で数えています (軽)。
  - IPv6 では /64 の中でアドレスを変えれば締め出しを逃れられる。覚えている数 (4096) が締め出し中の IP アドレスで埋まると、新しい IP アドレスは数えない (`servhs::AuthThrottle::failed`)。
  - 案: IPv6 は /64 ごとに数える。埋まったときは数えないのでなく、全体の数で締め出す。
- [ ] #40 `--enable-notify-send` のとき、チャンネル名やコメントをそのまま notify-send に渡しています (軽)。
  - 本文は多くの通知のデーモンでマークアップとして解釈される。
  - 案: `&` `<` `>` を実体参照にしてから渡す。

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

## clippy の警告 (2026-10-01、セキュリティの見直しのあとで片付ける)

`cargo clippy --workspace --all-targets` (clippy 0.1.98) で、error はなく warning が 82 件。セキュリティの問題ではなく、書き方の提案と、少し性能に関わるもの。#34〜#40 が済んだら、まとめて片付けるコミットを作る。それ以降は、変更のたびに警告が増えていないことを確かめる。

- [ ] 性能に少し関わるもの
  - `Err` の型が 136 バイト以上と大きい (`server/channel.rs`・`servent.rs`・`servmgr.rs`)。`Error` を `Box` にするなどを考える。
  - enum の要素の大きさの差が大きい (`chandir.rs` 368 バイト、`server/regex.rs` 256 バイト)。
  - `DoubleEndedIterator` に `last()` を使っていて、全部たどる (`server/servent_http.rs`)。
- [ ] 間違いにつながりうるもの
  - `if` の両方の枝が同じ (`server/channel.rs`)。意図したものか確かめる。
  - 引き算の下限を手で確かめている (`chanpacket.rs` 2 件、`saturating_sub` にできる)。
  - `Result<_, ()>` を返している (`xml.rs` 3 件)。誤りの理由が分からない。
- [ ] 書き方だけのもの (自動で直せるものが多い。`cargo clippy --fix` も使える)
  - `map_or` を簡単にできる (13 件: `server/host.rs`・`regex.rs`・`servent.rs`・`sys.rs`・`directory.rs`・`flag.rs`・`chanmgr.rs`・`jrpc.rs`)。
  - 型が複雑なので `type` で名前を付ける (7 件: `server/log.rs` 4 件・`channel.rs`・`server/commands.rs`・rtmp-server-rs)。
  - `repeat().take()` を `repeat_n` に (6 件、rtmp-server-rs の `amf0.rs` とテスト)。
  - 不要な `to_vec` (`commands.rs` 4 件)、すぐ外す参照 (`server/servmgr.rs`・`directory.rs` 4 件)、`Default::default()` のあとのフィールドの代入 (`pcp/handshake.rs`・`server/chaninfo.rs`・`server/sources.rs`)、`Range::contains` にできる比べ方 (`json/mod.rs`・`servent.rs`・`template/value.rs`)。
  - そのほか 1 件ずつのもの (引数が 8 つの関数 `bbs/mod.rs`、`SSL` の名前 `server/tls.rs`、`next` という名前のメソッド `server/sys.rs`、`assert_eq!` に `true`/`false` `servhs/tests.rs` など)。
