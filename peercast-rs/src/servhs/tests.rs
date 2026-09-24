use super::*;

#[test]
fn request_kinds() {
    assert_eq!(request_kind(b"GET / HTTP/1.1", b""), RequestKind::Get);
    assert_eq!(request_kind(b"GET x", b""), RequestKind::Bad);
    assert_eq!(request_kind(b"POST /api/1 HTTP/1.1", b""), RequestKind::Post);
    assert_eq!(request_kind(b"GIV /abc", b""), RequestKind::Giv);
    assert_eq!(request_kind(b"pcp\n", b""), RequestKind::Pcp);
    assert_eq!(request_kind(b"SOURCE /x ICE/1.0", b""), RequestKind::Source);
    assert_eq!(request_kind(b"hackme", b"hack"), RequestKind::Shoutcast);
    assert_eq!(request_kind(b"hackme", b""), RequestKind::Bad);
    assert!(is_http(b"GET / http/1.0"));
    assert!(!is_http(b"GET / HTTP/2"));
}

#[test]
fn get_routes() {
    assert_eq!(get_route(b"/admin?cmd=apply HTTP/1.1"), (GetKind::Admin, Some(16)));
    assert_eq!(get_route(b"/admin/?cmd=x"), (GetKind::AdminSlash, None));
    assert_eq!(get_route(b"/html/index.html HTTP/1.0").0, GetKind::HtmlIndex);
    assert_eq!(get_route(b"/html/index.htmlHTTP/1.0").0, GetKind::Html);
    assert_eq!(get_route(b"/html/ja/index.html HTTP/1.0").0, GetKind::Html);
    assert_eq!(get_route(b"/admin.cgi?pass=x&song=y").0, GetKind::AdminCgi);
    assert_eq!(get_route(b"/pls/0123.pls").0, GetKind::Pls);
    assert_eq!(get_route(b"/stream/0123.flv").0, GetKind::Stream);
    assert_eq!(get_route(b"/channel/0123").0, GetKind::Channel);
    assert_eq!(get_route(b"/api/1 HTTP/1.1").0, GetKind::Api1);
    assert_eq!(get_route(b"/api/1x").0, GetKind::Other);
    assert_eq!(get_route(b"/public").0, GetKind::Public);
    assert_eq!(get_route(b"/public/index.html").0, GetKind::Public);
    assert_eq!(get_route(b"/publicx").0, GetKind::Other);
    assert_eq!(get_route(b"/assets/css/a.css").0, GetKind::Assets);
    assert_eq!(get_route(b"/cgi-bin/flv.cgi?x").0, GetKind::CgiBinFlv);
    assert_eq!(get_route(b"/cgi-bin/x.cgi").0, GetKind::CgiBin);
    assert_eq!(get_route(b"/cmd?q=help").0, GetKind::Cmd);
    // 先頭が HTTP/1. なら fn の前に NUL を書く (fn は変わらない)
    assert_eq!(get_route(b"HTTP/1.1"), (GetKind::Other, Some(-1)));
}

#[test]
fn admin_cgi_args() {
    let a = admin_cgi(b"/admin.cgi?pass=x&mode=updinfo&song=Title%20A&mount=/live&url=http://u").unwrap();
    assert_eq!(a.song, b"Title%20A");
    assert_eq!(a.mount.as_deref(), Some(&b"/live"[..]));
    assert_eq!(a.url.as_deref(), Some(&b"http://u"[..]));
    assert_eq!(admin_cgi(b"/admin.cgi?song=x"), None);
    // 名前の途中にも一致する (strstr)
    assert_eq!(admin_cgi(b"/admin.cgi?xpass=1&xsong=2").unwrap().song, b"2");
}

#[test]
fn post_routes() {
    assert_eq!(post_route(b"POST /api/1?pass=x HTTP/1.1"), Some((PostKind::Api1, b"pass=x".to_vec())));
    assert_eq!(post_route(b"POST /?name=a&b=c?d HTTP/1.1"), Some((PostKind::Push, b"name=a&b=c?d".to_vec())));
    assert_eq!(post_route(b"POST /admin HTTP/1.1"), Some((PostKind::Admin, Vec::new())));
    assert_eq!(post_route(b"POST /x HTTP/1.1"), Some((PostKind::Other, Vec::new())));
    assert_eq!(post_route(b"POST  /api/1 HTTP/1.1"), None);
    assert_eq!(post_route(b"POST /api/1"), None);
}

#[test]
fn source_lines() {
    assert_eq!(source(b"SOURCE /live ICE/1.0"), Source { password: None, mount: b"/live ".to_vec() });
    assert_eq!(source(b"SOURCEICE/1.0"), Source { password: None, mount: b"CE/1.0".to_vec() });
    assert_eq!(source(b"SOURCE hackme /live"), Source { password: Some(b"hackme".to_vec()), mount: b"/live".to_vec() });
    // NUL は 6 文字目に書かれるので、7 文字目からのパスワードは行の終わりまで
    assert_eq!(source(b"SOURCE /live"), Source { password: Some(b"/live".to_vec()), mount: b"/live".to_vec() });
    assert_eq!(source(b"SOURCE a/b/c"), Source { password: Some(b"a/".to_vec()), mount: b"/c".to_vec() });
    // C++ 版は行の前のメモリを読んでいた
    assert_eq!(source(b"SOURCE hackme"), Source { password: Some(b"hackme".to_vec()), mount: Vec::new() });
    assert_eq!(source(b"SOURCE"), Source { password: Some(Vec::new()), mount: Vec::new() });
}

#[test]
fn icy_passwords() {
    // localhost: パスワードなしか、一致するもの
    assert!(icy_password_ok(b"", b"", true));
    assert!(icy_password_ok(b"", b"secret", true));
    assert!(icy_password_ok(b"secret", b"secret", true));
    assert!(!icy_password_ok(b"wrong", b"secret", true));
    assert!(!icy_password_ok(b"x", b"", true));
    // それ以外: パスワードが設定されていて一致するものだけ (C++ 版は空どうしを通していた)
    assert!(!icy_password_ok(b"", b"", false));
    assert!(!icy_password_ok(b"", b"secret", false));
    assert!(!icy_password_ok(b"wrong", b"secret", false));
    assert!(icy_password_ok(b"secret", b"secret", false));
}

#[test]
fn giv_and_auth_token() {
    let id = [0x12u8; 16];
    let hex: Vec<u8> = gnuid::to_str(&id).to_vec();
    assert_eq!(giv_id(&[&b"GIV /"[..], &hex].concat()), id);
    assert_eq!(giv_id(b"GIV"), [0; 16]);

    let bcid = [0x34u8; 16];
    let token = channel::auth_token(&bcid, &id);
    let lower = strutil::downcase(&hex);
    let req = [&lower[..], b".flv?auth=", &token].concat();
    assert!(valid_auth_token(&req, &bcid));
    assert!(!valid_auth_token(&[&lower[..], b".flv?auth=x"].concat(), &bcid));
    assert!(!valid_auth_token(&lower, &bcid));
    assert!(!valid_auth_token(&[&req[..], b"?"].concat(), &bcid));
}

#[test]
fn cookies() {
    assert_eq!(cookie_id(b"a=b; 7144_id=XYZ; c=d", 7144), CookieParse::Found(b"XYZ".to_vec()));
    assert_eq!(cookie_id(b"7144_id=a=b", 7144), CookieParse::Found(b"a=b".to_vec()));
    assert_eq!(cookie_id(b"a=b; junk; 7144_id=X", 7144), CookieParse::Invalid);
    assert_eq!(cookie_id(b"a=b;7144_id=X", 7144), CookieParse::NotFound);
    assert_eq!(cookie_id(b"8000_id=X", 7144), CookieParse::NotFound);
}

#[test]
fn query_and_cgi_args() {
    let q = Query::new(b"a=1&a=2&b&c=x%20y=z&&=e");
    assert_eq!(q.get(b"a"), b"1");
    assert_eq!(q.get(b"b"), b"");
    assert_eq!(q.get(b"c"), b"x y");
    assert_eq!(q.get(b""), b"e");
    assert_eq!(q.keys().cloned().collect::<Vec<_>>(), vec![b"".to_vec(), b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);

    // 名前は = まで読む (& では止まらない)
    assert_eq!(
        cgi_args(b"a=1&b&c=x=y&"),
        vec![(b"a".to_vec(), b"1".to_vec()), (b"b&c".to_vec(), b"x=y".to_vec())]
    );
    let long = [vec![b'k'; 600], b"=v".to_vec()].concat();
    let args = cgi_args(&long);
    assert_eq!(args[0].0.len(), 511);
    assert_eq!(args[0].1, [vec![b'k'; 89], b"=v".to_vec()].concat());
}

#[test]
fn numbers_and_paths() {
    assert_eq!(atoi(b"  -12x"), -12);
    // 範囲を超える数は、段階 3a と同じく端に丸める
    assert_eq!(atoi(b"4294967297"), i32::MAX);
    assert_eq!(atoi(b"-99999999999999999999"), i32::MIN);
    assert_eq!(atoi(b"+"), 0);
    assert!(is_valid_html_path(b"html/ja"));
    assert!(!is_valid_html_path(b"html/"));
    assert!(!is_valid_html_path(b"html/../x"));
    assert!(!is_valid_html_path(&[&b"html/"[..], &[b'a'; 65]].concat()));
    assert!(is_decimal(b"0") && is_decimal(b"120") && !is_decimal(b"01") && !is_decimal(b"") && !is_decimal(b"1a"));
}

#[test]
fn redirects() {
    assert_eq!(redirect_url(b"cmd=redirect&url=www.example.com%2Fa"), Some(b"http://www.example.com/a".to_vec()));
    assert_eq!(redirect_url(b"url=https://x/&y=1"), Some(b"https://x/".to_vec()));
    assert_eq!(redirect_url(b"cmd=redirect"), None);
    assert_eq!(rewrite_referer(b"http://h/html/ja/index.html", b"html/en"), Some(b"http://h/html/en/index.html".to_vec()));
    assert_eq!(rewrite_referer(b"http://h/html//html/ja", b"html/en"), Some(b"http://h/html//html/en".to_vec()));
    assert_eq!(rewrite_referer(b"http://h/", b"html/en"), None);
}

#[test]
fn apply() {
    let ops = apply_ops(b"servername=a%20b&icymeta=99999&htmlPath=ja&htmlPath=..&filt_ip=127.0.0.1&filt_bn=1&auth=x&auth=cookie&channel_feed_url=&allowHTML1=2&zzz=1");
    let keys: Vec<ApplyKey> = ops.iter().map(|o| o.key).collect();
    assert_eq!(
        keys,
        vec![ApplyKey::ServerName, ApplyKey::IcyMeta, ApplyKey::HtmlPath, ApplyKey::HtmlPath, ApplyKey::FiltIp, ApplyKey::FiltBan, ApplyKey::Auth, ApplyKey::AllowHtml]
    );
    assert_eq!(ops[0].str, b"a b");
    assert_eq!(ops[1].int, 16384);
    assert_eq!((ops[2].int, ops[2].str.as_slice()), (1, &b"html/ja"[..]));
    assert_eq!(ops[3].int, 0);
    assert_eq!(ops[6].int, 1);
    assert_eq!(ops[7].int, 1);
}

#[test]
fn icy_headers() {
    assert_eq!(icy_header(b"icy-name: x"), IcyHeader::Name);
    assert_eq!(icy_header(b"X-Foo: ICY-NAME"), IcyHeader::Name);
    assert_eq!(icy_header(b"icy-br: 128"), IcyHeader::Bitrate);
    assert_eq!(icy_header(b"x-peercast-channelid: 00"), IcyHeader::ChannelId);
    assert_eq!(icy_header(b"x-peercast-channelid 00"), IcyHeader::Other);
    assert_eq!(icy_header(b"Content-Type: audio/mpeg"), IcyHeader::ContentType);
    assert_eq!(icy_content_type(b"audio/x-mpegurl"), Some(&b"MP3"[..]));
    assert_eq!(icy_content_type(b"application/x-peercast-pcp"), Some(&b"PCP"[..]));
    assert_eq!(icy_content_type(b"video/x-flv"), None);
}

#[test]
fn local_files() {
    assert_eq!(mime_type_for(b"/x/html/ja/INDEX.HTML"), Some(&b"text/html"[..]));
    assert_eq!(mime_type_for(b"/x/a.png?.htm"), Some(&b"text/html"[..]));
    assert_eq!(mime_type_for(b"/x/a.txt"), None);
    let f = local_file(b"html/ja/play.html?id=0123&x=1");
    assert_eq!((f.page, f.split_ok, f.id.as_slice()), (LocalPage::Play, true, &b"0123"[..]));
    assert_eq!(local_file(b"html/ja/head.html").split_ok, false);
    assert_eq!(local_file(b"html/ja/connections.html?id=a?b").split_ok, false);
    assert_eq!(local_file(b"html/ja/index.html").page, LocalPage::Plain);
    assert_eq!(local_file_name(b"/r/", b"html/a.html"), b"/r/html/a.html");
    assert_eq!(local_file_name(b"/r/", &[b'a'; 252]), b"/r/");
}

#[test]
fn jrpc() {
    assert_eq!(jrpc_body_length(b"", 100), Err(("HTTP/1.0 411 Length required", 411)));
    assert_eq!(jrpc_body_length(b"-1", 100), Err(("HTTP/1.0 411 Length required", 411)));
    assert_eq!(jrpc_body_length(b"0", 100), Err(("HTTP/1.0 400 Bad Request", 400)));
    assert_eq!(jrpc_body_length(b"101", 100), Err(("HTTP/1.0 413 Request Entity Too Large", 413)));
    assert_eq!(jrpc_body_length(b" 42x", 100), Ok(42));
    // C++ 版は x86-64 で (int)LONG_MAX の -1 になり 411 だった
    assert_eq!(jrpc_body_length(b"99999999999999999999", 100), Err(("HTTP/1.0 413 Request Entity Too Large", 413)));
}

#[test]
fn flv() {
    let id = "0123456789abcdef0123456789ABCDEF";
    let args = flv_ffmpeg_args(format!("id={}&preset=ultrafast&audio_codec=aac&type=FLV&bitrate=1200", id).as_bytes(), 7144).unwrap();
    assert_eq!(args[5], format!("http://127.0.0.1:7144/stream/{}", id));
    assert_eq!((args[9].as_str(), args[17].as_str(), args[19].as_str()), ("aac", "bitrate=1200:vbv-maxrate=1200:vbv-bufsize=2400", "ultrafast"));
    for bitrate in ["", "&bitrate=0", "&bitrate=100001", "&bitrate=x"] {
        let q = format!("id={}&preset=p&audio_codec=a&type=t{}", id, bitrate);
        assert_eq!(flv_ffmpeg_args(q.as_bytes(), 1).unwrap()[17], "bitrate=500:vbv-maxrate=500:vbv-bufsize=1000");
    }
    for q in [
        "preset=p&audio_codec=a&type=t".to_string(),
        format!("id={}&preset=p&audio_codec=a", id),
        format!("id={}x&preset=p&audio_codec=a&type=t", id),
        format!("id={}&preset=-i&audio_codec=a&type=t", id),
        format!("id={}&preset=p&audio_codec=a%20b&type=t", id),
        format!("id={}&preset={}&audio_codec=a&type=t", id, "p".repeat(33)),
    ] {
        assert_eq!(flv_ffmpeg_args(q.as_bytes(), 7144), None, "{}", q);
    }
}

#[test]
fn flv_auth_token() {
    let bcid = [7u8; 16];
    let id = "0123456789abcdef0123456789ABCDEF";
    let token = String::from_utf8(channel::auth_token(&bcid, &gnuid::from_str(id.to_uppercase().as_bytes()))).unwrap();
    let req = |q: String| [&b"/cgi-bin/flv.cgi?"[..], q.as_bytes()].concat();
    assert!(flv_valid_auth_token(&req(format!("id={}&type=MKV&auth={}", id, token)), &bcid));
    assert!(flv_valid_auth_token(&req(format!("auth={}&id={}", token, id.to_lowercase())), &bcid));
    assert!(!flv_valid_auth_token(&req(format!("id={}&auth={}", id, token)), &[8; 16]));
    assert!(!flv_valid_auth_token(&req(format!("id={}&auth=", id)), &bcid));
    assert!(!flv_valid_auth_token(&req(format!("id={}", id)), &bcid));
    assert!(!flv_valid_auth_token(&req(format!("id={}0&auth={}", id, token)), &bcid));
    assert!(!flv_valid_auth_token(format!("/cgi-bin/flv.cgi&id={}&auth={}", id, token).as_bytes(), &bcid));
}

/// 乱数の入力でパニックしない
#[test]
fn fuzz() {
    let mut x: u32 = 4242;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    const ALPHA: &[u8] = b"/?&=;: %aAHTP1.-_0#htmlpasong";
    for _ in 0..100_000 {
        let n = (rnd() % 40) as usize;
        let s: Vec<u8> = (0..n).map(|_| if rnd() % 4 == 0 { rnd() as u8 } else { ALPHA[(rnd() as usize) % ALPHA.len()] }).collect();
        let _ = request_kind(&s, b"pass");
        let _ = get_route(&s);
        let _ = admin_cgi(&s);
        let _ = post_route(&s);
        let _ = giv_id(&s);
        let _ = source(&s);
        let _ = valid_auth_token(&s, &[1; 16]);
        let _ = flv_valid_auth_token(&s, &[1; 16]);
        let _ = cookie_id(&s, 7144);
        let _ = apply_ops(&s);
        let _ = redirect_url(&s);
        let _ = rewrite_referer(&s, b"html/en");
        let _ = icy_header(&s);
        let _ = icy_content_type(&s);
        let _ = local_file(&s);
        let _ = flv_ffmpeg_args(&s, 7144);
        let _ = jrpc_body_length(&s, 1 << 20);
        let _ = atoi(&s);
    }
}
