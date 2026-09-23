//! HTTP の要求の処理 (core/common/servhs.cpp の `Servent::handshake*`) のうち、要求の解釈と判断 (段階 8b)。
//!
//! 要求の行の種類とパスの振り分け、GIV・SOURCE の行、認証 (Cookie ヘッダー、`auth` トークン)、CGI の
//! 引数 (`nextCGIarg`、`getCGIarg`)、設定の保存 (`CMD_apply`) の引数、ICY のヘッダー、ローカルの
//! ファイルのパス、CGI スクリプトの出力のヘッダーなど。ソケットの読み書きと、サーバーやチャンネルの
//! 状態を触ることは C++ に残る。
//!
//! 文字列は C の文字列 (NUL の手前まで) として渡される前提。

use std::collections::BTreeMap;

use crate::{cgi, channel, gnuid, pcstring, strutil};

/// `MAX_CGI_LEN` (core/common/sys.h)
pub const MAX_CGI_LEN: usize = 512;

/// `strstr`。needle が空なら先頭。
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `stristr` (core/common/sys.cpp)。ASCII の英字だけ大文字小文字を区別しない。needle が空なら
/// 見つからない (`strstr` と違う)。
pub fn stristr(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// `cgi::Query` (core/common/cgi.cpp)
#[derive(Clone, Debug, Default)]
pub struct Query {
    dict: BTreeMap<Vec<u8>, Vec<Vec<u8>>>,
}

impl Query {
    pub fn new(query: &[u8]) -> Query {
        let mut dict: BTreeMap<Vec<u8>, Vec<Vec<u8>>> = BTreeMap::new();
        for assignment in strutil::split(query, b"&") {
            if assignment.is_empty() {
                continue;
            }
            let sides = strutil::split(&assignment, b"=");
            if sides.len() == 1 {
                dict.insert(sides[0].clone(), Vec::new());
            } else {
                dict.entry(sides[0].clone()).or_default().push(cgi::unescape(&sides[1]));
            }
        }
        Query { dict }
    }

    /// 最初の値。なければ空。
    pub fn get(&self, key: &[u8]) -> Vec<u8> {
        self.dict.get(key).and_then(|v| v.first()).cloned().unwrap_or_default()
    }

    pub fn keys(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.dict.keys()
    }
}

// ---------------------------------------------------------------- 要求の行

/// `handshakeIncoming`: 1 行目に `HTTP/1.` が (大文字小文字を区別せずに) 含まれるか
pub fn is_http(line: &[u8]) -> bool {
    stristr(line, b"HTTP/1.").is_some()
}

/// `handshakeHTTP` で、要求の行がどれか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Get = 0,
    Post = 1,
    Giv = 2,
    Pcp = 3,
    Source = 4,
    /// ShoutCast の放送 (行がパスワードで始まる)
    Shoutcast = 5,
    Bad = 6,
}

/// `handshakeHTTP` の振り分け (`isRequest` は前方一致)。`password` は `servMgr->password`。
pub fn request_kind(line: &[u8], password: &[u8]) -> RequestKind {
    if line.starts_with(b"GET /") {
        RequestKind::Get
    } else if line.starts_with(b"POST /") {
        RequestKind::Post
    } else if line.starts_with(b"GIV") {
        RequestKind::Giv
    } else if line.starts_with(b"pcp") {
        RequestKind::Pcp
    } else if line.starts_with(b"SOURCE") {
        RequestKind::Source
    } else if !password.is_empty() && line.starts_with(password) {
        RequestKind::Shoutcast
    } else {
        RequestKind::Bad
    }
}

/// `handshakeGET` の振り分け
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GetKind {
    /// `/admin?` (引数は 7 文字目から)
    Admin = 0,
    /// `/admin/?` (8 文字目から)
    AdminSlash = 1,
    HtmlIndex = 2,
    Html = 3,
    AdminCgi = 4,
    Pls = 5,
    Stream = 6,
    Channel = 7,
    Api1 = 8,
    Public = 9,
    Assets = 10,
    CgiBinFlv = 11,
    CgiBin = 12,
    Cmd = 13,
    Other = 14,
}

/// `handshakeGET` のパス (`cmdLine + 4`) を、後ろの ` HTTP/1.` の手前で切り、振り分ける。
/// 2 つ目の値は、C++ 版が NUL を書く位置 (`strstr(fn, "HTTP/1.")` の 1 つ前。-1 なら `fn` の前)。
pub fn get_route(path: &[u8]) -> (GetKind, Option<isize>) {
    let cut = find(path, b"HTTP/1.").map(|p| p as isize - 1);
    let fn_ = match cut {
        Some(c) if c >= 0 => &path[..c as usize],
        _ => path,
    };
    let kind = if fn_.starts_with(b"/admin?") {
        GetKind::Admin
    } else if fn_.starts_with(b"/admin/?") {
        GetKind::AdminSlash
    } else if fn_ == b"/html/index.html" {
        GetKind::HtmlIndex
    } else if fn_.starts_with(b"/html/") {
        GetKind::Html
    } else if fn_.starts_with(b"/admin.cgi") {
        GetKind::AdminCgi
    } else if fn_.starts_with(b"/pls/") {
        GetKind::Pls
    } else if fn_.starts_with(b"/stream/") {
        GetKind::Stream
    } else if fn_.starts_with(b"/channel/") {
        GetKind::Channel
    } else if fn_ == b"/api/1" {
        GetKind::Api1
    } else if fn_ == b"/public" || fn_.starts_with(b"/public/") {
        GetKind::Public
    } else if fn_.starts_with(b"/assets/") {
        GetKind::Assets
    } else if fn_.starts_with(b"/cgi-bin/") {
        if fn_.starts_with(b"/cgi-bin/flv.cgi") {
            GetKind::CgiBinFlv
        } else {
            GetKind::CgiBin
        }
    } else if fn_.starts_with(b"/cmd?") {
        GetKind::Cmd
    } else {
        GetKind::Other
    };
    (kind, cut)
}

/// `getCGIarg` のあと `termArgs` したときの値: `name` の最初の出現の直後から、次の `&` の手前まで。
pub fn cgi_arg(s: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    let start = find(s, name)? + name.len();
    let rest = &s[start..];
    let end = rest.iter().position(|&b| b == b'&').unwrap_or(rest.len());
    Some(rest[..end].to_vec())
}

/// `/admin.cgi` (ShoutCast の曲名の更新) の引数。`pass=` と `song=` がなければ `None`。
/// C++ 版と同じく、パスワードの中身は確かめない (docs/cpp-known-issues.md)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminCgi {
    pub song: Vec<u8>,
    pub mount: Option<Vec<u8>>,
    pub url: Option<Vec<u8>>,
}

pub fn admin_cgi(fn_: &[u8]) -> Option<AdminCgi> {
    let pass = cgi_arg(fn_, b"pass=");
    let song = cgi_arg(fn_, b"song=");
    let mount = cgi_arg(fn_, b"mount=");
    let url = cgi_arg(fn_, b"url=");
    match (pass, song) {
        (Some(_), Some(song)) => Some(AdminCgi { song, mount, url }),
        _ => None,
    }
}

/// `handshakePOST` の振り分け
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostKind {
    Api1 = 0,
    /// HTTP Push の放送 (`/`)
    Push = 1,
    Admin = 2,
    Other = 3,
}

/// `handshakePOST`。行が空白で 3 つに分かれなければ `None` (400)。パスと、`?` の後ろの引数。
pub fn post_route(line: &[u8]) -> Option<(PostKind, Vec<u8>)> {
    let vec = strutil::split(line, b" ");
    if vec.len() != 3 {
        return None;
    }
    let vec2 = strutil::split_limit(&vec[1], b"?", 2).unwrap_or_default();
    let args = if vec2.len() == 2 { vec2[1].clone() } else { Vec::new() };
    let path = vec2.first().cloned().unwrap_or_default();
    let kind = match path.as_slice() {
        b"/api/1" => PostKind::Api1,
        b"/" => PostKind::Push,
        b"/admin" => PostKind::Admin,
        _ => PostKind::Other,
    };
    Some((kind, args))
}

/// `handshakeGIV` のチャンネル ID (最初の `/` の後ろ。なければ 0)
pub fn giv_id(line: &[u8]) -> [u8; 16] {
    match line.iter().position(|&b| b == b'/') {
        Some(p) => gnuid::from_str(&line[p + 1..]),
        None => [0; 16],
    }
}

/// `handshakeSOURCE` の行 (`SOURCE ...`) から取り出すもの
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// ICY (`SOURCE password /mount`) ならパスワード。ICE/1.0 なら `None`。
    pub password: Option<Vec<u8>>,
    pub mount: Vec<u8>,
}

/// `handshakeSOURCE`。
///
/// C++ 版は、ICE/1.0 でない行に `/` がないと、行の先頭より前のメモリを NUL が見つかるまで後ろ向きに
/// 読み、そこに `/` があれば 1 つ前に NUL を書いていた。Rust 版は行の先頭で止め、マウントを空にする。
/// 行が 7 文字より短いときのパスワード (C++ 版は行の後ろの古い中身を読む) も空にする。
pub fn source(line: &[u8]) -> Source {
    let tail = |from: usize, to: usize| -> Vec<u8> {
        if from < to {
            line[from..to].to_vec()
        } else {
            Vec::new()
        }
    };
    if let Some(ps) = find(line, b"ICE/1.0") {
        // mount = in + 7 で、ICE/1.0 の先頭に NUL を書く
        let end = if ps >= 7 { ps } else { line.len() };
        return Source { password: None, mount: tail(7, end) };
    }
    match line.iter().rposition(|&b| b == b'/') {
        Some(p) => {
            // p の 1 つ前に NUL を書くので、パスワード (7 文字目から) はそこまで。NUL が 7 文字目より
            // 前なら、行の終わりまで
            let end = if p >= 8 { p - 1 } else { line.len() };
            Source { password: Some(tail(7, end)), mount: line[p..].to_vec() }
        }
        None => Source { password: Some(tail(7, line.len())), mount: Vec::new() },
    }
}

/// `Servent::hasValidAuthToken`。`request_filename` はパスの後ろ (`<チャンネル ID>...?auth=...`)。
pub fn valid_auth_token(request_filename: &[u8], broadcast_id: &[u8; 16]) -> bool {
    let vec = strutil::split(request_filename, b"?");
    if vec.len() != 2 {
        return false;
    }
    let token = Query::new(&vec[1]).get(b"auth");
    let chanid = strutil::upcase(&vec[0][..vec[0].len().min(32)]);
    let id = gnuid::from_str(&chanid);
    channel::auth_token(broadcast_id, &id) == token
}

/// `handshakeAuth` の Cookie ヘッダーの解釈
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CookieParse {
    /// `=` のない組があった (そこで止める)
    Invalid,
    /// `<port>_id` の値
    Found(Vec<u8>),
    NotFound,
}

pub fn cookie_id(header: &[u8], port: u16) -> CookieParse {
    let id_key = format!("{}_id", port).into_bytes();
    for assignment in strutil::split(header, b"; ") {
        let sides = strutil::split_limit(&assignment, b"=", 2).unwrap_or_default();
        if sides.len() != 2 {
            return CookieParse::Invalid;
        } else if sides[0] == id_key {
            return CookieParse::Found(sides[1].clone());
        }
    }
    CookieParse::NotFound
}

// ---------------------------------------------------------------- CGI の引数

/// `nextCGIarg` を最後まで繰り返したもの。名前と値は、それぞれ `MAX_CGI_LEN - 1` バイトで切れる
/// (名前が長すぎると `=` を読まずに値の読み取りに移る)。
pub fn cgi_args(cmd: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut res = Vec::new();
    let mut cp = 0;
    while cp < cmd.len() {
        let mut key = Vec::new();
        while cp < cmd.len() {
            let c = cmd[cp];
            cp += 1;
            if c == b'=' {
                break;
            }
            key.push(c);
            if key.len() >= MAX_CGI_LEN - 1 {
                break;
            }
        }
        let mut arg = Vec::new();
        while cp < cmd.len() {
            let c = cmd[cp];
            cp += 1;
            if c == b'&' {
                break;
            }
            arg.push(c);
            if arg.len() >= MAX_CGI_LEN - 1 {
                break;
            }
        }
        res.push((key, arg));
    }
    res
}

/// C の `atoi` (先頭の空白と符号を読み、`int` に収まらない値は、glibc の x86-64 と同じく `long` で
/// 計算してから下位 32 ビットにする)
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    // strtol は long に丸める (桁あふれは LONG_MAX / LONG_MIN)
    let mut v: i64 = 0;
    let mut overflow = false;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as i64;
        match v.checked_mul(10).and_then(|x| if neg { x.checked_sub(d) } else { x.checked_add(d) }) {
            Some(x) => v = x,
            None => overflow = true,
        }
        i += 1;
    }
    if overflow {
        v = if neg { i64::MIN } else { i64::MAX };
    }
    v as i32
}

/// `ServMgr::isValidHtmlPath`: `html/` の後ろが 1〜64 文字の英数字、`-`、`_`。
pub fn is_valid_html_path(path: &[u8]) -> bool {
    match path.strip_prefix(b"html/") {
        Some(name) => {
            !name.is_empty()
                && name.len() <= 64
                && name.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        }
        None => false,
    }
}

/// `isDecimal` (`^(0|[1-9][0-9]*)$`)
pub fn is_decimal(s: &[u8]) -> bool {
    match s {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] => rest.iter().all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// `CMD_redirect` の飛び先。`url=` がなければ `None` (400)。
pub fn redirect_url(cmd: &[u8]) -> Option<Vec<u8>> {
    // Sys::strcpy_truncate(buf, MAX_CGI_LEN, cmd)
    let buf = &cmd[..cmd.len().min(MAX_CGI_LEN - 1)];
    let j = cgi_arg(buf, b"url=")?;
    // url.set(j, String::T_ESC); url.convertTo(String::T_ASCII)
    let mut url = pcstring::esc_to_ascii(&j[..j.len().min(255)]);
    url.truncate(255);
    if !url.starts_with(b"http://") && !url.starts_with(b"https://") {
        // String::prepend は、合わせて 255 バイトに収まらなければ何も足さない (String::append と同じ)
        if url.len() + 7 < 255 {
            url.splice(0..0, b"http://".iter().copied());
        } else {
            url = b"http://".to_vec();
        }
    }
    Some(url)
}

/// `CMD_chooseLanguage`: Referer の最初の `html/[^/]+` を `new_html_path` に置き換える。
/// 見つからなければ `None`。
pub fn rewrite_referer(referer: &[u8], new_html_path: &[u8]) -> Option<Vec<u8>> {
    let mut from = 0;
    while let Some(p) = find(&referer[from..], b"html/") {
        let start = from + p;
        let name_start = start + 5;
        let name_len = referer[name_start..].iter().position(|&c| c == b'/').unwrap_or(referer.len() - name_start);
        if name_len > 0 {
            let mut out = referer[..start].to_vec();
            out.extend_from_slice(new_html_path);
            out.extend_from_slice(&referer[name_start + name_len..]);
            return Some(out);
        }
        from = start + 1;
    }
    None
}

/// `CMD_apply` で行うこと。値は `Op` の種類ごとに決まった形にしてある。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyKey {
    ServerName = 0,     // 文字列 (unescape)
    ServerActive = 1,   // 真偽値
    Port = 2,           // 数
    IcyMeta = 3,        // 数 (0〜16384 に丸めたもの)
    PassNew = 4,        // 文字列 (そのまま)
    Root = 5,
    BrRoot = 6,
    GetUpd = 7,
    HuInt = 8,
    ForceIp = 9,        // 文字列 (そのまま)
    HtmlPath = 10,      // 文字列 ("html/" + 値)、数は正しいパスなら 1
    DjMsg = 11,         // 文字列 (unescape)
    PcMsg = 12,         // 文字列 (unescape)
    MaxCin = 13,
    MaxSin = 14,
    MaxUp = 15,
    MaxRelays = 16,
    MaxDirect = 17,
    MaxRelayPc = 18,
    FiltIp = 19,        // 文字列 (unescape)
    FiltBan = 20,
    FiltPrivate = 21,
    FiltNetwork = 22,
    FiltDirect = 23,
    ChannelFeedUrl = 24, // 文字列 (unescape)。値が空なら何もしない
    ClientActive = 25,
    Yp = 26,            // 文字列 (unescape)
    DeadHitAge = 27,
    Refresh = 28,
    Chat = 29,
    RandomizeChid = 30,
    PublicDirectory = 31,
    Auth = 32,          // 1 Cookie、2 HTTP の Basic 認証
    Expire = 33,        // 0 セッションの間、1 無期限
    LogLevel = 34,
    AllowHtml = 35,     // 数 (atoi が 0 でなければ 1)
    AllowNetwork = 36,
    AllowBroadcast = 37,
    AllowDirect = 38,
    Transcoding = 39,
    Preset = 40,        // 文字列 (そのまま)
    AudioCodec = 41,
    PreferredTheme = 42,
    AccentColor = 43,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOp {
    pub key: ApplyKey,
    pub int: i32,
    pub str: Vec<u8>,
}

/// `CMD_apply` の引数 (`nextCGIarg` で読む) を、行うことの並びにする。知らない名前と、何もしない値
/// (空の `channel_feed_url`、知らない `auth` / `expire`) は入れない。
pub fn apply_ops(cmd: &[u8]) -> Vec<ApplyOp> {
    use ApplyKey::*;
    let mut ops = Vec::new();
    for (curr, arg) in cgi_args(cmd) {
        let b = (arg == b"1") as i32;
        let n = atoi(&arg);
        let op = |key, int, str: Vec<u8>| Some(ApplyOp { key, int, str });
        let o = match curr.as_slice() {
            b"servername" => op(ServerName, 0, cgi::unescape(&arg)),
            b"serveractive" => op(ServerActive, b, Vec::new()),
            b"port" => op(Port, n, Vec::new()),
            b"icymeta" => op(IcyMeta, n.clamp(0, 16384), Vec::new()),
            b"passnew" => op(PassNew, 0, arg.clone()),
            b"root" => op(Root, b, Vec::new()),
            b"brroot" => op(BrRoot, b, Vec::new()),
            b"getupd" => op(GetUpd, b, Vec::new()),
            b"huint" => op(HuInt, n, Vec::new()),
            b"forceip" => op(ForceIp, 0, arg.clone()),
            b"htmlPath" => {
                let p = [&b"html/"[..], &arg].concat();
                op(HtmlPath, is_valid_html_path(&p) as i32, p)
            }
            b"djmsg" => op(DjMsg, 0, cgi::unescape(&arg)),
            b"pcmsg" => op(PcMsg, 0, cgi::unescape(&arg)),
            b"maxcin" => op(MaxCin, n, Vec::new()),
            b"maxsin" => op(MaxSin, n, Vec::new()),
            b"maxup" => op(MaxUp, n, Vec::new()),
            b"maxrelays" => op(MaxRelays, n, Vec::new()),
            b"maxdirect" => op(MaxDirect, n, Vec::new()),
            b"maxrelaypc" => op(MaxRelayPc, n, Vec::new()),
            c if c.starts_with(b"filt_") => {
                let fs = &c[5..];
                if fs.starts_with(b"ip") {
                    op(FiltIp, 0, cgi::unescape(&arg))
                } else if fs.starts_with(b"bn") {
                    op(FiltBan, 0, Vec::new())
                } else if fs.starts_with(b"pr") {
                    op(FiltPrivate, 0, Vec::new())
                } else if fs.starts_with(b"nw") {
                    op(FiltNetwork, 0, Vec::new())
                } else if fs.starts_with(b"di") {
                    op(FiltDirect, 0, Vec::new())
                } else {
                    None
                }
            }
            b"channel_feed_url" if !arg.is_empty() => op(ChannelFeedUrl, 0, cgi::unescape(&arg)),
            b"channel_feed_url" => None,
            b"clientactive" => op(ClientActive, b, Vec::new()),
            b"yp" => op(Yp, 0, cgi::unescape(&arg)),
            b"deadhitage" => op(DeadHitAge, n, Vec::new()),
            b"refresh" => op(Refresh, n, Vec::new()),
            b"chat" => op(Chat, b, Vec::new()),
            b"randomizechid" => op(RandomizeChid, b, Vec::new()),
            b"public_directory" => op(PublicDirectory, 1, Vec::new()),
            b"auth" => match arg.as_slice() {
                b"cookie" => op(Auth, 1, Vec::new()),
                b"http" => op(Auth, 2, Vec::new()),
                _ => None,
            },
            b"expire" => match arg.as_slice() {
                b"session" => op(Expire, 0, Vec::new()),
                b"never" => op(Expire, 1, Vec::new()),
                _ => None,
            },
            b"logLevel" => op(LogLevel, n, Vec::new()),
            b"allowHTML1" => op(AllowHtml, (n != 0) as i32, Vec::new()),
            b"allowNetwork1" => op(AllowNetwork, (n != 0) as i32, Vec::new()),
            b"allowBroadcast1" => op(AllowBroadcast, (n != 0) as i32, Vec::new()),
            b"allowDirect1" => op(AllowDirect, (n != 0) as i32, Vec::new()),
            b"transcoding_enabled" => op(Transcoding, b, Vec::new()),
            b"preset" => op(Preset, 0, arg.clone()),
            b"audio_codec" => op(AudioCodec, 0, arg.clone()),
            b"preferredTheme" => op(PreferredTheme, 0, arg.clone()),
            b"accentColor" => op(AccentColor, 0, arg.clone()),
            _ => None,
        };
        ops.extend(o);
    }
    ops
}

// ---------------------------------------------------------------- 放送 (ICY、HTTP Push)

/// `readICYHeader` で、ヘッダーの行が何か。`isHeader` は行のどこかに名前が含まれるか
/// (大文字小文字を区別しない) を見るので、前にあるものが優先される。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcyHeader {
    Name = 0,
    Url = 1,
    Bitrate = 2,
    Genre = 3,
    Desc = 4,
    Authorization = 5,
    ChannelId = 6,
    Password = 7,
    ContentType = 8,
    Other = 9,
}

pub fn icy_header(line: &[u8]) -> IcyHeader {
    let has = |names: &[&[u8]]| names.iter().any(|n| stristr(line, n).is_some());
    if has(&[b"x-audiocast-name", b"icy-name", b"ice-name"]) {
        IcyHeader::Name
    } else if has(&[b"x-audiocast-url", b"icy-url", b"ice-url"]) {
        IcyHeader::Url
    } else if has(&[b"x-audiocast-bitrate", b"icy-br", b"ice-bitrate", b"icy-bitrate"]) {
        IcyHeader::Bitrate
    } else if has(&[b"x-audiocast-genre", b"ice-genre", b"icy-genre"]) {
        IcyHeader::Genre
    } else if has(&[b"x-audiocast-description", b"ice-description"]) {
        IcyHeader::Desc
    } else if has(&[b"Authorization"]) {
        IcyHeader::Authorization
    } else if has(&[b"x-peercast-channelid:"]) {
        IcyHeader::ChannelId
    } else if has(&[b"ice-password"]) {
        IcyHeader::Password
    } else if has(&[b"content-type"]) {
        IcyHeader::ContentType
    } else {
        IcyHeader::Other
    }
}

/// `readICYHeader` の content-type。種類 (`ChanInfo::TYPE` の名前) か、PCP のときは `b"PCP"`。
/// 当てはまらなければ `None`。
pub fn icy_content_type(value: &[u8]) -> Option<&'static [u8]> {
    const TABLE: [(&[u8], &[u8]); 12] = [
        (b"application/ogg", b"OGG"),
        (b"application/x-ogg", b"OGG"),
        (b"audio/mpeg", b"MP3"),
        (b"audio/x-mpeg", b"MP3"),
        (b"application/binary", b"RAW"),
        (b"application/x-peercast-pcp", b"PCP"),
        (b"audio/x-scpls", b"PLS"),
        (b"audio/mpegurl", b"PLS"),
        (b"audio/x-mpegurl", b"PLS"),
        (b"audio/m3u", b"PLS"),
        (b"audio/mpegurl", b"PLS"),
        (b"text/plain", b"PLS"),
    ];
    TABLE.iter().find(|(mime, _)| stristr(value, mime).is_some()).map(|&(_, t)| t)
}

// ---------------------------------------------------------------- ローカルのファイル

/// `Servent::fileNameToMimeType` (名前のどこかに拡張子が含まれるか。大文字小文字を区別しない)
pub fn mime_type_for(file_name: &[u8]) -> Option<&'static [u8]> {
    const TABLE: [(&[u8], &[u8]); 7] = [
        (b".htm", b"text/html"),
        (b".css", b"text/css"),
        (b".jpg", b"image/jpeg"),
        (b".gif", b"image/gif"),
        (b".png", b"image/png"),
        (b".js", b"application/javascript; charset=utf-8"),
        (b".ico", b"image/vnd.microsoft.icon"),
    ];
    TABLE.iter().find(|(ext, _)| stristr(file_name, ext).is_some()).map(|&(_, m)| m)
}

/// `handshakeLocalFile` の特別なページ (テンプレートに `channel` を置くもの)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPage {
    /// 視聴ページ。チャンネルのリレーを始めておく
    Play = 0,
    /// relayinfo.html / head.html
    RelayInfo = 1,
    /// connections.html / editinfo.html (ID がなくてもよい)
    Connections = 2,
    Plain = 3,
}

/// `handshakeLocalFile` のページの種類と、`?` の後ろの `id` (`String` に入れたもの)。
/// `?` で 2 つに分かれなければ `split_ok` は false。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    pub page: LocalPage,
    pub split_ok: bool,
    pub id: Vec<u8>,
}

pub fn local_file(fn_: &[u8]) -> LocalFile {
    let page = if find(fn_, b"/play.html").is_some() {
        LocalPage::Play
    } else if find(fn_, b"/relayinfo.html").is_some() || find(fn_, b"/head.html").is_some() {
        LocalPage::RelayInfo
    } else if find(fn_, b"connections.html").is_some() || find(fn_, b"editinfo.html").is_some() {
        LocalPage::Connections
    } else {
        LocalPage::Plain
    };
    let vec = strutil::split(fn_, b"?");
    let split_ok = vec.len() == 2;
    let id = if split_ok {
        let v = Query::new(&vec[1]).get(b"id");
        // String id = ....c_str() (NUL で切れ、255 バイトまで)
        let v = match v.iter().position(|&b| b == 0) {
            Some(n) => v[..n].to_vec(),
            None => v,
        };
        v[..v.len().min(255)].to_vec()
    } else {
        Vec::new()
    };
    LocalFile { page, split_ok, id }
}

/// `handshakeLocalFile` の `String fileName = documentRoot; fileName.append(fn)`。
/// 合わせて 255 バイトに収まらなければ `fn` を足さない (`String::append`)。
pub fn local_file_name(document_root: &[u8], fn_: &[u8]) -> Vec<u8> {
    let mut name = document_root[..document_root.len().min(255)].to_vec();
    if name.len() + fn_.len() < 255 {
        name.extend_from_slice(fn_);
    }
    name
}

// ---------------------------------------------------------------- CGI スクリプト、JSON-RPC

/// `invokeCGIScript` の SERVER_NAME: Host ヘッダーが全体で `^[A-Za-z0-9\-_.]+:\d+$` なら `:` の前。
pub fn cgi_server_name(host: &[u8]) -> Option<Vec<u8>> {
    let colon = host.iter().position(|&c| c == b':')?;
    let (name, port) = (&host[..colon], &host[colon + 1..]);
    let name_ok = !name.is_empty() && name.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'));
    let port_ok = !port.is_empty() && port.iter().all(|c| c.is_ascii_digit());
    if name_ok && port_ok {
        // str::split(host, ":")[0]
        Some(name.to_vec())
    } else {
        None
    }
}

/// CGI スクリプトの出力のヘッダーの行 (`^([A-Za-z\-]+):\s*(.*)$`)。名前と値。
pub fn cgi_header_line(line: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let colon = line.iter().position(|&c| !(c.is_ascii_alphabetic() || c == b'-'))?;
    if colon == 0 || line[colon] != b':' {
        return None;
    }
    let mut v = colon + 1;
    while v < line.len() && matches!(line[v], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        v += 1;
    }
    let value = &line[v..];
    // . は改行 (\n と \r) にだけ一致しない
    if value.iter().any(|&c| c == b'\n' || c == b'\r') {
        return None;
    }
    Some((line[..colon].to_vec(), value.to_vec()))
}

/// `handshakeJRPC` の本体の長さ。`Err` は返す状態の行と番号 (411、400、413)。
pub fn jrpc_body_length(content_length: &[u8], max: i32) -> Result<i32, (&'static str, i32)> {
    if content_length.is_empty() {
        return Err(("HTTP/1.0 411 Length required", 411));
    }
    let n = atoi(content_length);
    if n == -1 {
        return Err(("HTTP/1.0 411 Length required", 411));
    }
    if n <= 0 {
        return Err(("HTTP/1.0 400 Bad Request", 400));
    }
    if n > max {
        return Err(("HTTP/1.0 413 Request Entity Too Large", 413));
    }
    Ok(n)
}

#[cfg(test)]
mod tests;
