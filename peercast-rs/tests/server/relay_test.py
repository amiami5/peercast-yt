#!/usr/bin/env python3
"""配信 (HTTP Push) → 直接視聴 → 別のサーバーでの中継を試す。

使い方: relay_test.py SRC_BIN RELAY_BIN
  SRC_BIN   配信を受けるサーバーの実行ファイル
  RELAY_BIN 中継するサーバーの実行ファイル
"""
import http.client
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time

# リポジトリの一番上 (peercast-rs/tests/server の 3 つ上)
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
# ui/linux で make したときの配布用のディレクトリ (html などを使う)
TEMPLATE = ROOT + '/ui/linux/peercast-yt'
INI = ROOT + '/bvt/peercast.ini.master'
# サーバーを起こすディレクトリ
WORK = os.environ.get('PCYT_TEST_DIR', '/tmp/pcyt-servertest')
RS = ROOT + '/peercast-rs/target/release/peercast'
# make WITH_RUST_SERVER=no TARGET=peercast-cxx peercast-cxx で作る C++ 版
CXX = ROOT + '/ui/linux/peercast-cxx'


def setup(name, binary, port):
    d = WORK + '/relaytest/' + name
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(TEMPLATE, d)
    shutil.copy(binary, d + '/peercast')
    ini = open(INI).read().replace('serverPort = 7144', 'serverPort = %d' % port)
    ini = ini.replace('logLevel = 7', 'logLevel = 3')
    # 実在の YP に接続しない
    ini = ini.replace('rootHost = yp.pcgw.pgw.jp:7146', 'rootHost = ')
    open(d + '/peercast.ini', 'w').write(ini)
    log = open(d + '/log.txt', 'w')
    # トークンなどを書くディレクトリも作業用の下にする
    env = dict(os.environ, XDG_STATE_HOME=d + '/state', XDG_CACHE_HOME=d + '/cache', XDG_CONFIG_HOME=d + '/conf')
    p = subprocess.Popen(['./peercast', '-i', 'peercast.ini', '-P', '.'], cwd=d, stdout=log, stderr=subprocess.STDOUT, env=env)
    return p


def flv_tag(ty, ts, data):
    head = struct.pack('>B', ty) + struct.pack('>I', len(data))[1:] + struct.pack('>I', ts)[1:] + bytes([ts >> 24 & 0xff]) + b'\0\0\0'
    return head + data + struct.pack('>I', len(head) + len(data))


def flv_stream(seconds):
    """FLV のヘッダーと、1 秒ごとにキーフレームがある 30fps の映像のタグ"""
    yield b'FLV\x01\x01\x00\x00\x00\x09' + b'\0\0\0\0'
    meta = b'\x02\x00\x0aonMetaData\x08\x00\x00\x00\x00\x00\x00\x09'
    yield flv_tag(18, 0, meta)
    for i in range(seconds * 30):
        key = (i % 30 == 0)
        body = bytes([0x12 if key else 0x22]) + bytes([i & 0xff]) * 2000
        yield flv_tag(9, i * 33, body)


def push(port, name, seconds, done):
    s = socket.create_connection(('127.0.0.1', port))
    s.sendall(('POST /?name=%s&type=FLV&bitrate=500 HTTP/1.1\r\nHost: 127.0.0.1:%d\r\n'
               'Content-Type: video/x-flv\r\nTransfer-Encoding: chunked\r\n\r\n' % (name, port)).encode())
    start = time.time()
    n = 0
    for chunk in flv_stream(seconds):
        s.sendall(b'%x\r\n' % len(chunk) + chunk + b'\r\n')
        n += 1
        # 30fps の速さで
        target = start + n / 30.0
        if target > time.time():
            time.sleep(target - time.time())
        if done.is_set():
            break
    s.close()


def jrpc(port, method, params=None):
    c = http.client.HTTPConnection('127.0.0.1', port, timeout=10)
    body = json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params or []})
    c.request('POST', '/api/1', body, {'Content-Type': 'application/json'})
    return json.loads(c.getresponse().read())


def read_stream(port, path, nbytes, timeout=20):
    s = socket.create_connection(('127.0.0.1', port), timeout=timeout)
    s.sendall(('GET %s HTTP/1.0\r\nHost: 127.0.0.1:%d\r\n\r\n' % (path, port)).encode())
    buf = b''
    end = time.time() + timeout
    while len(buf) < nbytes and time.time() < end:
        try:
            d = s.recv(65536)
        except socket.timeout:
            break
        if not d:
            break
        buf += d
    s.close()
    return buf


def check_flv(label, data):
    i = data.find(b'\r\n\r\n')
    head, body = data[:i], data[i + 4:]
    status = head.split(b'\r\n')[0]
    ok = status.endswith(b'200 OK') and body.startswith(b'FLV\x01')
    print('%-10s %s status=%r body=%d bytes' % (label, 'OK ' if ok else 'NG ', status, len(body)))
    return ok


def main():
    src_bin, relay_bin = sys.argv[1], sys.argv[2]
    a = setup('src', src_bin, 7150)
    b = setup('relay', relay_bin, 7151)
    time.sleep(1.5)
    done = threading.Event()
    t = threading.Thread(target=push, args=(7150, 'relaytest', 40, done))
    t.start()
    ok = True
    try:
        time.sleep(3)
        chans = jrpc(7150, 'getChannels')['result']
        if not chans:
            print('no channel')
            return 1
        cid = chans[0]['channelId']
        print('channel', cid, chans[0]['status']['status'])
        ok &= check_flv('direct', read_stream(7150, '/stream/%s.flv' % cid, 200000))
        ok &= check_flv('relay', read_stream(7151, '/stream/%s.flv?tip=127.0.0.1:7150' % cid, 200000, timeout=30))
        rc = jrpc(7151, 'getChannels')['result']
        print('relay side:', [(c['channelId'][:8], c['status']['status']) for c in rc])
        conns = jrpc(7150, 'getChannelConnections', [cid])['result']
        print('src connections:', [(c['type'], c['status']) for c in conns])
    finally:
        done.set()
        t.join()
        for p in (a, b):
            p.send_signal(2)
        for p in (a, b):
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                print('did not exit on SIGINT')
                p.kill()
                ok = False
    print('RESULT', 'ok' if ok else 'failed')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
