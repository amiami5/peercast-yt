//! 管理画面の掲示板ビューワー (もとは Python の ui/cgi-bin/bbs_reader.py、board.cgi、thread.cgi、
//! post.cgi)。したらば (jbbs.shitaraba.net) と 2ch 形式の掲示板の、板の設定とスレッドの一覧、
//! スレッドの書き込みを読み、書き込みを送る。
//!
//! ここは取得した中身の解釈と、要求の組み立てと、JSON の組み立てだけを行う。取得は `Fetch` を通す
//! (サーバーでは `server::bbs_http` の、公開アドレスにしか接続しない HTTP クライアント)。
//! 結果は、もとの Python 版と同じになるようにしている (`py` モジュールと `codec` モジュール)。

pub mod codec;
pub mod py;

use codec::{Codec, Errors};

/// もとの Python 版の例外に当たるもの。スクリプトが途中で落ちたときと同じく 500 にする
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Error(m.into()))
}

/// 取得の結果
pub enum Fetched {
    /// 2xx の応答の本体
    Ok(Vec<u8>),
    /// 2xx 以外の応答 (`urllib.error.HTTPError`)
    Status(u16),
}

/// 取得するもの (`safe_urlopen`)。接続できない、公開アドレスでない、大きすぎるなどは `Err`
pub trait Fetch {
    fn get(&mut self, url: &str) -> Result<Fetched>;
    /// `application/x-www-form-urlencoded` の本体を POST する
    fn post(&mut self, url: &str, body: &[u8], referer: &str) -> Result<Fetched>;
}

// ---------------------------------------------------------------- 引数の検査

fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// `^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?(?::[0-9]{1,5})?$`
fn host_ok(s: &str) -> bool {
    let (host, port) = match s.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (s, None),
    };
    if let Some(p) = port {
        if p.is_empty() || p.len() > 5 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let b = host.as_bytes();
    if b.is_empty() || b.len() > 253 || !is_alnum(b[0]) || !is_alnum(b[b.len() - 1]) {
        return false;
    }
    b.iter().all(|&c| is_alnum(c) || c == b'.' || c == b'-')
}

/// `^[A-Za-z0-9_.-]{1,64}$`
fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|c| is_alnum(c) || matches!(c, b'_' | b'.' | b'-'))
}

/// `^[0-9]{1,20}$`
fn num_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|c| c.is_ascii_digit())
}

/// `check_params`: 不正なら理由
pub fn check_params(fqdn: &str, category: &str, board_num: &str, thread_id: Option<&str>) -> Option<&'static str> {
    if !host_ok(fqdn) {
        return Some("bad fqdn");
    }
    if !name_ok(category) || category == "." || category == ".." {
        return Some("bad category");
    }
    if !board_num.is_empty() && !num_ok(board_num) {
        return Some("bad board_num");
    }
    if let Some(id) = thread_id {
        if !num_ok(id) {
            return Some("bad id");
        }
    }
    None
}

// ---------------------------------------------------------------- 板とスレッド

pub struct Board {
    pub fqdn: String,
    pub shitaraba: bool,
    pub category: String,
    pub board_num: String,
    pub resmax: i64,
    urlpath: String,
    encoding: Codec,
}

pub struct Thread {
    pub id: String,
    pub title: String,
    pub last: i64,
}

pub struct Post {
    pub no: i64,
    pub name: String,
    pub mail: String,
    pub date: String,
    pub body: String,
}

impl Board {
    pub fn new(fqdn: &str, category: &str, board_num: &str) -> Result<Board> {
        if let Some(e) = check_params(fqdn, category, board_num, None) {
            return err(e);
        }
        let shitaraba = fqdn.contains("jbbs.shitaraba.net");
        Ok(Board {
            fqdn: fqdn.to_string(),
            shitaraba,
            category: category.to_string(),
            board_num: board_num.to_string(),
            resmax: 1000,
            urlpath: if board_num.is_empty() { category.to_string() } else { format!("{}/{}", category, board_num) },
            encoding: if shitaraba { Codec::EucJp } else { Codec::Cp932 },
        })
    }

    fn settings_url(&self) -> String {
        if self.shitaraba {
            format!("http://{}/bbs/api/setting.cgi/{}", self.fqdn, self.urlpath)
        } else {
            format!("http://{}/{}/SETTING.TXT", self.fqdn, self.urlpath)
        }
    }

    fn thread_list_url(&self) -> String {
        format!("http://{}/{}/subject.txt", self.fqdn, self.urlpath)
    }

    pub fn dat_url(&self, thread_num: &str) -> String {
        if self.shitaraba {
            format!("http://{}/bbs/rawmode.cgi/{}/{}/", self.fqdn, self.urlpath, thread_num)
        } else {
            format!("http://{}/{}/dat/{}.dat", self.fqdn, self.urlpath, thread_num)
        }
    }

    fn download(&self, f: &mut dyn Fetch, url: &str) -> Result<Fetched> {
        f.get(url)
    }

    /// `settings`: 板の設定 (キーは小文字)。取れなければ `error` だけのもの
    pub fn settings(&self, f: &mut dyn Fetch) -> Result<Vec<(String, String)>> {
        let body = match self.download(f, &self.settings_url())? {
            Fetched::Ok(b) => b,
            Fetched::Status(_) => return Ok(vec![("error".to_string(), "[Settings download error]".to_string())]),
        };
        let text = match codec::decode(self.encoding, &body, Errors::Strict) {
            Some(t) => t,
            None => String::from_utf8(body).or_else(|_| err("settings: not UTF-8"))?,
        };
        parse_settings(&text)
    }

    /// `thread_list`: subject.txt (したらばでスレッドがまだなければ 404 になるので空)
    fn thread_list(&self, f: &mut dyn Fetch) -> Result<String> {
        let body = match self.download(f, &self.thread_list_url())? {
            Fetched::Ok(b) => b,
            Fetched::Status(_) => Vec::new(),
        };
        codec::decode(self.encoding, &body, Errors::Strict).ok_or_else(|| Error("subject.txt: decode error".into()))
    }

    /// `threads`: スレッドの一覧。`resmax` も更新する
    pub fn threads(&mut self, f: &mut dyn Fetch) -> Result<Vec<Thread>> {
        let text = self.thread_list(f)?;
        let mut threads = Vec::new();
        for line in py::splitlines(&text) {
            if line.is_empty() {
                // したらばでスレッドがないときは空行だけになる
                continue;
            }
            let (id, title, count) = match parse_subject_line(line, self.shitaraba) {
                Some(m) => m,
                None => return err(format!("subject.txt: unexpected line {:?}", line)),
            };
            let count = py::int(count).ok_or_else(|| Error("subject.txt: bad count".into()))?;
            self.resmax = self.resmax.max(count);
            threads.push(Thread { id: id.to_string(), title: py::html_unescape(title), last: count });
        }
        if self.shitaraba && threads.len() > 1 {
            threads.pop();
        }
        Ok(threads)
    }

    /// `thread`: 番号のスレッド
    pub fn thread(&mut self, f: &mut dyn Fetch, thread_num: &str) -> Result<Option<Thread>> {
        Ok(self.threads(f)?.into_iter().find(|t| t.id == thread_num))
    }

    /// `Thread.posts(range(first, resmax))`
    pub fn posts(&self, f: &mut dyn Fetch, thread: &mut Thread, first: i64) -> Result<Vec<Post>> {
        let dat = self.dat_for_range(f, thread, first, self.resmax)?;
        let mut posts = Vec::new();
        for (i, line) in py::splitlines(&dat).into_iter().enumerate() {
            let mut p = parse_post(line, self.shitaraba)?;
            if p.no == 0 {
                p.no = i as i64 + first;
            }
            thread.last = thread.last.max(p.no);
            posts.push(p);
        }
        Ok(posts)
    }

    fn dat_for_range(&self, f: &mut dyn Fetch, thread: &Thread, start: i64, stop: i64) -> Result<String> {
        let url = self.dat_url(&thread.id);
        if self.shitaraba {
            let query = if stop >= self.resmax { format!("{}-", start) } else { format!("{}-{}", start, stop) };
            let body = match self.download(f, &format!("{}{}", url, query))? {
                Fetched::Ok(b) => b,
                Fetched::Status(c) => return err(format!("HTTP Error {}", c)),
            };
            Ok(codec::decode(self.encoding, &body, Errors::Replace).unwrap_or_default())
        } else {
            let body = match self.download(f, &url)? {
                Fetched::Ok(b) => b,
                Fetched::Status(c) => return err(format!("HTTP Error {}", c)),
            };
            let text = codec::decode(self.encoding, &body, Errors::Replace).unwrap_or_default();
            let lines = py::splitlines(&text);
            // lines[start-1:] (負の数は後ろから数える)
            let n = lines.len() as i64;
            let mut from = start - 1;
            if from < 0 {
                from = (n + from).max(0);
            }
            let from = from.min(n) as usize;
            Ok(lines[from..].iter().map(|l| format!("{}\n", l)).collect())
        }
    }
}

/// subject.txt の 1 行: したらばは `^(\d+)\.cgi,(.+?)\((\d+)\)$`、2ch は `^(\d+)\.dat<>(.+?)\s\((\d+)\)$`。
/// (番号、題名、書き込み数)
fn parse_subject_line(line: &str, shitaraba: bool) -> Option<(&str, &str, &str)> {
    let n = line.char_indices().find(|&(_, c)| !py::is_digit(c)).map_or(line.len(), |(i, _)| i);
    if n == 0 {
        return None;
    }
    let id = &line[..n];
    let rest = line[n..].strip_prefix(if shitaraba { ".cgi," } else { ".dat<>" })?;
    // 末尾は "(" 数字 ")"。数字に "(" は含まれないので、最後の "(" から
    let body = rest.strip_suffix(')')?;
    let open = body.rfind('(')?;
    let count = &body[open + 1..];
    if count.is_empty() || !count.chars().all(py::is_digit) {
        return None;
    }
    let mut title = &body[..open];
    if !shitaraba {
        // (.+?)\s: 題名の後ろの空白 1 文字
        let last = title.chars().next_back()?;
        if !py::is_space(last) {
            return None;
        }
        title = &title[..title.len() - last.len_utf8()];
    }
    if title.is_empty() {
        return None;
    }
    Some((id, title, count))
}

/// `Post.from_line`: したらばは `番号<>名前<>メール<>日付<>本文<>...`、2ch は `名前<>メール<>日付<>本文<>...`
fn parse_post(line: &str, shitaraba: bool) -> Result<Post> {
    let (maxsplit, need) = if shitaraba { (6, 5) } else { (4, 4) };
    let fields: Vec<&str> = line.splitn(maxsplit + 1, "<>").collect();
    if fields.len() < need {
        return err("dat: too few fields");
    }
    let (no, f) = if shitaraba {
        (py::int(fields[0]).ok_or_else(|| Error("dat: bad number".into()))?, &fields[1..])
    } else {
        (0, &fields[..])
    };
    Ok(Post { no, name: f[0].to_string(), mail: f[1].to_string(), date: f[2].to_string(), body: f[3].to_string() })
}

/// SETTING.TXT を `configparser` で `[DEFAULT]` として読んだもの (キーは小文字、値の前後の空白は除く)。
/// 区切りは最初の `=` か `:`。`#` と `;` で始まる行と空行は読み飛ばし、空白で始まる行は前の値の続き
/// (改行でつなぐ)。区切りのない行、同じキー、`[...]` の後ろ (DEFAULT でない節) は、configparser と同じく
/// 誤りにするか、DEFAULT に入れない。
pub fn parse_settings(text: &str) -> Result<Vec<(String, String)>> {
    let mut res: Vec<(String, String)> = Vec::new();
    let mut in_default = true;
    let mut last: Option<usize> = None;
    let mut seen: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let stripped = line.trim_matches(py::is_space);
        if stripped.is_empty() {
            // 空行は値の続きを終わらせる (configparser は空行も値に含めうるが、末尾の空行は除かれる)
            last = None;
            continue;
        }
        if stripped.starts_with('#') || stripped.starts_with(';') {
            continue;
        }
        let indented = line.starts_with(|c: char| py::is_space(c));
        if indented {
            if let Some(i) = last {
                if in_default {
                    res[i].1.push('\n');
                    res[i].1.push_str(stripped);
                }
                continue;
            }
        }
        if stripped.starts_with('[') && stripped.ends_with(']') {
            in_default = &stripped[1..stripped.len() - 1] == "DEFAULT";
            last = None;
            continue;
        }
        let pos = stripped.find(['=', ':']);
        let pos = match pos {
            Some(p) => p,
            None => return err(format!("settings: no delimiter in {:?}", stripped)),
        };
        let key = stripped[..pos].trim_matches(py::is_space).to_lowercase();
        let value = stripped[pos + 1..].trim_matches(py::is_space).to_string();
        if key.is_empty() {
            return err("settings: empty key");
        }
        if seen.contains(&key) {
            return err(format!("settings: duplicate option {:?}", key));
        }
        seen.push(key.clone());
        if in_default {
            res.push((key, value));
            last = Some(res.len() - 1);
        } else {
            last = None;
        }
    }
    Ok(res)
}

// ---------------------------------------------------------------- 書き込み

/// `urllib.parse.quote_plus`
fn quote_plus(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => s.push(c as char),
            b' ' => s.push('+'),
            _ => s.push_str(&format!("%{:02X}", c)),
        }
    }
    s
}

/// `urllib.parse.urlencode` (値はバイト列)
fn urlencode(fields: &[(&str, Vec<u8>)]) -> Vec<u8> {
    fields.iter().map(|(k, v)| format!("{}={}", quote_plus(k.as_bytes()), quote_plus(v))).collect::<Vec<_>>().join("&").into_bytes()
}

/// 書き込みの結果 (JSON にするもの)
pub enum PostResult {
    Ok,
    Error(u16),
}

/// `post.cgi`: 書き込む
pub fn post_message(f: &mut dyn Fetch, fqdn: &str, category: &str, board_num: &str, thread_id: &str, name: &str, mail: &str, body: &str) -> Result<PostResult> {
    if fqdn.contains("shitaraba") {
        let url = format!("https://{}/bbs/write.cgi/{}/{}/{}/", fqdn, category, board_num, thread_id);
        let referer = format!("https://{}/bbs/read.cgi/{}/{}/{}/", fqdn, category, board_num, thread_id);
        let e = |s: &str| codec::encode_xmlcharref(Codec::EucJp, s);
        let data = urlencode(&[
            ("BBS", board_num.as_bytes().to_vec()),
            ("KEY", thread_id.as_bytes().to_vec()),
            ("DIR", category.as_bytes().to_vec()),
            ("NAME", e(name)),
            ("MAIL", e(mail)),
            ("MESSAGE", e(body)),
        ]);
        match f.post(&url, &data, &referer)? {
            Fetched::Ok(_) => Ok(PostResult::Ok),
            // したらばは HTTPError を捕まえていなかった (スクリプトが落ちる)
            Fetched::Status(c) => err(format!("HTTP Error {}", c)),
        }
    } else {
        let url = format!("http://{}/test/bbs.cgi", fqdn);
        let referer = format!("http://{}/{}/", fqdn, category);
        let e = |s: &str| codec::encode_xmlcharref(Codec::ShiftJis, s);
        let data = urlencode(&[
            ("FROM", e(name)),
            ("mail", e(mail)),
            ("MESSAGE", e(body)),
            ("bbs", category.as_bytes().to_vec()),
            ("key", thread_id.as_bytes().to_vec()),
            ("submit", e("書き込む")),
        ]);
        match f.post(&url, &data, &referer)? {
            Fetched::Ok(_) => Ok(PostResult::Ok),
            Fetched::Status(c) => Ok(PostResult::Error(c)),
        }
    }
}

// ---------------------------------------------------------------- JSON (Python の json.dumps と同じ形)

pub enum Json {
    Str(String),
    Int(i64),
    Arr(Vec<Json>),
    Obj(Vec<(&'static str, Json)>),
}

/// `json.dumps` (ensure_ascii、区切りは ", " と ": ")
pub fn dumps(v: &Json) -> String {
    let mut out = String::new();
    write_json(v, &mut out);
    out
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json(v: &Json, out: &mut String) {
    match v {
        Json::Str(s) => write_str(s, out),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json(x, out);
            }
            out.push(']');
        }
        Json::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_str(k, out);
                out.push_str(": ");
                write_json(x, out);
            }
            out.push('}');
        }
    }
}

fn s(v: impl Into<String>) -> Json {
    Json::Str(v.into())
}

/// `board.cgi` の結果
pub fn board_json(f: &mut dyn Fetch, fqdn: &str, category: &str, board_num: &str) -> Result<String> {
    let mut board = Board::new(fqdn, category, board_num)?;
    let settings = board.settings(f)?;
    let get = |k: &str| settings.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    let title = match get("error") {
        Some(e) => e,
        None => get("bbs_title").ok_or_else(|| Error("settings: no bbs_title".into()))?,
    };
    let threads: Vec<Json> = board
        .threads(f)?
        .into_iter()
        .map(|t| Json::Obj(vec![("id", s(t.id)), ("title", s(t.title)), ("last", Json::Int(t.last))]))
        .collect();
    Ok(dumps(&Json::Obj(vec![
        ("status", s("ok")),
        ("title", s(title)),
        ("threads", Json::Arr(threads)),
        ("category", s(board.category.clone())),
        ("board_num", s(board.board_num.clone())),
    ])))
}

/// `thread.cgi` の結果
pub fn thread_json(f: &mut dyn Fetch, fqdn: &str, category: &str, board_num: &str, id: &str, first: i64) -> Result<String> {
    let mut board = Board::new(fqdn, category, board_num)?;
    let mut thread = board.thread(f, id)?.ok_or_else(|| Error("thread not found".into()))?;
    let posts: Vec<Json> = board
        .posts(f, &mut thread, first)?
        .into_iter()
        .map(|p| Json::Obj(vec![("no", Json::Int(p.no)), ("name", s(p.name)), ("mail", s(p.mail)), ("body", s(p.body)), ("date", s(p.date))]))
        .collect();
    Ok(dumps(&Json::Obj(vec![
        ("status", s("ok")),
        ("id", s(thread.id.clone())),
        ("title", s(thread.title.clone())),
        ("last", Json::Int(thread.last)),
        ("posts", Json::Arr(posts)),
    ])))
}

/// `post.cgi` の結果
pub fn post_json(r: &PostResult) -> String {
    match r {
        PostResult::Ok => dumps(&Json::Obj(vec![("status", s("ok"))])),
        PostResult::Error(c) => dumps(&Json::Obj(vec![("status", s("error")), ("code", Json::Int(*c as i64))])),
    }
}

// ---------------------------------------------------------------- 要求と応答

/// 要求の引数 (もとは cgi.FieldStorage)。値が空の引数は無いものとし、同じ名前は最初のものを使う。
/// `+` は空白、`%XX` は UTF-8 として (不正なバイトは U+FFFD にして) 読む
pub struct Form(Vec<(String, String)>);

fn unquote_plus(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(h, 16).unwrap_or(0));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Form {
    pub fn parse(query: &[u8]) -> Form {
        let q = String::from_utf8_lossy(query);
        let mut v: Vec<(String, String)> = Vec::new();
        for part in q.split('&') {
            // 区切りのないもの (値が空) と、値が空のものは入れない
            let (k, val) = match part.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            if val.is_empty() {
                continue;
            }
            let k = unquote_plus(k);
            let val = unquote_plus(val);
            if !v.iter().any(|(x, _)| *x == k) {
                v.push((k, val));
            }
        }
        Form(v)
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
    }
}

/// 応答
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

fn bad_request(message: &str) -> Reply {
    Reply { status: 400, content_type: "text/plain", body: format!("{}\n", message).into_bytes() }
}

fn json_reply(json: String) -> Reply {
    Reply { status: 200, content_type: "application/json; charset=UTF-8", body: format!("{}\n", json).into_bytes() }
}

/// `/cgi-bin/<script>` の要求 (board.cgi、thread.cgi、post.cgi)。`Err` は 500 にする
pub fn handle(script: &str, query: &[u8], f: &mut dyn Fetch) -> Result<Reply> {
    let form = Form::parse(query);
    let get = |k: &str| form.get(k);
    match script {
        "board.cgi" => {
            let (fqdn, category) = match (get("fqdn"), get("category")) {
                (Some(a), Some(b)) => (a, b),
                _ => return Ok(bad_request("bad parameter")),
            };
            let board_num = get("board_num").unwrap_or("");
            if let Some(e) = check_params(fqdn, category, board_num, None) {
                return Ok(bad_request(e));
            }
            Ok(json_reply(board_json(f, fqdn, category, board_num)?))
        }
        "thread.cgi" => {
            let (fqdn, category, id) = match (get("fqdn"), get("category"), get("id")) {
                (Some(a), Some(b), Some(c)) => (a, b, c),
                _ => return Ok(bad_request("bad parameter")),
            };
            let first = match get("first") {
                None => 1,
                Some(v) => py::int(v).ok_or_else(|| Error("bad first".into()))?,
            };
            let board_num = get("board_num").unwrap_or("");
            if let Some(e) = check_params(fqdn, category, board_num, Some(id)) {
                return Ok(bad_request(e));
            }
            Ok(json_reply(thread_json(f, fqdn, category, board_num, id, first)?))
        }
        "post.cgi" => {
            for key in ["fqdn", "category", "id", "body"] {
                if get(key).is_none() {
                    return Ok(bad_request(key));
                }
            }
            let (fqdn, category, id, body) = (get("fqdn").unwrap_or(""), get("category").unwrap_or(""), get("id").unwrap_or(""), get("body").unwrap_or(""));
            let board_num = get("board_num").unwrap_or("");
            if let Some(e) = check_params(fqdn, category, board_num, Some(id)) {
                return Ok(bad_request(e));
            }
            let r = post_message(f, fqdn, category, board_num, id, get("name").unwrap_or(""), get("mail").unwrap_or(""), body)?;
            Ok(json_reply(post_json(&r)))
        }
        _ => err("no such script"),
    }
}

#[cfg(test)]
mod tests;
