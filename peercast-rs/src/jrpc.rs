//! JSON-RPC の API (core/common/jrpc.cpp の `JrpcApi`) (段階 8a)。
//!
//! 要求の解釈 (JSON の構文解析、`jsonrpc` / `id` / `method` / `params` の検査、メソッドの選び方、
//! 名前付き引数の並べ替え、引数の型の変換) と、結果の JSON の組み立てを行う。チャンネルやサーバーの
//! 状態 (`chanMgr`、`servMgr`、ログなど) は `Host` トレイトを通してだけ触る。
//!
//! 引数の型の変換 (`get<int>()` など) とその失敗の文言、評価の順序は C++ 版と同じにしてある。
//! どの時点で状態を触るか (途中で例外になったときに、どこまで変わっているか) も同じ。

use crate::chanhit::{self, Hit};
use crate::json::{self, Object, Value};
use crate::{chaninfo, gnuid, hostgraph, strutil, utf8};

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
pub const CHANNEL_NOT_FOUND: i32 = -1;
pub const UNKNOWN_ERROR: i32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// C++ の関数が投げた例外 (`what()`)
#[derive(Clone, Debug, PartialEq)]
pub enum HostError {
    Exception(Vec<u8>),
    /// `std::domain_error` (`fetch` はこれを `invalid_params` にする)
    DomainError(Vec<u8>),
}

pub type HostResult<T> = Result<T, HostError>;

/// `ChanInfo` のうち、ここで使う欄
#[derive(Clone, Debug, Default)]
pub struct InfoData {
    pub id: [u8; 16],
    pub name: Vec<u8>,
    pub content_type: Vec<u8>,
    pub mime: Vec<u8>,
    pub desc: Vec<u8>,
    pub genre: Vec<u8>,
    pub url: Vec<u8>,
    pub comment: Vec<u8>,
    pub bitrate: i32,
    pub track_contact: Vec<u8>,
    pub track_title: Vec<u8>,
    pub track_artist: Vec<u8>,
    pub track_album: Vec<u8>,
    pub track_genre: Vec<u8>,
}

/// `Channel` のうち、ここで使う値
#[derive(Clone, Debug, Default)]
pub struct ChannelData {
    pub info: InfoData,
    pub status: i32,
    pub source_url: Vec<u8>,
    /// `sourceHost.host.str()`
    pub source_host: Vec<u8>,
    /// `info.getUptime()`
    pub uptime: u32,
    pub local_relays: i32,
    pub local_directs: i32,
    pub total_relays: i32,
    pub total_directs: i32,
    pub is_broadcasting: bool,
    pub is_full: bool,
    pub is_receiving: bool,
    pub ip_version: i32,
    /// `sock ? sock->host.str() : null`
    pub sock_host: Option<Vec<u8>>,
    /// `sourceData ? sourceData->getSourceRate() : 0`
    pub source_rate: i32,
    pub src_protocol: i32,
    pub stream_pos: u32,
}

/// `Servent` のうち、ここで使う値
#[derive(Clone, Debug, Default)]
pub struct ServentData {
    pub index: i32,
    pub type_str: Vec<u8>,
    pub status_str: Vec<u8>,
    pub send_rate: u32,
    pub recv_rate: u32,
    pub protocol: i32,
    pub agent: Vec<u8>,
    pub sock_host: Option<Vec<u8>>,
}

/// `fetch` で作るチャンネル
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub url: Vec<u8>,
    pub name: Vec<u8>,
    pub desc: Vec<u8>,
    pub genre: Vec<u8>,
    pub contact: Vec<u8>,
    pub bitrate: i32,
    pub type_str: Vec<u8>,
    pub ipv6: bool,
}

/// `getChannelRelayTree` に使うもの
#[derive(Clone, Debug)]
pub enum RelayTreeData {
    NoChannel,
    NoHitList,
    /// 自分 (`initLocal` したもの) とヒットリストの並び。`rhost[0].ip.str()` を添える。
    Hits(Vec<(Hit, Vec<u8>)>),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub max_relays: u32,
    pub max_relays_per_channel: i32,
    pub max_direct: u32,
    pub max_bitrate_out: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    MaxRelays,
    MaxRelaysPerChannel,
    MaxDirect,
    MaxBitrateOut,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub uptime: u32,
    /// `ServMgr::FW_STATE`
    pub firewall: i32,
    /// `serverHost.IPtoStr()`
    pub global_ip: Vec<u8>,
    pub port: u16,
    /// `serverLocalIP.str()`
    pub local_ip: Vec<u8>,
}

/// `ChannelEntry`
#[derive(Clone, Debug, Default)]
pub struct YpEntry {
    pub feed_url: Vec<u8>,
    pub name: Vec<u8>,
    pub id: [u8; 16],
    pub tip: Vec<u8>,
    pub url: Vec<u8>,
    pub genre: Vec<u8>,
    pub desc: Vec<u8>,
    pub comment: Vec<u8>,
    pub bitrate: i32,
    pub content_type: Vec<u8>,
    pub track_name: Vec<u8>,
    pub track_album: Vec<u8>,
    pub track_artist: Vec<u8>,
    pub track_contact: Vec<u8>,
    pub num_directs: i32,
    pub num_relays: i32,
}

/// `ChanHitList` (`getChannelsFound`)
#[derive(Clone, Debug, Default)]
pub struct FoundData {
    pub info: InfoData,
    pub uptime: u32,
    pub skips: u32,
    pub age: u32,
    pub bcflags: u8,
    pub hosts: i32,
    pub listeners: i32,
    pub relays: i32,
    pub firewalled: i32,
    pub closest: i32,
    pub furthest: i32,
    /// `sys->getTime() - newestHit()`
    pub newest: u32,
    /// IP アドレスが設定されているヒット
    pub hits: Vec<FoundHit>,
}

/// `ChanHit` (`getChannelsFound`)
#[derive(Clone, Debug, Default)]
pub struct FoundHit {
    /// `host.str()`
    pub ip: Vec<u8>,
    pub hops: u32,
    pub listeners: u32,
    pub relays: u32,
    pub uptime: u32,
    pub push: bool,
    pub relay: bool,
    pub direct: bool,
    pub cin: bool,
    pub stable: bool,
    pub version: u32,
    /// `sys->getTime() - time`
    pub update: u32,
    pub tracker: bool,
}

/// `getState` で渡せる名前 (この順の番号で `Host::state` を呼ぶ)
pub const STATE_NAMES: [&[u8]; 6] = [b"servMgr", b"chanMgr", b"stats", b"notificationBuffer", b"sys", b"ypList"];

/// サーバーの状態を触るもの。C++ 側 (`core/common/rustjrpc.h`) は、C++ 版の同じ箇所を写したもの。
pub trait Host {
    fn log(&mut self, level: Level, msg: &[u8]);
    /// `PCX_AGENT`
    fn agent(&mut self) -> Vec<u8>;
    /// ログの各行 (時刻と種類を付けたもの)
    fn log_lines(&mut self) -> HostResult<Vec<Vec<u8>>>;
    fn clear_log(&mut self) -> HostResult<()>;
    fn log_level(&mut self) -> HostResult<i32>;
    fn set_log_level(&mut self, level: i32) -> HostResult<()>;
    /// チャンネルを作って配信元に接続する。作れなければ `None`。作ったチャンネルの ID を返す。
    fn fetch(&mut self, req: &FetchRequest) -> HostResult<Option<[u8; 16]>>;
    /// `chanMgr->channel` の並び
    fn channels(&mut self) -> HostResult<Vec<ChannelData>>;
    fn find_channel(&mut self, id: &[u8; 16]) -> HostResult<Option<ChannelData>>;
    /// `chanID` が `id` のサーバント
    fn servents(&mut self, id: &[u8; 16]) -> HostResult<Vec<ServentData>>;
    /// チャンネルの中継の接続を止める。止めたら true。
    fn stop_connection(&mut self, id: &[u8; 16], connection_id: i32) -> HostResult<bool>;
    fn relay_tree(&mut self, id: &[u8; 16]) -> HostResult<RelayTreeData>;
    /// `bump` を立てる。チャンネルがなければ false。
    fn bump(&mut self, id: &[u8; 16]) -> HostResult<bool>;
    fn play(&mut self, id: &[u8; 16]) -> HostResult<()>;
    /// チャンネルがあれば、そのスレッドを止める。
    fn stop_channel(&mut self, id: &[u8; 16]) -> HostResult<()>;
    fn root_host(&mut self) -> HostResult<Vec<u8>>;
    fn clear_root_host(&mut self) -> HostResult<()>;
    fn settings(&mut self) -> HostResult<Settings>;
    fn set_setting(&mut self, key: SettingKey, value: i32) -> HostResult<()>;
    fn status(&mut self) -> HostResult<Status>;
    /// `STATE_NAMES[which]` の `getState().inspect()`
    fn state(&mut self, which: usize) -> HostResult<Vec<u8>>;
    /// チャンネルの情報を、`fields` (name, desc, genre, url, comment, track の contact, title,
    /// artist, album, genre の順。`String` に代入する前のもの) で `updateInfo` する。
    fn update_info(&mut self, id: &[u8; 16], fields: &[Vec<u8>; 10]) -> HostResult<()>;
    fn yp_channels(&mut self) -> HostResult<Vec<YpEntry>>;
    /// 状態のファイル (`<key>.json`) の中身。開けなければ `None`。
    fn read_storage(&mut self, key: &[u8]) -> HostResult<Option<Vec<u8>>>;
    fn write_storage(&mut self, key: &[u8], value: &[u8]) -> HostResult<()>;
    /// 使われているヒットリスト (`isUsed`)
    fn channels_found(&mut self) -> HostResult<Vec<FoundData>>;
}

/// メソッドから飛ぶ例外。`what()` の文字列を持つ。
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
    MethodNotFound(Vec<u8>),
    InvalidParams(Vec<u8>),
    Application(i32, Vec<u8>),
    /// それ以外の `std::exception`
    Internal(Vec<u8>),
}

impl From<HostError> for CallError {
    fn from(e: HostError) -> Self {
        match e {
            HostError::Exception(w) | HostError::DomainError(w) => CallError::Internal(w),
        }
    }
}

/// `json` の変換の失敗 (`type_error` など) は、`std::exception` として扱われる。
fn te<T>(r: Result<T, Vec<u8>>) -> Result<T, CallError> {
    r.map_err(CallError::Internal)
}

type CallResult = Result<Value, CallError>;

/// `what()` は C の文字列なので、NUL の手前までになる。
fn c_str(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `String` に代入したときの中身 (NUL の手前まで、255 バイトまで)
fn pc_string(s: &[u8]) -> Vec<u8> {
    let s = c_str(s);
    s[..s.len().min(255)].to_vec()
}

/// `GnuID(std::string)`
fn gnuid_of(s: &[u8]) -> [u8; 16] {
    gnuid::from_str(c_str(s))
}

fn id_value(id: &[u8; 16]) -> Value {
    Value::str(&gnuid::to_str(id))
}

/// `str::valid_utf8`
fn vu(s: &[u8]) -> Value {
    Value::Str(utf8::valid(s))
}

fn uint(u: u32) -> Value {
    Value::UInt(u as u64)
}

fn int(i: i32) -> Value {
    Value::Int(i as i64)
}

// ---------------------------------------------------------------- 呼び出し

/// JSON のネストの深さ (`[` と `{` の入れ子) が max を超えていないかを調べる。
/// 文字列リテラルの中の括弧は数えない。
fn json_nesting_is_too_deep(s: &[u8], max: i32) -> bool {
    let mut depth = 0;
    let mut in_string = false;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if in_string {
            if c == b'\\' {
                i += 1; // エスケープされた文字を飛ばす
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == b'[' || c == b'{' {
            depth += 1;
            if depth > max {
                return true;
            }
        } else if (c == b']' || c == b'}') && depth > 0 {
            depth -= 1;
        }
        i += 1;
    }
    false
}

fn error_object(code: i32, message: &[u8], id: Value, data: Value) -> Value {
    let mut err = Object::new();
    err.insert(b"code".to_vec(), int(code));
    err.insert(b"message".to_vec(), Value::str(c_str(message)));
    if !data.is_null() {
        err.insert(b"data".to_vec(), data);
    }
    Value::object([("jsonrpc", Value::str(b"2.0")), ("error", Value::Object(err)), ("id", id)])
}

/// `JrpcApi::call`。応答の JSON を返す。応答を書き出せなかったときは、その例外の `what()` を返す
/// (C++ 版は例外がそのまま上に飛ぶ)。
pub fn call(input: &[u8], host: &mut dyn Host) -> Result<Vec<u8>, Vec<u8>> {
    let result = json::dump(&call_internal(input, host))?;
    let mut msg = b"jrpc response: ".to_vec();
    msg.extend_from_slice(&utf8::truncate(&result, 60).unwrap_or_default());
    host.log(Level::Debug, &msg);
    Ok(result)
}

fn call_internal(input: &[u8], host: &mut dyn Host) -> Value {
    let mut msg = b"jrpc request: ".to_vec();
    msg.extend_from_slice(input);
    host.log(Level::Debug, &msg);

    // json のコピーやデストラクタは入れ子の深さだけ再帰するので、深い入力で
    // スタックを使い果たして落ちる。正当なリクエストは数段しかない。
    if json_nesting_is_too_deep(input, 64) {
        return error_object(PARSE_ERROR, b"Parse error", Value::Null, Value::Null);
    }

    // C++ 版は、有限でない数 (1e400 など) の out_of_range を捕まえておらず、例外が上に飛んで
    // 応答を返さなかった。Rust 版は、ほかの構文の誤りと同じく Parse error にする。
    let j = match json::parse(input) {
        Ok(j) => j,
        Err(_) => return error_object(PARSE_ERROR, b"Parse error", Value::Null, Value::Null),
    };

    let obj = match &j {
        Value::Object(o) if o.get(&b"jsonrpc"[..]).map_or(false, |v| v.is_str(b"2.0")) => o,
        _ => return error_object(INVALID_REQUEST, b"Invalid Request", Value::Null, Value::Null),
    };

    let id = match obj.get(&b"id"[..]) {
        Some(id) => id.clone(),
        None => return error_object(INVALID_REQUEST, b"Invalid Request", Value::Null, Value::Null),
    };

    let method = match obj.get(&b"method"[..]) {
        Some(m) => m,
        None => return error_object(INVALID_REQUEST, b"Invalid Request", id, Value::Null),
    };

    let params = match obj.get(&b"params"[..]) {
        None => Value::Array(Vec::new()),
        Some(p @ (Value::Object(_) | Value::Array(_))) => p.clone(),
        Some(_) => {
            return error_object(
                INVALID_PARAMS,
                b"Invalid params",
                id,
                Value::str(b"params must be either object or array"),
            )
        }
    };

    let result = dispatch(method, &params, host).and_then(|r| {
        // 書き出せるかどうかを確かめる (!)
        te(json::dump(&r))?;
        Ok(r)
    });
    match result {
        Ok(r) => Value::object([("jsonrpc", Value::str(b"2.0")), ("result", r), ("id", id)]),
        Err(CallError::MethodNotFound(what)) => {
            let what = c_str(&what).to_vec();
            let mut msg = b"Method not found: ".to_vec();
            msg.extend_from_slice(&what);
            host.log(Level::Debug, &msg);
            error_object(METHOD_NOT_FOUND, b"Method not found", id, Value::Str(what))
        }
        Err(CallError::InvalidParams(what)) => {
            error_object(INVALID_PARAMS, b"Invalid params", id, Value::str(c_str(&what)))
        }
        Err(CallError::Application(code, what)) => error_object(code, &what, id, Value::Null),
        Err(CallError::Internal(what)) => error_object(INTERNAL_ERROR, &what, id, Value::Null),
    }
}

type Method = fn(&mut dyn Host, Vec<Value>) -> CallResult;

/// メソッドの表。名前と、名前付き引数の並び。
static METHODS: &[(&str, Method, &[&str])] = &[
    ("bumpChannel", bump_channel, &["channelId"]),
    ("clearLog", clear_log, &[]),
    ("fetch", fetch, &["url", "name", "desc", "genre", "contact", "bitrate", "type", "network"]),
    ("getChannelConnections", get_channel_connections, &["channelId"]),
    ("getChannelInfo", get_channel_info, &["channelId"]),
    ("getChannelRelayTree", get_channel_relay_tree, &["channelId"]),
    ("getChannelStatus", get_channel_status, &["channelId"]),
    ("getChannels", get_channels, &[]),
    ("getLog", get_log, &["from", "maxLines"]),
    ("getLogSettings", get_log_settings, &[]),
    ("getNewVersions", get_new_versions, &[]),
    ("getNotificationMessages", get_notification_messages, &[]),
    ("getPlugins", get_plugins, &[]),
    ("getServerStorageItem", get_server_storage_item, &["key"]),
    ("getSettings", get_settings, &[]),
    ("getState", get_state, &["objectNames"]),
    ("getStatus", get_status, &[]),
    ("getVersionInfo", get_version_info, &[]),
    ("getYPChannels", get_yp_channels, &[]),
    ("getYellowPageProtocols", get_yellow_page_protocols, &[]),
    ("getYellowPages", get_yellow_pages, &[]),
    ("playChannel", play_channel, &["channelId"]),
    ("removeYellowPage", remove_yellow_page, &["yellowPageId"]),
    ("setChannelInfo", set_channel_info, &["channelId", "info", "track"]),
    ("setLogSettings", set_log_settings, &["settings"]),
    ("setServerStorageItem", set_server_storage_item, &["key", "value"]),
    ("setSettings", set_settings, &["settings"]),
    ("stopChannel", stop_channel, &["channelId"]),
    ("stopChannelConnection", stop_channel_connection, &["channelId", "connectionId"]),
];

/// 名前付き引数を、表の並びの位置引数にする。ない引数は null。
fn to_positional_arguments(named: &Object, names: &[&str]) -> Value {
    let mut result = vec![Value::Null; names.len()];
    for (k, v) in named {
        if let Some(i) = names.iter().position(|n| n.as_bytes() == k.as_slice()) {
            result[i] = v.clone();
        }
    }
    Value::Array(result)
}

/// メソッドを選んで呼ぶ。引数の数が合わなければ invalid_params、メソッドがなければ
/// method_not_found (メソッド名が文字列でなければ、その変換の type_error)。
fn dispatch(m: &Value, p: &Value, host: &mut dyn Host) -> CallResult {
    for &(name, method, names) in METHODS {
        if !m.is_str(name.as_bytes()) {
            continue;
        }
        let arguments = match p {
            Value::Array(_) => p.clone(),
            Value::Object(o) => to_positional_arguments(o, names),
            Value::Null if names.is_empty() => Value::Array(Vec::new()),
            _ => Value::Null,
        };
        if arguments.size() != names.len() {
            return Err(CallError::InvalidParams(b"Wrong number of arguments".to_vec()));
        }
        let args = match arguments {
            Value::Array(a) => a,
            other => return Err(CallError::Internal(json::type_error(302, &format!("type must be array, but is {}", other.type_name())))),
        };
        return method(host, args);
    }
    Err(CallError::MethodNotFound(te(m.as_string())?.to_vec()))
}

/// JSON-RPC では呼べず、C++ から直接呼ぶだけのメソッド
static INTERNAL_METHODS: &[(&str, Method, &[&str])] = &[("getChannelsFound", get_channels_found, &[])];

/// C++ 側から、メソッドを位置引数で直接呼ぶ (`JrpcApi::getChannels` など)。
pub fn invoke(method: &[u8], args: Vec<Value>, host: &mut dyn Host) -> CallResult {
    for &(name, f, names) in METHODS.iter().chain(INTERNAL_METHODS) {
        if name.as_bytes() == method {
            if args.len() < names.len() {
                // C++ 版は範囲外を読む。呼ぶ側 (テストなど) の誤りなので、引数の数の誤りとして返す。
                return Err(CallError::InvalidParams(b"Wrong number of arguments".to_vec()));
            }
            return f(host, args);
        }
    }
    Err(CallError::MethodNotFound(method.to_vec()))
}

// ---------------------------------------------------------------- 各メソッド

fn get_log(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let mut from = args[0].clone();
    let mut max_lines = args[1].clone();

    if !from.is_null() && te(from.as_int())? < 0 {
        return Err(CallError::InvalidParams(b"from must be non negative".to_vec()));
    }
    // C++ 版と同じく、from が null でないときだけ maxLines を確かめる
    if !from.is_null() && te(max_lines.as_int())? < 0 {
        return Err(CallError::InvalidParams(b"maxLines must be non negative".to_vec()));
    }

    let mut lines = host.log_lines()?;
    lines.push(Vec::new());

    if from.is_null() {
        from = Value::Int(0);
    }
    let skip = (te(from.as_size())?).min(lines.len() as u64) as usize;
    lines.drain(..skip);

    if max_lines.is_null() {
        max_lines = Value::UInt(lines.len() as u64);
    }
    while lines.len() as u64 > te(max_lines.as_size())? {
        lines.pop();
    }

    let log = strutil::join(b"\n", &lines);
    Ok(Value::object([
        ("from", int(te(from.as_int())?)),
        ("lines", Value::UInt(lines.len() as u64)),
        ("log", Value::Str(log)),
    ]))
}

fn clear_log(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    host.clear_log()?;
    Ok(Value::Null)
}

fn get_log_settings(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    // YT -> PeerCastStation
    // 7 OFF   -> 0 OFF
    // 6 FATAL -> 1 FATAL
    // 5 ERROR -> 2 ERROR
    // 4 WARN  -> 3 WARN
    // 3 INFO  -> 4 INFO
    // 2 DEBUG -> 5 DEBUG
    // 1 TRACE -> 5 DEBUG
    let level = host.log_level()?;
    Ok(Value::object([("level", int(5.min(7i32.wrapping_sub(level))))]))
}

fn set_log_settings(host: &mut dyn Host, mut args: Vec<Value>) -> CallResult {
    let level = te(te(args[0].index_mut(b"level"))?.as_int())?;
    if (0..=5).contains(&level) {
        host.set_log_level(7 - level)?;
    }
    Ok(Value::Null)
}

fn fetch(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let url = te(args[0].as_string())?.to_vec();
    let name = te(args[1].as_string())?;
    let desc = te(args[2].as_string())?;
    let genre = te(args[3].as_string())?;
    let contact = te(args[4].as_string())?;
    let bitrate = te(args[5].as_int())?;
    let type_str = te(args[6].as_string())?.to_vec();
    let network: &[u8] = if args[7].is_null() { b"ipv4" } else { te(args[7].as_string())? };

    let clean = |s: &[u8]| utf8::truncate(&utf8::valid(s), 255).unwrap_or_default();
    let req = FetchRequest {
        url,
        name: clean(name),
        desc: clean(desc),
        genre: clean(genre),
        contact: clean(contact),
        bitrate,
        type_str,
        ipv6: network == b"ipv6",
    };
    match host.fetch(&req) {
        Ok(Some(id)) => Ok(id_value(&id)),
        Ok(None) => Err(CallError::Application(UNKNOWN_ERROR, b"failed to create channel".to_vec())),
        Err(HostError::DomainError(w)) => Err(CallError::InvalidParams(w)),
        Err(e) => Err(e.into()),
    }
}

fn get_version_info(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::object([
        ("agentName", Value::Str(host.agent())),
        ("apiVersion", Value::str(b"1.0.0")),
        ("jsonrpc", Value::str(b"2.0")),
    ]))
}

fn info_json(info: &InfoData) -> Value {
    Value::object([
        ("name", vu(&info.name)),
        ("url", vu(&info.url)),
        ("genre", vu(&info.genre)),
        ("desc", vu(&info.desc)),
        ("comment", vu(&info.comment)),
        ("bitrate", int(info.bitrate)),
        ("contentType", Value::str(&info.content_type)),
        ("mimeType", Value::str(chaninfo::effective_mime(&info.content_type, &info.mime))),
    ])
}

fn track_json(info: &InfoData) -> Value {
    Value::object([
        ("name", vu(&info.track_title)),
        ("genre", vu(&info.track_genre)),
        ("album", vu(&info.track_album)),
        ("creator", vu(&info.track_artist)),
        ("url", vu(&info.track_contact)),
    ])
}

fn ip_version_json(v: i32) -> Value {
    match v {
        4 => Value::str(b"ipv4"),
        6 => Value::str(b"ipv6"),
        _ => Value::str(b"unknown"),
    }
}

/// `Channel::statusMsgs`
const STATUS_MSGS: [&[u8]; 13] = [
    b"NONE", b"WAIT", b"CONNECT", b"REQUEST", b"CLOSE", b"RECEIVE", b"BROADCAST", b"ABORT", b"SEARCH", b"NOHOSTS",
    b"IDLE", b"ERROR", b"NOTFOUND",
];

// PeerCastStation では Receiving, Searching, Error, Idle のどれか
// が返る。peercast では NONE, WAIT, CONNECT, REQUEST, CLOSE,
// RECEIVE, BROADCAST, ABORT, SEARCH, NOHOSTS, IDLE, ERROR,
// NOTFOUND のどれか。
fn status_json(status: i32) -> Value {
    match status {
        5 | 6 => Value::str(b"Receiving"), // S_RECEIVING, S_BROADCASTING
        8 | 2 => Value::str(b"Searching"), // S_SEARCHING, S_CONNECTING
        11 => Value::str(b"Error"),
        10 => Value::str(b"Idle"),
        s => Value::str(STATUS_MSGS.get(s as usize).copied().unwrap_or(b"")),
    }
}

fn source_uri(c: &ChannelData) -> Vec<u8> {
    if !c.source_url.is_empty() {
        return c.source_url.clone();
    }
    let s = strutil::downcase(&gnuid::to_str(&c.info.id));
    let mut out = b"pcp://".to_vec();
    out.extend_from_slice(&c.source_host);
    out.push(b'/');
    for (i, part) in [&s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32]].iter().enumerate() {
        if i > 0 {
            out.push(b'-');
        }
        out.extend_from_slice(part);
    }
    out
}

fn channel_status(c: &ChannelData) -> Value {
    Value::object([
        ("status", status_json(c.status)),
        ("source", Value::Str(source_uri(c))),
        ("uptime", uint(c.uptime)),
        ("localRelays", int(c.local_relays)),
        ("localDirects", int(c.local_directs)),
        ("totalRelays", int(c.total_relays)),
        ("totalDirects", int(c.total_directs)),
        ("isBroadcasting", Value::Bool(c.is_broadcasting)),
        ("isRelayFull", Value::Bool(c.is_full)),
        ("isDirectFull", Value::Null),
        ("isReceiving", Value::Bool(c.is_receiving)),
        ("network", ip_version_json(c.ip_version)),
    ])
}

fn channel_json(c: &ChannelData) -> Value {
    Value::object([
        ("channelId", id_value(&c.info.id)),
        ("status", channel_status(c)),
        ("info", info_json(&c.info)),
        ("track", track_json(&c.info)),
        ("yellowPages", Value::Array(Vec::new())),
    ])
}

fn opt_str(s: &Option<Vec<u8>>) -> Value {
    match s {
        Some(s) => Value::str(s),
        None => Value::Null,
    }
}

fn channel_id_arg(v: &Value) -> Result<[u8; 16], CallError> {
    Ok(gnuid_of(te(v.as_string())?))
}

fn not_found() -> CallError {
    CallError::Application(CHANNEL_NOT_FOUND, b"Channel not found".to_vec())
}

// チャンネルに関して特定の接続を停止する。成功すれば true、失敗す
// れば false を返す。
fn stop_channel_connection(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    let connection_id = te(args[1].as_int())?;
    Ok(Value::Bool(host.stop_connection(&id, connection_id)?))
}

fn connection_json(s: &ServentData) -> Value {
    let remote = opt_str(&s.sock_host);
    Value::object([
        ("connectionId", int(s.index)),
        ("type", Value::Str(strutil::downcase(&s.type_str))),
        ("status", Value::str(&s.status_str)),
        ("sendRate", uint(s.send_rate)),
        ("recvRate", uint(s.recv_rate)),
        ("protocolName", Value::str(chaninfo::protocol_str(s.protocol))),
        ("localRelays", Value::Null),
        ("localDirects", Value::Null),
        ("contentPosition", Value::Null),
        ("agentName", Value::str(c_str(&s.agent))),
        ("remoteEndPoint", remote.clone()),
        ("remoteHostStatus", Value::Array(Vec::new())), // 何を入れたらいいのかわからない。
        ("remoteName", remote),
    ])
}

fn source_connection_json(c: &ChannelData) -> Value {
    let remote_name = if c.source_url.is_empty() { &c.source_host } else { &c.source_url };
    Value::object([
        ("connectionId", int(-1)),
        ("type", Value::str(b"source")),
        ("status", status_json(c.status)),
        ("sendRate", Value::Float(0.0)),
        ("recvRate", int(c.source_rate)),
        ("protocolName", Value::str(chaninfo::protocol_str(c.src_protocol))),
        ("localRelays", Value::Null),
        ("localDirects", Value::Null),
        ("contentPosition", uint(c.stream_pos)),
        ("agentName", Value::Null),
        ("remoteEndPoint", opt_str(&c.sock_host)),
        ("remoteHostStatus", Value::Array(Vec::new())), // 何を入れたらいいのかわからない。
        ("remoteName", Value::str(c_str(remote_name))),
    ])
}

fn get_channel_connections(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    let c = host.find_channel(&id)?.ok_or_else(not_found)?;
    let mut result = vec![source_connection_json(&c)];
    for s in host.servents(&id)? {
        result.push(connection_json(&s));
    }
    Ok(Value::Array(result))
}

fn get_channel_info(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    let c = host.find_channel(&id)?.ok_or_else(not_found)?;
    Ok(Value::object([
        ("info", info_json(&c.info)),
        ("track", track_json(&c.info)),
        ("yellowPages", Value::Array(Vec::new())),
    ]))
}

fn get_channel_status(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    let c = host.find_channel(&id)?.ok_or_else(not_found)?;
    Ok(channel_status(&c))
}

fn get_channels(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(host.channels()?.iter().map(channel_json).collect()))
}

fn get_yellow_pages(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    let root = pc_string(&host.root_host()?);
    if root.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    // 配信中のチャンネルとルートサーバーとの接続状態は、いつも "Connected"
    let channels = host
        .channels()?
        .iter()
        .filter(|c| c.is_broadcasting)
        .map(|c| Value::object([("channelId", id_value(&c.info.id)), ("status", Value::str(b"Connected"))]))
        .collect();
    // String::format は MAX_LEN - 1 を vsnprintf に渡すので、254 バイトで切れる (文字の途中でも)
    let mut uri = [&b"pcp://"[..], &root, b"/"].concat();
    uri.truncate(254);
    Ok(Value::Array(vec![Value::object([
        ("yellowPageId", int(0)),
        ("name", Value::Str(root.clone())),
        ("uri", Value::Str(uri.clone())),
        ("announceUri", Value::Str(uri)),
        ("channelsUri", Value::Null),
        ("protocol", Value::str(b"pcp")),
        ("channels", Value::Array(channels)),
    ])]))
}

fn get_yellow_page_protocols(_host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(vec![Value::object([("name", Value::str(b"PCP")), ("protocol", Value::str(b"pcp"))])]))
}

fn set_settings(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let mut settings = te(args[0].as_object())?.clone();
    // std::map の operator[] はないキーを null で作るので、そのときは型の変換で失敗する。
    // 途中で失敗したら、それまでの設定は変わったまま。
    for (key, which) in [
        (&b"maxRelays"[..], SettingKey::MaxRelays),
        (b"maxRelaysPerChannel", SettingKey::MaxRelaysPerChannel),
        (b"maxDirects", SettingKey::MaxDirect),
        // maxDirectsPerChannel は無視。
        (b"maxUpstreamRate", SettingKey::MaxBitrateOut),
        // maxUpstreamRatePerChannel は無視。
        // channelCleaner, portMapper は無視。
    ] {
        let v = te(settings.entry(key.to_vec()).or_insert(Value::Null).as_int())?;
        host.set_setting(which, v)?;
    }
    Ok(Value::Null)
}

fn get_settings(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    let s = host.settings()?;
    Ok(Value::object([
        ("maxRelays", uint(s.max_relays)),
        ("maxRelaysPerChannel", int(s.max_relays_per_channel)),
        ("maxDirects", uint(s.max_direct)),
        ("maxDirectsPerChannel", int(0)),
        ("maxUpstreamRate", uint(s.max_bitrate_out)),
        ("maxUpstreamRatePerChannel", int(0)),
        // channelCleaner は無視。
    ]))
}

fn get_plugins(_host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(Vec::new()))
}

fn firewall_json(host: &mut dyn Host, state: i32) -> Value {
    match state {
        0 => Value::Bool(false), // FW_OFF
        1 => Value::Bool(true),  // FW_ON
        2 => Value::Null,        // FW_UNKNOWN
        _ => {
            host.log(Level::Error, b"Invalid firewall state");
            Value::Null
        }
    }
}

fn get_status(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    let s = host.status()?;
    let global = || Value::Array(vec![Value::str(&s.global_ip), Value::UInt(s.port as u64)]);
    let local = || Value::Array(vec![Value::str(&s.local_ip), Value::UInt(s.port as u64)]);
    let fw = firewall_json(host, s.firewall);
    Ok(Value::object([
        ("uptime", uint(s.uptime)),
        ("isFirewalled", fw),
        ("globalRelayEndPoint", global()),
        ("globalDirectEndPoint", global()),
        ("localRelayEndPoint", local()),
        ("localDirectEndPoint", local()),
    ]))
}

fn get_state(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let names = te(args[0].as_strings())?;
    let mut result = Object::new();
    for name in names {
        let which = match STATE_NAMES.iter().position(|n| *n == name.as_slice()) {
            Some(w) => w,
            None => {
                let mut msg = b"Unknown object name: ".to_vec();
                msg.extend_from_slice(&name);
                return Err(CallError::InvalidParams(msg));
            }
        };
        let text = host.state(which)?;
        let v = json::parse(&text).map_err(|e| CallError::Internal(e.what().to_vec()))?;
        result.insert(name, v);
    }
    Ok(Value::Object(result))
}

fn get_new_versions(_host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(Vec::new()))
}

fn get_notification_messages(_host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(Vec::new()))
}

/// `mergeChanInfo` の `info.at(key).get<std::string>()`
fn at_string<'a>(o: &'a Object, key: &[u8]) -> Result<&'a [u8], CallError> {
    match o.get(key) {
        Some(v) => te(v.as_string()),
        // std::map::at の std::out_of_range
        None => Err(CallError::Internal(b"map::at".to_vec())),
    }
}

fn set_channel_info(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let channel_id = te(args[0].as_string())?;
    let id = gnuid_of(channel_id);
    let channel = host.find_channel(&id)?.ok_or_else(not_found)?;

    // mergeChanInfo(channel->info, info, track) の引数は、x86-64 の GCC では右から変換される
    let track = te(args[2].as_object())?;
    let info = te(args[1].as_object())?;

    let mut fields: [Vec<u8>; 10] = Default::default();
    for (i, key) in [&b"name"[..], b"desc", b"genre", b"url", b"comment"].iter().enumerate() {
        fields[i] = at_string(info, key)?.to_vec();
    }
    for (i, key) in [&b"url"[..], b"name", b"creator", b"album", b"genre"].iter().enumerate() {
        fields[5 + i] = at_string(track, key)?.to_vec();
    }

    // 合わせた情報を LOG_DEBUG に出す (書き出せなければ例外)
    let mut merged = channel.info.clone();
    merged.name = pc_string(&fields[0]);
    merged.desc = pc_string(&fields[1]);
    merged.genre = pc_string(&fields[2]);
    merged.url = pc_string(&fields[3]);
    merged.comment = pc_string(&fields[4]);
    let mut msg = b"i = ".to_vec();
    msg.extend_from_slice(&te(json::dump(&info_json(&merged)))?);
    host.log(Level::Debug, &msg);

    host.update_info(&id, &fields)?;
    Ok(Value::Null)
}

fn stop_channel(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = gnuid_of(te(args[0].as_string())?);
    host.stop_channel(&id)?;
    Ok(Value::Null)
}

fn relay_tree_json(host: &mut dyn Host, hits: &[(Hit, Vec<u8>)]) -> Value {
    let nodes: Vec<hostgraph::Node> =
        hits.iter().map(|(h, _)| hostgraph::Node { rhost: h.rhost, uphost: h.uphost }).collect();
    let entries = hostgraph::build(&nodes);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut roots = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        match e.parent {
            Some(p) => children[p].push(k),
            None => roots.push(k),
        }
    }

    fn to_tree(
        host: &mut dyn Host,
        k: usize,
        path: &[usize],
        hits: &[(Hit, Vec<u8>)],
        entries: &[hostgraph::Entry],
        children: &[Vec<usize>],
    ) -> Value {
        let (hit, addr) = &hits[entries[k].index];
        let mut kids = Vec::new();
        for &child in &children[k] {
            if !path.contains(&child) {
                let mut p = path.to_vec();
                p.push(k);
                kids.push(to_tree(host, child, &p, hits, entries, children));
            } else {
                host.log(Level::Warn, b"toRelayTree: circularity detected.");
            }
        }
        Value::object([
            ("sessionId", id_value(&hit.session_id)),
            ("address", Value::str(addr)),
            ("port", Value::UInt(hit.rhost[0].port as u64)), // ペカステに合わせて 0 を入れないようにすべき？
            ("isFirewalled", Value::Bool(hit.firewalled)),
            ("localRelays", uint(hit.num_relays)),
            ("localDirects", uint(hit.num_listeners)),
            ("isTracker", Value::Bool(hit.tracker)),
            ("isRelayFull", Value::Bool(!hit.relay)),
            ("isDirectFull", Value::Bool(!hit.direct)),
            ("isReceiving", Value::Bool(hit.recv)),
            ("isControlFull", Value::Bool(!hit.cin)),
            ("version", uint(hit.version)),
            ("versionString", Value::Str(chanhit::version_string(hit))),
            ("children", Value::Array(kids)),
        ])
    }

    Value::Array(roots.iter().map(|&r| to_tree(host, r, &[r], hits, &entries, &children)).collect())
}

fn get_channel_relay_tree(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    match host.relay_tree(&id)? {
        RelayTreeData::NoChannel => Err(not_found()),
        RelayTreeData::NoHitList => Err(CallError::Application(UNKNOWN_ERROR, b"Hit list not found".to_vec())),
        RelayTreeData::Hits(hits) => Ok(relay_tree_json(host, &hits)),
    }
}

fn bump_channel(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = channel_id_arg(&args[0])?;
    if id.iter().all(|&b| b == 0) {
        return Err(CallError::InvalidParams(b"id".to_vec()));
    }
    if !host.bump(&id)? {
        return Err(not_found());
    }
    Ok(Value::Null)
}

fn play_channel(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let id = gnuid_of(te(args[0].as_string())?);
    host.play(&id)?;
    Ok(Value::Null)
}

fn remove_yellow_page(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    if te(args[0].as_int())? == 0 {
        host.clear_root_host()?;
        Ok(Value::Null)
    } else {
        Err(CallError::Application(UNKNOWN_ERROR, b"Unknown yellow page id".to_vec()))
    }
}

fn get_yp_channels(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    let res = host
        .yp_channels()?
        .iter()
        .map(|c| {
            Value::object([
                ("yellowPage", Value::str(&c.feed_url)),
                ("name", Value::str(&c.name)),
                ("channelId", id_value(&c.id)),
                ("tracker", Value::str(&c.tip)),
                ("contactUrl", Value::str(&c.url)),
                ("genre", Value::str(&c.genre)),
                ("description", Value::str(&c.desc)),
                ("comment", Value::str(&c.comment)),
                ("bitrate", int(c.bitrate)),
                ("contentType", Value::str(&c.content_type)),
                ("trackTitle", Value::str(&c.track_name)),
                ("album", Value::str(&c.track_album)),
                ("creator", Value::str(&c.track_artist)),
                ("trackUrl", Value::str(&c.track_contact)),
                ("listeners", int(c.num_directs)),
                ("relays", int(c.num_relays)),
            ])
        })
        .collect();
    Ok(Value::Array(res))
}

fn found_json(f: &FoundData) -> Value {
    let hits = f
        .hits
        .iter()
        .map(|h| {
            Value::object([
                ("ip", Value::str(&h.ip)),
                ("hops", uint(h.hops)),
                ("listeners", uint(h.listeners)),
                ("relays", uint(h.relays)),
                ("uptime", uint(h.uptime)),
                ("push", Value::Bool(h.push)),
                ("relay", Value::Bool(h.relay)),
                ("direct", Value::Bool(h.direct)),
                ("cin", Value::Bool(h.cin)), // これrootモードで動いてるかってこと？
                ("stable", Value::Bool(h.stable)),
                ("version", uint(h.version)),
                ("update", uint(h.update)),
                ("tracker", Value::Bool(h.tracker)),
            ])
        })
        .collect();
    let info = &f.info;
    Value::object([
        ("name", vu(&info.name)),
        ("id", id_value(&info.id)),
        ("bitrate", int(info.bitrate)),
        ("type", Value::str(&info.content_type)),
        ("genre", vu(&info.genre)),
        ("desc", vu(&info.desc)),
        ("url", Value::str(&info.url)),
        ("uptime", uint(f.uptime)),
        ("comment", vu(&info.comment)),
        ("skips", uint(f.skips)),
        ("age", uint(f.age)),
        ("bcflags", uint(f.bcflags as u32)),
        (
            "hit_stat",
            Value::object([
                ("hosts", int(f.hosts)),
                ("listeners", int(f.listeners)),
                ("relays", int(f.relays)),
                ("firewalled", int(f.firewalled)),
                ("closest", int(f.closest)),
                ("furthest", int(f.furthest)),
                ("newest", uint(f.newest)),
            ]),
        ),
        ("hits", Value::Array(hits)),
        ("track", track_json(info)),
    ])
}

/// ヒットリストの一覧 (`public.cpp` が使う)
fn get_channels_found(host: &mut dyn Host, _args: Vec<Value>) -> CallResult {
    Ok(Value::Array(host.channels_found()?.iter().map(found_json).collect()))
}

const KNOWN_KEYS: [&[u8]; 1] = [b"channelFilters"];

fn get_server_storage_item(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let key = te(args[0].as_string())?;
    if !KNOWN_KEYS.contains(&key) {
        return Err(CallError::Application(UNKNOWN_ERROR, b"invalid key".to_vec()));
    }
    Ok(match host.read_storage(key)? {
        Some(s) => Value::Str(s),
        None => Value::Null,
    })
}

fn set_server_storage_item(host: &mut dyn Host, args: Vec<Value>) -> CallResult {
    let key = te(args[0].as_string())?;
    let value = te(args[1].as_string())?;
    if !KNOWN_KEYS.contains(&key) {
        return Err(CallError::Application(UNKNOWN_ERROR, b"invalid key".to_vec()));
    }
    host.write_storage(key, value)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests;
