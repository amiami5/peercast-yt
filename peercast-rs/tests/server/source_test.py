#!/usr/bin/env python3
"""配信元の種類ごとに試す: HTTP の取得 (fetch)、ShoutCast と Icecast の放送 (MP3)、ICY メタデータ付きの視聴。

使い方: source_test.py BIN
"""
import http.server
import os
import socket
import socketserver
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import relay_test as rt  # noqa: E402

PORT = 7180
HTTPD_PORT = 7189


def mp3_frames(n, tag=0):
    # MPEG1 Layer III 128kbps 44.1kHz、パディングなし: 417 バイト
    frame = b'\xff\xfb\x90\x64' + bytes([tag & 0xff]) * 413
    return frame * n


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.startswith('/live.flv'):
            self.send_response(200)
            self.send_header('Content-Type', 'video/x-flv')
            self.end_headers()
            try:
                start = time.time()
                for i, chunk in enumerate(rt.flv_stream(30)):
                    self.wfile.write(chunk)
                    t = start + i / 30.0
                    if t > time.time():
                        time.sleep(t - time.time())
            except (BrokenPipeError, ConnectionResetError):
                pass
        else:
            self.send_response(404)
            self.end_headers()


def serve():
    socketserver.ThreadingTCPServer.allow_reuse_address = True
    httpd = socketserver.ThreadingTCPServer(('127.0.0.1', HTTPD_PORT), Handler)
    httpd.daemon_threads = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def icy_push(port, first_line, headers, seconds, done):
    s = socket.create_connection(('127.0.0.1', port))
    s.sendall(first_line + b'\r\n')
    time.sleep(0.2)
    s.sendall(b''.join(h + b'\r\n' for h in headers) + b'\r\n')
    resp = s.recv(100)
    start = time.time()
    i = 0
    try:
        while time.time() - start < seconds and not done.is_set():
            s.sendall(mp3_frames(38, i))
            i += 1
            time.sleep(1.0)
    except (BrokenPipeError, ConnectionResetError):
        pass
    s.close()
    return resp


def main():
    binary = sys.argv[1]
    httpd = serve()
    orig = rt.INI
    ini = open(orig).read().replace('password = ', 'password = pass', 1)
    os.makedirs(rt.WORK, exist_ok=True)
    open(rt.WORK + '/source_test.ini', 'w').write(ini)
    rt.INI = rt.WORK + '/source_test.ini'
    p = rt.setup('source', binary, PORT)
    time.sleep(1.5)
    ok = True
    done = threading.Event()
    try:
        # HTTP の取得
        r = rt.jrpc(PORT, 'fetch', {'url': 'http://127.0.0.1:%d/live.flv' % HTTPD_PORT, 'name': 'fetched', 'desc': '', 'genre': '', 'contact': '', 'bitrate': 0, 'type': 'FLV'})
        print('fetch:', r)
        cid = r['result']
        time.sleep(3)
        ok &= rt.check_flv('fetch', rt.read_stream(PORT, '/stream/%s.flv' % cid, 100000))

        # ShoutCast (パスワードの行) と Icecast (SOURCE)
        t1 = threading.Thread(target=icy_push, args=(PORT, b'pass', [b'icy-name:shout', b'icy-br:128', b'content-type:audio/mpeg'], 20, done))
        t2 = threading.Thread(target=icy_push, args=(PORT, b'SOURCE pass /mnt', [b'ice-name:ice', b'ice-bitrate:128', b'content-type:audio/mpeg'], 20, done))
        t1.start()
        t2.start()
        time.sleep(3)
        chans = rt.jrpc(PORT, 'getChannels')['result']
        names = {c['info']['name']: c['channelId'] for c in chans}
        print('channels:', sorted(names))
        for n in ('shout', 'ice'):
            if n not in names:
                print(n, 'NG not created')
                ok = False
                continue
            s = socket.create_connection(('127.0.0.1', PORT), timeout=10)
            s.sendall(('GET /stream/%s.mp3 HTTP/1.0\r\nIcy-MetaData:1\r\n\r\n' % names[n]).encode())
            buf = b''
            end = time.time() + 5
            while len(buf) < 20000 and time.time() < end:
                d = s.recv(65536)
                if not d:
                    break
                buf += d
            s.close()
            head = buf[:buf.find(b'\r\n\r\n')]
            good = head.startswith(b'ICY 200 OK') and b'icy-metaint:' in head
            print('%-10s %s %r' % (n, 'OK ' if good else 'NG ', head.split(b'\r\n')[:3]))
            ok &= good
    finally:
        done.set()
        time.sleep(1.5)
        p.send_signal(2)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            print('did not exit on SIGINT')
            p.kill()
            ok = False
        httpd.shutdown()
    print('RESULT', 'ok' if ok else 'failed')


if __name__ == '__main__':
    main()
