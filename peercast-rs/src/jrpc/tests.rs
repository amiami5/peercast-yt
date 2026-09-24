use super::*;

/// 状態をメモリーに持つだけの Host
#[derive(Default)]
struct Mock {
    logs: Vec<(Level, Vec<u8>)>,
    lines: Vec<Vec<u8>>,
    level: i32,
    channels: Vec<ChannelData>,
    servents: Vec<([u8; 16], ServentData)>,
    root: Vec<u8>,
    settings: Settings,
    calls: Vec<String>,
    state: Vec<u8>,
    storage: Option<Vec<u8>>,
    fail: Option<HostError>,
    hits: Option<Vec<(Hit, Vec<u8>)>>,
}

impl Mock {
    fn fail<T>(&mut self) -> HostResult<()> {
        match self.fail.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Host for Mock {
    fn log(&mut self, level: Level, msg: &[u8]) {
        self.logs.push((level, msg.to_vec()));
    }
    fn agent(&mut self) -> Vec<u8> {
        b"PeerCast/0.1218 (YT33)".to_vec()
    }
    fn log_lines(&mut self) -> HostResult<Vec<Vec<u8>>> {
        Ok(self.lines.clone())
    }
    fn clear_log(&mut self) -> HostResult<()> {
        self.lines.clear();
        Ok(())
    }
    fn log_level(&mut self) -> HostResult<i32> {
        Ok(self.level)
    }
    fn set_log_level(&mut self, level: i32) -> HostResult<()> {
        self.level = level;
        Ok(())
    }
    fn fetch(&mut self, req: &FetchRequest) -> HostResult<Option<[u8; 16]>> {
        self.fail::<()>()?;
        self.calls.push(format!("fetch {} {}", String::from_utf8_lossy(&req.name), req.ipv6));
        Ok(if req.url.is_empty() { None } else { Some([0xab; 16]) })
    }
    fn channels(&mut self) -> HostResult<Vec<ChannelData>> {
        Ok(self.channels.clone())
    }
    fn find_channel(&mut self, id: &[u8; 16]) -> HostResult<Option<ChannelData>> {
        Ok(self.channels.iter().find(|c| &c.info.id == id).cloned())
    }
    fn servents(&mut self, id: &[u8; 16]) -> HostResult<Vec<ServentData>> {
        Ok(self.servents.iter().filter(|(i, _)| i == id).map(|(_, s)| s.clone()).collect())
    }
    fn stop_connection(&mut self, id: &[u8; 16], connection_id: i32) -> HostResult<bool> {
        Ok(self.servents.iter().any(|(i, s)| i == id && s.index == connection_id))
    }
    fn relay_tree(&mut self, id: &[u8; 16]) -> HostResult<RelayTreeData> {
        if !self.channels.iter().any(|c| &c.info.id == id) {
            return Ok(RelayTreeData::NoChannel);
        }
        Ok(match &self.hits {
            Some(h) => RelayTreeData::Hits(h.clone()),
            None => RelayTreeData::NoHitList,
        })
    }
    fn bump(&mut self, id: &[u8; 16]) -> HostResult<bool> {
        self.calls.push("bump".into());
        Ok(self.channels.iter().any(|c| &c.info.id == id))
    }
    fn play(&mut self, _id: &[u8; 16]) -> HostResult<()> {
        self.calls.push("play".into());
        Ok(())
    }
    fn stop_channel(&mut self, _id: &[u8; 16]) -> HostResult<()> {
        self.calls.push("stop".into());
        Ok(())
    }
    fn root_host(&mut self) -> HostResult<Vec<u8>> {
        Ok(self.root.clone())
    }
    fn clear_root_host(&mut self) -> HostResult<()> {
        self.root.clear();
        Ok(())
    }
    fn settings(&mut self) -> HostResult<Settings> {
        Ok(self.settings)
    }
    fn set_setting(&mut self, key: SettingKey, value: i32) -> HostResult<()> {
        match key {
            SettingKey::MaxRelays => self.settings.max_relays = value as u32,
            SettingKey::MaxRelaysPerChannel => self.settings.max_relays_per_channel = value,
            SettingKey::MaxDirect => self.settings.max_direct = value as u32,
            SettingKey::MaxBitrateOut => self.settings.max_bitrate_out = value as u32,
        }
        Ok(())
    }
    fn status(&mut self) -> HostResult<Status> {
        Ok(Status { uptime: 5, firewall: 2, global_ip: b"127.0.0.1".to_vec(), port: 7144, local_ip: b"192.168.0.2".to_vec() })
    }
    fn state(&mut self, _which: usize) -> HostResult<Vec<u8>> {
        Ok(self.state.clone())
    }
    fn update_info(&mut self, _id: &[u8; 16], fields: &[Vec<u8>; 10]) -> HostResult<()> {
        self.calls.push(format!("update {}", String::from_utf8_lossy(&fields[0])));
        Ok(())
    }
    fn yp_channels(&mut self) -> HostResult<Vec<YpEntry>> {
        Ok(vec![YpEntry { name: b"yp\xff".to_vec(), ..Default::default() }])
    }
    fn read_storage(&mut self, _key: &[u8]) -> HostResult<Option<Vec<u8>>> {
        Ok(self.storage.clone())
    }
    fn write_storage(&mut self, _key: &[u8], value: &[u8]) -> HostResult<()> {
        self.fail::<()>()?;
        self.storage = Some(value.to_vec());
        Ok(())
    }
    fn channels_found(&mut self) -> HostResult<Vec<FoundData>> {
        Ok(vec![FoundData { hits: vec![FoundHit::default()], ..Default::default() }])
    }
}

fn run(m: &mut Mock, req: &str) -> String {
    String::from_utf8(call(req.as_bytes(), m).unwrap()).unwrap()
}

fn rpc(m: &mut Mock, method: &str, params: &str) -> String {
    run(m, &format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{}","params":{}}}"#, method, params))
}

const CH: [u8; 16] = [0x11; 16];
const CHS: &str = "11111111111111111111111111111111";

fn channel() -> ChannelData {
    ChannelData {
        info: InfoData { id: CH, name: b"ch\xe3\x81".to_vec(), content_type: b"FLV".to_vec(), bitrate: 500, ..Default::default() },
        status: 6,
        source_host: b"1.2.3.4:7144".to_vec(),
        is_broadcasting: true,
        ip_version: 4,
        ..Default::default()
    }
}

#[test]
fn envelope() {
    let mut m = Mock::default();
    assert_eq!(run(&mut m, "hello world"), r#"{"error":{"code":-32700,"message":"Parse error"},"id":null,"jsonrpc":"2.0"}"#);
    // C++ 版は 1e400 の例外が上に飛んでいた
    assert_eq!(run(&mut m, "1e400"), r#"{"error":{"code":-32700,"message":"Parse error"},"id":null,"jsonrpc":"2.0"}"#);
    assert_eq!(run(&mut m, r#""hoge""#), r#"{"error":{"code":-32600,"message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#);
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","id":1234}"#),
        r#"{"error":{"code":-32600,"message":"Invalid Request"},"id":1234,"jsonrpc":"2.0"}"#
    );
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","method":"getVersionInfo"}"#),
        r#"{"error":{"code":-32600,"message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#
    );
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","method":"x","id":[1.5],"params":3}"#),
        r#"{"error":{"code":-32602,"data":"params must be either object or array","message":"Invalid params"},"id":[1.5],"jsonrpc":"2.0"}"#
    );
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","method":"nonexistentMethod","id":"a"}"#),
        r#"{"error":{"code":-32601,"data":"nonexistentMethod","message":"Method not found"},"id":"a","jsonrpc":"2.0"}"#
    );
    // メソッド名が文字列でなければ、その変換の例外
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","method":5,"id":1}"#),
        r#"{"error":{"code":-32603,"message":"[json.exception.type_error.302] type must be string, but is number"},"id":1,"jsonrpc":"2.0"}"#
    );
    assert_eq!(
        run(&mut m, r#"{"jsonrpc":"2.0","method":"stopChannel","id":1}"#),
        r#"{"error":{"code":-32602,"data":"Wrong number of arguments","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#
    );
    // what() は NUL の手前まで
    let r = run(&mut m, r#"{"jsonrpc":"2.0","method":"a\u0000b","id":1}"#);
    assert_eq!(r, r#"{"error":{"code":-32601,"data":"a","message":"Method not found"},"id":1,"jsonrpc":"2.0"}"#);
    // 応答のログは先頭 60 バイトまで
    assert_eq!(m.logs.last().unwrap().1, [&b"jrpc response: "[..], &r.as_bytes()[..60]].concat());
}

#[test]
fn nesting() {
    let mut m = Mock::default();
    let deep = format!(r#"{{"jsonrpc":"2.0","method":"getStatus","id":1,"params":{}{}}}"#, "[".repeat(200000), "]".repeat(200000));
    assert!(run(&mut m, &deep).contains("-32700"));
    assert!(run(&mut m, &"[".repeat(200000)).contains("-32700"));
    let s = format!(
        r#"{{"jsonrpc":"2.0","method":"getServerStorageItem","id":2,"params":["{}\"{}"]}}"#,
        "[".repeat(500),
        "[".repeat(500)
    );
    assert!(run(&mut m, &s).contains("invalid key"));
}

#[test]
fn methods_table() {
    assert_eq!(METHODS.len(), 29);
    let mut o = Object::new();
    o.insert(b"b".to_vec(), Value::UInt(2));
    o.insert(b"a".to_vec(), Value::UInt(1));
    o.insert(b"c".to_vec(), Value::UInt(3));
    assert_eq!(to_positional_arguments(&o, &["a", "b"]), Value::Array(vec![Value::UInt(1), Value::UInt(2)]));
    assert_eq!(to_positional_arguments(&Object::new(), &["a"]), Value::Array(vec![Value::Null]));
}

#[test]
fn get_log_quirks() {
    let mut m = Mock { lines: vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()], ..Default::default() };
    assert_eq!(rpc(&mut m, "getLog", "[null,null]"), r#"{"id":1,"jsonrpc":"2.0","result":{"from":0,"lines":4,"log":"a\nb\nc\n"}}"#);
    assert_eq!(rpc(&mut m, "getLog", "[1,2]"), r#"{"id":1,"jsonrpc":"2.0","result":{"from":1,"lines":2,"log":"b\nc"}}"#);
    assert!(rpc(&mut m, "getLog", "[-1,2]").contains("from must be non negative"));
    // maxLines を確かめるのは from が null でないときだけ
    assert!(rpc(&mut m, "getLog", "[0,-1]").contains("maxLines must be non negative"));
    assert!(rpc(&mut m, "getLog", "[null,-1]").contains(r#""lines":4"#));
    assert!(rpc(&mut m, "getLog", "[1,null]").contains("type must be number, but is null"));
    // get<size_t> は真偽値を受け付けない
    assert!(rpc(&mut m, "getLog", "[true,1]").contains("type must be number, but is boolean"));
    assert!(rpc(&mut m, "getLog", "{}").contains(r#""from":0"#));
    // 範囲外の浮動小数点数は x86-64 と同じ
    assert!(rpc(&mut m, "getLog", "[1e10,1]").contains("from must be non negative"));
    assert!(rpc(&mut m, "getLog", "[null,1e20]").contains(r#""lines":0"#));
    // 書き出せない文字列は Internal error
    m.lines = vec![b"\xff".to_vec()];
    assert!(rpc(&mut m, "getLog", "[null,null]").contains("invalid UTF-8 byte at index 0: 0xFF"));
}

#[test]
fn settings_and_log_level() {
    let mut m = Mock { level: 3, ..Default::default() };
    assert!(rpc(&mut m, "getLogSettings", "[]").contains(r#""result":{"level":4}"#));
    rpc(&mut m, "setLogSettings", r#"[{"level":1}]"#);
    assert_eq!(m.level, 6);
    rpc(&mut m, "setLogSettings", r#"[{"level":6}]"#);
    assert_eq!(m.level, 6);
    assert!(rpc(&mut m, "setLogSettings", r#"[{}]"#).contains("type must be number, but is null"));
    assert!(rpc(&mut m, "setLogSettings", r#"[5]"#).contains("cannot use operator[] with a string argument with number"));
    // null はオブジェクトになり、level が null になる
    assert!(rpc(&mut m, "setLogSettings", r#"[null]"#).contains("type must be number, but is null"));

    // 途中で失敗しても、それまでの設定は変わる
    let r = rpc(&mut m, "setSettings", r#"[{"maxRelays":3,"maxRelaysPerChannel":-1,"maxDirects":"x"}]"#);
    assert!(r.contains("type must be number, but is string"));
    assert_eq!(m.settings.max_relays, 3);
    assert_eq!(m.settings.max_relays_per_channel, -1);
    assert!(rpc(&mut m, "getSettings", "[]").contains(r#""maxRelays":3,"maxRelaysPerChannel":-1"#));
    rpc(&mut m, "setSettings", r#"[{"maxRelays":1,"maxRelaysPerChannel":1,"maxDirects":-1,"maxUpstreamRate":true}]"#);
    assert!(rpc(&mut m, "getSettings", "[]").contains(r#""maxDirects":4294967295,"#));
    assert_eq!(m.settings.max_bitrate_out, 1);
}

#[test]
fn channels() {
    let mut m = Mock { channels: vec![channel()], ..Default::default() };
    let r = rpc(&mut m, "getChannels", "[]");
    assert!(r.contains(r#""channelId":"11111111111111111111111111111111""#), "{}", r);
    assert!(r.contains(r#""name":"ch[E3][81]""#), "{}", r);
    assert!(r.contains(r#""mimeType":"video/x-flv""#), "{}", r);
    assert!(r.contains(r#""source":"pcp://1.2.3.4:7144/11111111-1111-1111-1111-111111111111""#), "{}", r);
    assert!(r.contains(r#""status":"Receiving""#), "{}", r);

    let r = rpc(&mut m, "getChannelStatus", &format!(r#"["{}"]"#, CHS));
    assert!(r.contains(r#""isBroadcasting":true"#), "{}", r);
    assert!(rpc(&mut m, "getChannelInfo", r#"["hoge"]"#).contains(r#""code":-1"#));

    m.servents.push((CH, ServentData { index: 7, type_str: b"RELAY".to_vec(), agent: b"a\0b".to_vec(), ..Default::default() }));
    let r = rpc(&mut m, "getChannelConnections", &format!(r#"["{}"]"#, CHS));
    assert!(r.contains(r#""sendRate":0.0"#), "{}", r);
    assert!(r.contains(r#""agentName":"a","connectionId":7"#), "{}", r);
    assert!(r.contains(r#""type":"relay""#), "{}", r);
    assert!(rpc(&mut m, "stopChannelConnection", &format!(r#"["{}",7]"#, CHS)).contains(r#""result":true"#));

    // bumpChannel
    assert!(rpc(&mut m, "bumpChannel", r#"[null]"#).contains("-32603"));
    assert!(rpc(&mut m, "bumpChannel", r#"[""]"#).contains("-32602"));
    assert!(rpc(&mut m, "bumpChannel", r#"["00000000000000000000000000000000"]"#).contains("-32602"));
    assert!(rpc(&mut m, "bumpChannel", r#"["22222222222222222222222222222222"]"#).contains(r#""code":-1"#));
    assert!(rpc(&mut m, "bumpChannel", &format!(r#"["{}"]"#, CHS)).contains(r#""result":null"#));

    // yellow pages
    assert!(rpc(&mut m, "getYellowPages", "[]").contains(r#""result":[]"#));
    m.root = b"hogehoge".to_vec();
    let r = rpc(&mut m, "getYellowPages", "[]");
    assert!(r.contains(r#""announceUri":"pcp://hogehoge/","channels":[{"channelId":"11111111111111111111111111111111","status":"Connected"}]"#), "{}", r);
    assert!(rpc(&mut m, "removeYellowPage", "[1234]").contains("Unknown yellow page id"));
    rpc(&mut m, "removeYellowPage", "[0]");
    assert!(m.root.is_empty());
}

#[test]
fn set_channel_info() {
    let mut m = Mock { channels: vec![channel()], ..Default::default() };
    let info = r#"{"name":"n","desc":"d","genre":"g","url":"u","comment":"c"}"#;
    let track = r#"{"url":"u","name":"n","creator":"c","album":"a","genre":"g"}"#;
    let r = rpc(&mut m, "setChannelInfo", &format!(r#"["{}",{},{}]"#, CHS, info, track));
    assert!(r.contains(r#""result":null"#), "{}", r);
    assert_eq!(m.calls, vec!["update n"]);
    assert!(m.logs.iter().any(|(_, l)| l.starts_with(br#"i = {"bitrate":500,"comment":"c","contentType":"FLV""#)));

    // 引数は右 (track) から変換される
    let r = rpc(&mut m, "setChannelInfo", &format!(r#"["{}",1,"x"]"#, CHS));
    assert!(r.contains("type must be object, but is string"), "{}", r);
    let r = rpc(&mut m, "setChannelInfo", &format!(r#"["{}",{{}},{}]"#, CHS, track));
    assert!(r.contains(r#""message":"map::at""#), "{}", r);
    assert!(rpc(&mut m, "setChannelInfo", r#"["x",1,2]"#).contains("Channel not found"));
}

#[test]
fn misc_methods() {
    let mut m = Mock::default();
    assert_eq!(
        rpc(&mut m, "getVersionInfo", "[]"),
        r#"{"id":1,"jsonrpc":"2.0","result":{"agentName":"PeerCast/0.1218 (YT33)","apiVersion":"1.0.0","jsonrpc":"2.0"}}"#
    );
    assert!(rpc(&mut m, "getStatus", "[]").contains(
        r#""result":{"globalDirectEndPoint":["127.0.0.1",7144],"globalRelayEndPoint":["127.0.0.1",7144],"isFirewalled":null,"#
    ));
    assert!(rpc(&mut m, "getYellowPageProtocols", "[]").contains(r#"[{"name":"PCP","protocol":"pcp"}]"#));
    assert!(rpc(&mut m, "getYPChannels", "[]").contains("invalid UTF-8 byte at index 2: 0xFF"));

    assert!(rpc(&mut m, "fetch", r#"["http://x/","n","","","",1,"FLV",null]"#).contains(r#""result":"ABABABAB"#));
    assert!(rpc(&mut m, "fetch", r#"["","n","","","",1,"FLV","ipv6"]"#).contains("failed to create channel"));
    assert_eq!(m.calls, vec!["fetch n false", "fetch n true"]);
    m.fail = Some(HostError::DomainError(b"dom".to_vec()));
    assert!(rpc(&mut m, "fetch", r#"["u","n","","","",1,"FLV",null]"#).contains(r#""data":"dom""#));

    // getState
    m.state = br#"{"a":1}"#.to_vec();
    assert!(rpc(&mut m, "getState", r#"[["servMgr","sys"]]"#).contains(r#""result":{"servMgr":{"a":1},"sys":{"a":1}}"#));
    assert!(rpc(&mut m, "getState", r#"[["x"]]"#).contains(r#""data":"Unknown object name: x""#));
    assert!(rpc(&mut m, "getState", r#"[[1]]"#).contains("type must be string, but is number"));
    m.state = b"{\"a\":nan}".to_vec();
    assert!(rpc(&mut m, "getState", r#"[["stats"]]"#).contains("[json.exception.parse_error.101]"));

    // server storage
    assert!(rpc(&mut m, "getServerStorageItem", r#"["x"]"#).contains("invalid key"));
    assert!(rpc(&mut m, "getServerStorageItem", r#"["channelFilters"]"#).contains(r#""result":null"#));
    rpc(&mut m, "setServerStorageItem", r#"["channelFilters","[1]"]"#);
    assert!(rpc(&mut m, "getServerStorageItem", r#"["channelFilters"]"#).contains(r#""result":"[1]""#));
    m.fail = Some(HostError::Exception(b"Unable to open".to_vec()));
    assert!(rpc(&mut m, "setServerStorageItem", r#"["channelFilters","x"]"#).contains(r#""code":-32603,"message":"Unable to open""#));
}

#[test]
fn relay_tree() {
    let mut m = Mock { channels: vec![channel()], ..Default::default() };
    assert!(rpc(&mut m, "getChannelRelayTree", r#"["hoge"]"#).contains("Channel not found"));
    assert!(rpc(&mut m, "getChannelRelayTree", &format!(r#"["{}"]"#, CHS)).contains("Hit list not found"));

    let host = |a: u8, port: u16| chanhit::Host { ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 10, 0, 0, a], port };
    let none = chanhit::Host { ip: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0], port: 0 };
    let mut me = Hit { tracker: true, uphost: none, ..Default::default() };
    me.rhost = [host(1, 7144), none];
    let mut child = Hit { uphost: host(1, 7144), num_relays: 2, relay: true, ..Default::default() };
    child.rhost = [host(2, 7144), none];
    m.hits = Some(vec![(me, b"10.0.0.1".to_vec()), (child, b"10.0.0.2".to_vec())]);
    let r = rpc(&mut m, "getChannelRelayTree", &format!(r#"["{}"]"#, CHS));
    assert!(r.contains(r#""address":"10.0.0.1","children":[{"address":"10.0.0.2","children":[]"#), "{}", r);
    assert!(r.contains(r#""isRelayFull":false,"isTracker":false,"localDirects":0,"localRelays":2"#), "{}", r);
}

#[test]
fn invoke_direct() {
    let mut m = Mock { channels: vec![channel()], ..Default::default() };
    assert!(matches!(invoke(b"getChannels", vec![], &mut m), Ok(Value::Array(a)) if a.len() == 1));
    assert!(matches!(
        invoke(b"getChannelRelayTree", vec![Value::str(b"hoge")], &mut m),
        Err(CallError::Application(CHANNEL_NOT_FOUND, _))
    ));
    assert!(matches!(invoke(b"nope", vec![], &mut m), Err(CallError::MethodNotFound(_))));
    let found = json::dump(&invoke(b"getChannelsFound", vec![], &mut m).unwrap()).unwrap();
    assert!(found.starts_with(br#"[{"age":0,"bcflags":0,"bitrate":0,"comment":"","desc":"","genre":"","hit_stat":{"closest":0"#));
    // JSON-RPC からは呼べない
    assert!(rpc(&mut m, "getChannelsFound", "[]").contains("Method not found"));
}

#[test]
fn index_txt() {
    let mut c = channel();
    c.info.name = b"a b".to_vec();
    c.uptime = 3661;
    c.total_directs = 3;
    let mut m = Mock { channels: vec![c], ..Default::default() };
    let text = String::from_utf8(channel_index(&mut m, b"1.2.3.4:7144").unwrap()).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "a b<>11111111111111111111111111111111<>1.2.3.4:7144<><><><>3<>0<>500<>FLV<><><><><>a+b<>01:01<>click<><>0"
    );
    // ほかのノードから教わったもの (ヒットがあるもの)
    assert_eq!(lines.next().unwrap(), "<>00000000000000000000000000000000<><><><><>0<>0<>0<><><><><><><>00:00<>click<><>0");
    assert!(lines.next().is_none());

    // 書き出せない文字列があると、C++ 版と同じく例外
    m.channels[0].info.content_type = b"\xff".to_vec();
    assert_eq!(channel_index(&mut m, b"x").unwrap_err(), b"[json.exception.type_error.316] invalid UTF-8 byte at index 0: 0xFF");
}

/// 乱数の欄で index.txt を作ってもパニックしない
#[test]
fn fuzz_index_txt() {
    let mut x: u32 = 9151;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut m = Mock::default();
    for _ in 0..20_000 {
        m.channels = (0..rnd() % 4)
            .map(|_| {
                let mut c = channel();
                let n = (rnd() % 8) as usize;
                c.info.name = (0..n).map(|_| if rnd() % 3 == 0 { rnd() as u8 } else { b'a' + (rnd() % 26) as u8 }).collect();
                c.uptime = rnd();
                c.total_directs = rnd() as i32;
                c.is_broadcasting = rnd() % 4 != 0;
                c
            })
            .collect();
        match channel_index(&mut m, b"1.2.3.4:7144") {
            Ok(t) => assert!(t.ends_with(b"\n")),
            Err(w) => assert!(w.starts_with(b"[json.exception.type_error.316]")),
        }
        m.logs.clear();
    }
}

/// 変異させた要求でパニックしない
#[test]
fn fuzz_requests() {
    let seeds: Vec<String> = vec![
        format!(r#"{{"jsonrpc":"2.0","id":1,"method":"getChannelConnections","params":["{}"]}}"#, CHS),
        r#"{"jsonrpc":"2.0","id":1,"method":"getLog","params":{"from":1,"maxLines":2}}"#.into(),
        r#"{"jsonrpc":"2.0","id":[1.5e300],"method":"setSettings","params":[{"maxRelays":1,"maxRelaysPerChannel":2,"maxDirects":3,"maxUpstreamRate":4}]}"#.into(),
        format!(r#"{{"jsonrpc":"2.0","id":"x","method":"setChannelInfo","params":["{}",{{"name":"n","desc":"d","genre":"g","url":"u","comment":"c"}},{{"url":"u","name":"n","creator":"c","album":"a","genre":"g"}}]}}"#, CHS),
        r#"{"jsonrpc":"2.0","id":null,"method":"fetch","params":["u","n","d","g","c",1,"FLV","ipv6"]}"#.into(),
        r#"{"jsonrpc":"2.0","id":3,"method":"getState","params":[["servMgr","chanMgr","stats","notificationBuffer","sys","ypList"]]}"#.into(),
        format!(r#"{{"jsonrpc":"2.0","id":4,"method":"getChannelRelayTree","params":["{}"]}}"#, CHS),
    ];
    let mut x: u32 = 777;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut m = Mock { channels: vec![channel()], state: br#"{"a":[1,2,{"b":null}]}"#.to_vec(), ..Default::default() };
    m.hits = Some(vec![(Hit::default(), b"0.0.0.0".to_vec()); 3]);
    for i in 0..200_000 {
        let mut s = seeds[i % seeds.len()].clone().into_bytes();
        for _ in 0..1 + rnd() % 3 {
            let pos = (rnd() as usize) % (s.len() + 1);
            match rnd() % 3 {
                0 if pos < s.len() => s[pos] = rnd() as u8,
                1 => s.insert(pos, rnd() as u8),
                _ if pos < s.len() => {
                    s.remove(pos);
                }
                _ => {}
            }
        }
        let out = call(&s, &mut m).unwrap();
        assert!(json::parse(&out).is_ok());
        m.logs.clear();
        m.calls.clear();
    }
}
