#!/usr/bin/env python3
"""Rust 版だけを対象にした運用上の頑健性テスト。
  1. 何も送らずに居座るクライアント (slowloris) が、読み取りタイムアウトで切られ、次の配信を受けられること
  2. 出力先 (PeerCast) が途中で切断しても、サーバーが落ちずに次の配信を受けられること
使い方: robustness.py RUST_BIN
"""
import socket, subprocess, sys, threading, time, importlib.util
spec = importlib.util.spec_from_file_location("d", __file__.replace("robustness.py", "differential.py"))
d = importlib.util.module_from_spec(spec); spec.loader.exec_module(d)

binary = sys.argv[1]
ok = True
def check(name, cond, extra=""):
    global ok
    print(("[PASS] " if cond else "[FAIL] ") + name + (" " + extra if extra else ""))
    ok &= bool(cond)

# --- 1. slowloris
sink = d.Sink(); port = d.free_port()
p = subprocess.Popen([binary, "-p", str(port), f"http://127.0.0.1:{sink.port}/?name=t"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.5)
slow = socket.create_connection(("127.0.0.1", port)); slow.sendall(b"\x03")   # C0 だけ送って黙る
t0 = time.time()
good = socket.create_connection(("127.0.0.1", port)); good.settimeout(60)     # 待ち行列に並ぶ
good.sendall(d.SCENARIOS["normal"]); good.shutdown(socket.SHUT_WR)
resp = b""
while True:
    x = good.recv(65536)
    if not x: break
    resp += x
waited = time.time() - t0
time.sleep(0.3)
flv = [c for c in sink.conns if b"FLV" in c]
check("slowloris: 黙っているクライアントは約 30 秒で切られ、次の配信が処理される", 25 <= waited <= 40 and len(flv) == 1, f"(waited {waited:.1f}s, flv sinks={len(flv)})")

# --- 2. 出力先が即切断
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); srv.bind(("127.0.0.1", 0)); srv.listen(4)
sport = srv.getsockname()[1]
def slam():
    while True:
        try: c, _ = srv.accept()
        except OSError: return
        c.close()
threading.Thread(target=slam, daemon=True).start()
port2 = d.free_port()
p2 = subprocess.Popen([binary, "-p", str(port2), f"http://127.0.0.1:{sport}/?name=t"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.5)
for i in range(3):
    c = socket.create_connection(("127.0.0.1", port2)); c.settimeout(10)
    try: c.sendall(d.SCENARIOS["big-video-messages"]); c.shutdown(socket.SHUT_WR)
    except OSError: pass
    try:
        while c.recv(65536): pass
    except OSError: pass
    c.close()
time.sleep(0.3)
check("出力先が切断してもサーバーは生き残り、次の接続を受け付ける", p2.poll() is None)

# --- 3. 出力先が開けない (接続拒否) → C++ 版は例外で落ちる箇所
port3 = d.free_port(); dead = d.free_port()
p3 = subprocess.Popen([binary, "-p", str(port3), f"http://127.0.0.1:{dead}/?name=t"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.5)
c = socket.create_connection(("127.0.0.1", port3)); c.sendall(b"\x03"); time.sleep(0.5); c.close(); time.sleep(0.3)
check("出力先に接続できなくてもサーバーは落ちない", p3.poll() is None)

for x in (p, p2, p3): x.kill()
sys.exit(0 if ok else 1)
