#!/usr/bin/env python3
"""C++ 版 rtmp-server と Rust 版に同じ RTMP 入力を与えて、出力を比較する。

比較するもの:
  - 出力先 (HTTP シンク) が受け取ったリクエスト行と FLV 本体
  - サーバーからクライアントへ返したバイト列 (S1 の乱数部だけは実装ごとに違うので除く)
  - セッション後にサーバーが生きていて、次の接続を受け付けるか

使い方:
  differential.py CPP_BIN RUST_BIN            # シナリオ + ファズ
  differential.py CPP_BIN RUST_BIN --fuzz 500 --seed 1
"""
import argparse, random, socket, struct, subprocess, sys, threading, time

# ---------------------------------------------------------------- AMF0 / RTMP builders
def amf_num(x):  return b"\x00" + struct.pack(">d", x)
def amf_str(s):
    s = s.encode() if isinstance(s, str) else s
    return b"\x02" + struct.pack(">H", len(s)) + s
def amf_null():  return b"\x05"
def amf_bool(b): return b"\x01" + (b"\x01" if b else b"\x00")
def amf_obj(d):
    out = b"\x03"
    for k, v in d.items():
        kb = k.encode()
        out += struct.pack(">H", len(kb)) + kb + v
    return out + b"\x00\x00\x09"
def amf_ecma(d):
    out = b"\x08" + struct.pack(">I", len(d))
    for k, v in d.items():
        kb = k.encode()
        out += struct.pack(">H", len(kb)) + kb + v
    return out + b"\x00\x00\x09"

def chunk_msg(cs, ts, mtype, sid, payload, chunk_size=128, fmt=0, tdelta=None):
    """1 メッセージを fmt 0/1/2 の先頭チャンク + fmt 3 の続きチャンクにして返す。"""
    b0 = lambda f: bytes([(f << 6) | cs])
    if fmt == 0:
        hdr = b0(0) + struct.pack(">I", ts)[1:] + struct.pack(">I", len(payload))[1:] + bytes([mtype]) + struct.pack("<I", sid)
    elif fmt == 1:
        hdr = b0(1) + struct.pack(">I", tdelta)[1:] + struct.pack(">I", len(payload))[1:] + bytes([mtype])
    elif fmt == 2:
        hdr = b0(2) + struct.pack(">I", tdelta)[1:]
    else:
        hdr = b0(3)
    out = hdr + payload[:chunk_size]
    rest = payload[chunk_size:]
    while rest:
        out += b0(3) + rest[:chunk_size]
        rest = rest[chunk_size:]
    return out

def cmd(*vals): return b"".join(vals)

C0C1C2 = b"\x03" + struct.pack(">II", 0, 0) + bytes((i * 7 + 3) & 0xff for i in range(1528)) + bytes(1536)

def connect_msg():
    return chunk_msg(3, 0, 0x14, 0, cmd(amf_str("connect"), amf_num(1), amf_obj({"app": amf_str("live"), "tcUrl": amf_str("rtmp://localhost/live")})))

def std_publish_prefix():
    return (connect_msg()
        + chunk_msg(3, 0, 0x14, 0, cmd(amf_str("releaseStream"), amf_num(2), amf_null(), amf_str("key")))
        + chunk_msg(3, 0, 0x14, 0, cmd(amf_str("FCPublish"), amf_num(3), amf_null(), amf_str("key")))
        + chunk_msg(3, 0, 0x14, 0, cmd(amf_str("createStream"), amf_num(4), amf_null()))
        + chunk_msg(4, 0, 0x14, 1, cmd(amf_str("publish"), amf_num(5), amf_null(), amf_str("key"), amf_str("live"))))

def metadata(with_set=True, av=True):
    d = {"width": amf_num(640), "height": amf_num(360), "framerate": amf_num(30)}
    if av:
        d["videocodecid"] = amf_num(7); d["audiocodecid"] = amf_num(10)
    body = (amf_str("@setDataFrame") if with_set else b"") + amf_str("onMetaData") + amf_ecma(d)
    return chunk_msg(4, 0, 0x12, 1, body)

def av_stream(n=20, chunk=4096, big=False):
    out = chunk_msg(2, 0, 0x01, 0, struct.pack(">I", chunk))  # client SetChunkSize
    ts = 0
    for i in range(n):
        vp = bytes([0x17, 0x01, 0, 0, 0]) + bytes((i + j) & 0xff for j in range(3000 if big else 200 + i * 37))
        ap = bytes([0xaf, 0x01]) + bytes((i * 3 + j) & 0xff for j in range(100 + i))
        if i == 0:
            out += chunk_msg(6, ts, 0x09, 1, vp, chunk)
            out += chunk_msg(4 + 2, ts, 0x08, 1, ap, chunk) if False else chunk_msg(5, ts, 0x08, 1, ap, chunk)
        else:
            ts += 33
            out += chunk_msg(6, ts, 0x09, 1, vp, chunk, fmt=1, tdelta=33)
            out += chunk_msg(5, ts, 0x08, 1, ap, chunk, fmt=1, tdelta=33)
    return out

END = chunk_msg(3, 0, 0x14, 0, cmd(amf_str("deleteStream"), amf_num(6), amf_null(), amf_num(1)))

def normal_session():
    return C0C1C2 + std_publish_prefix() + metadata() + av_stream() + END

SCENARIOS = {
    "normal": normal_session(),
    "normal-no-delete": C0C1C2 + std_publish_prefix() + metadata() + av_stream(),
    "big-video-messages": C0C1C2 + std_publish_prefix() + metadata() + av_stream(8, 4096, big=True) + END,
    "small-client-chunk": C0C1C2 + std_publish_prefix() + metadata() + av_stream(5, 128) + END,
    "audio-only-metadata": C0C1C2 + std_publish_prefix() + chunk_msg(4, 0, 0x12, 1, amf_str("@setDataFrame") + amf_str("onMetaData") + amf_ecma({"audiocodecid": amf_num(10)})) + av_stream(3) + END,
    # --- 異常系 ---
    "video-before-metadata": C0C1C2 + std_publish_prefix() + chunk_msg(6, 0, 0x09, 1, b"\x17\x01\0\0\0abc") + END,
    "second-metadata": C0C1C2 + std_publish_prefix() + metadata() + metadata() + END,
    "metadata-without-setdataframe": C0C1C2 + std_publish_prefix() + metadata(with_set=False) + av_stream(2) + END,
    "fcpublish-missing-param": C0C1C2 + connect_msg() + chunk_msg(3, 0, 0x14, 0, cmd(amf_str("FCPublish"), amf_num(3), amf_null())),
    "bool-transaction-id": C0C1C2 + chunk_msg(3, 0, 0x14, 0, cmd(amf_str("connect"), amf_bool(True), amf_null())),
    "bad-c0": b"\x40" + C0C1C2[1:],
    "cs-id-1": C0C1C2 + bytes([0x01, 0x00]) + b"x" * 20,
    "fmt1-without-prev": C0C1C2 + bytes([0x40 | 5]) + b"\0\0\0\0\0\x04\x09abcd",
    "big-control-message": C0C1C2 + bytes([3]) + b"\0\0\0" + struct.pack(">I", 0x200000)[1:] + b"\x14" + b"\0\0\0\0" + b"x" * 128,
    "extended-timestamp": C0C1C2 + bytes([3]) + b"\xff\xff\xff\0\0\x04\x14\0\0\0\0" + b"\0\0\0\0",
    "set-chunk-size-zero": C0C1C2 + chunk_msg(2, 0, 0x01, 0, struct.pack(">I", 0)),
    "set-chunk-size-short": C0C1C2 + chunk_msg(2, 0, 0x01, 0, b"\0\0"),
    "unknown-message-type": C0C1C2 + chunk_msg(2, 0, 0x33, 0, b"abc") + END,
    "window-ack-size": C0C1C2 + chunk_msg(2, 0, 0x05, 0, struct.pack(">I", 2500000)) + END,
    "truncated-mid-message": (C0C1C2 + std_publish_prefix() + metadata() + av_stream(3))[:-30],
    "silent-client-eof": C0C1C2[:1 + 1536],
}
# fmt2/fmt3 の再利用シナリオは手で組む: 同じ cs で fmt0 → fmt2 → fmt3(次のメッセージ)
_m = b"\x17\x01\0\0\0" + b"a" * 50
SCENARIOS["fmt2-and-fmt3-reuse"] = (C0C1C2 + std_publish_prefix() + metadata()
    + chunk_msg(6, 100, 0x09, 1, _m)
    + bytes([(2 << 6) | 6]) + struct.pack(">I", 33)[1:] + _m
    + bytes([(3 << 6) | 6]) + _m
    + END)

# ---------------------------------------------------------------- harness
def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

class Sink(threading.Thread):
    """peercast の代わり。POST を受けて、以降のバイト列を全部溜める。"""
    def __init__(self):
        super().__init__(daemon=True)
        self.srv = socket.socket(); self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", 0)); self.srv.listen(4)
        self.port = self.srv.getsockname()[1]
        self.conns = []           # 接続ごとの受信バイト列
        self.lock = threading.Lock()
        self.start()
    def run(self):
        while True:
            try: c, _ = self.srv.accept()
            except OSError: return
            threading.Thread(target=self.handle, args=(c,), daemon=True).start()
    def handle(self, c):
        buf = b""
        c.settimeout(10)
        try:
            while True:
                d = c.recv(65536)
                if not d: break
                buf += d
        except Exception: pass
        with self.lock: self.conns.append(buf)

def run_case(binary, data, timeout=8):
    sink = Sink()
    port = free_port()
    proc = subprocess.Popen([binary, "-p", str(port), f"http://127.0.0.1:{sink.port}/?name=t&type=FLV"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try: c = socket.create_connection(("127.0.0.1", port), timeout=2); break
            except OSError: time.sleep(0.05)
        else:
            return {"error": "server did not start"}
        c.settimeout(timeout)
        got = b""
        try:
            c.sendall(data)
            c.shutdown(socket.SHUT_WR)
        except OSError: pass
        try:
            while True:
                d = c.recv(65536)
                if not d: break
                got += d
        except socket.timeout:
            got += b"<TIMEOUT>"
        except OSError: pass
        c.close()
        time.sleep(0.15)
        # 生きているか、次の接続を受けられるか
        alive = proc.poll() is None
        second = False
        if alive:
            try:
                c2 = socket.create_connection(("127.0.0.1", port), timeout=2); c2.close(); second = True
            except OSError: pass
        with sink.lock:
            conns = list(sink.conns)
        # S0 + S1 (1537 バイト) のうち乱数部 (先頭 9 バイトの後) は実装ごとに違うので除く。
        if len(got) >= 1 + 1536: got = got[:9] + got[1537:]
        return {"resp": got, "sink": conns, "alive": alive and second, "rc": proc.poll()}
    finally:
        if proc.poll() is None: proc.kill()
        proc.wait()
        sink.srv.close()

def compare(name, data, cpp, rust):
    a, b = run_case(cpp, data), run_case(rust, data)
    diffs = []
    if a.get("error") or b.get("error"): diffs.append(f"start error cpp={a.get('error')} rust={b.get('error')}")
    else:
        if a["sink"] != b["sink"]: diffs.append(f"FLV/sink differs (cpp {[len(x) for x in a['sink']]} bytes vs rust {[len(x) for x in b['sink']]})")
        if a["resp"] != b["resp"]: diffs.append(f"server response differs (cpp {len(a['resp'])} vs rust {len(b['resp'])} bytes)")
        if a["alive"] != b["alive"]: diffs.append(f"survival differs (cpp alive={a['alive']} rust alive={b['alive']})")
    return diffs, a, b

def mutate(data, rng):
    d = bytearray(data)
    for _ in range(rng.choice([1, 1, 2, 3, 8])):
        kind = rng.random()
        if not d: break
        i = rng.randrange(len(d))
        if kind < 0.5: d[i] = rng.randrange(256)
        elif kind < 0.7: d[i] ^= 1 << rng.randrange(8)
        elif kind < 0.85: del d[i:i + rng.randrange(1, 40)]
        else: d[i:i] = bytes(rng.randrange(256) for _ in range(rng.randrange(1, 40)))
    return bytes(d)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cpp"); ap.add_argument("rust")
    ap.add_argument("--fuzz", type=int, default=0); ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--only")
    args = ap.parse_args()

    bad = 0
    for name, data in SCENARIOS.items():
        if args.only and args.only != name: continue
        diffs, a, b = compare(name, data, args.cpp, args.rust)
        status = "SAME" if not diffs else "DIFF"
        print(f"[{status}] {name:32s} sink={[len(x) for x in b.get('sink', [])]} alive(cpp/rust)={a.get('alive')}/{b.get('alive')}")
        for d in diffs: print("        ", d)
        bad += bool(diffs)

    if args.fuzz:
        rng = random.Random(args.seed)
        base = [SCENARIOS["normal"], SCENARIOS["big-video-messages"], SCENARIOS["fmt2-and-fmt3-reuse"]]
        stats = {"same": 0, "diff": 0, "rust_dead": 0, "cpp_dead": 0}
        examples = []
        for n in range(args.fuzz):
            data = mutate(rng.choice(base), rng)
            diffs, a, b = compare("fuzz", data, args.cpp, args.rust)
            if not b.get("alive"): stats["rust_dead"] += 1
            if not a.get("alive"): stats["cpp_dead"] += 1
            if diffs:
                stats["diff"] += 1
                if len(examples) < 8: examples.append((n, diffs, data))
            else: stats["same"] += 1
        print("fuzz:", stats)
        for n, diffs, data in examples:
            print(f"  case #{n}: {diffs}")
            open(f"/tmp/fuzz_diff_{n}.bin", "wb").write(data)
        bad += stats["rust_dead"]
    sys.exit(1 if bad else 0)

if __name__ == "__main__":
    main()
