#!/usr/bin/env python3
"""C++ 版と Rust 版のサーバーを同じ設定で起動し、同じ要求を送って応答を比べる。

時刻、セッション ID、ポート番号、Date ヘッダーなど、起動ごとに変わるものは伏せて比べる。
"""
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import relay_test as rt  # noqa: E402

CXX = rt.CXX
RS = rt.RS
PORTS = {'cxx': 7160, 'rs': 7161}


def setup(name, binary, port):
    d = rt.WORK + '/serverdiff/' + name
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(rt.TEMPLATE, d)
    shutil.copy(binary, d + '/peercast')
    ini = open(rt.INI).read().replace('serverPort = 7144', 'serverPort = %d' % port)
    ini = ini.replace('rootHost = yp.pcgw.pgw.jp:7146', 'rootHost = ')
    open(d + '/peercast.ini', 'w').write(ini)
    log = open(d + '/log.txt', 'w')
    return subprocess.Popen(['./peercast', '-i', 'peercast.ini', '-P', '.'], cwd=d, stdout=log, stderr=subprocess.STDOUT,
                            env=dict(os.environ, XDG_STATE_HOME=d + '/state', XDG_CACHE_HOME=d + '/cache', XDG_CONFIG_HOME=d + '/conf'))


def raw(port, data, timeout=5):
    s = socket.create_connection(('127.0.0.1', port), timeout=timeout)
    s.sendall(data)
    buf = b''
    end = time.time() + timeout
    while time.time() < end:
        try:
            d = s.recv(65536)
        except (socket.timeout, ConnectionResetError):
            break
        if not d:
            break
        buf += d
    s.close()
    return buf


def get(path, extra=b''):
    return lambda port: raw(port, b'GET ' + path + b' HTTP/1.0\r\nHost: 127.0.0.1:%d\r\n' % port + extra + b'\r\n')


def post(path, body, extra=b''):
    return lambda port: raw(port, b'POST ' + path + b' HTTP/1.0\r\nHost: 127.0.0.1:%d\r\nContent-Length: %d\r\n' % (port, len(body)) + extra + b'\r\n' + body)


def jrpc(method, params=None, id=1):
    body = json.dumps({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params if params is not None else []}).encode()
    return post(b'/api/1', body, b'Content-Type: application/json\r\n')


def normalize(b, port, sids):
    b = b.replace(b'%d' % port, b'PORT')
    for sid in sids:
        b = b.replace(sid, b'SESSIONID')
    b = re.sub(rb'Date: [^\r\n]*', b'Date: X', b)
    b = re.sub(rb'Last-Modified: [^\r\n]*', b'Last-Modified: X', b)
    b = re.sub(rb'\d{4}/\d\d/\d\d \d\d:\d\d:\d\d(\.\d+)?', b'TIME', b)
    b = re.sub(rb'(Sun|Mon|Tue|Wed|Thu|Fri|Sat) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) [ \d]\d \d\d:\d\d:\d\d \d{4}', b'CTIME', b)
    return b


def mask_numbers(b):
    return re.sub(rb'\d+', b'#', b)


def main():
    procs = {k: setup(k, CXX if k == 'cxx' else RS, p) for k, p in PORTS.items()}
    time.sleep(1.5)
    done = threading.Event()
    pushers = [threading.Thread(target=rt.push, args=(p, 'difftest', 60, done)) for p in PORTS.values()]
    for t in pushers:
        t.start()
    time.sleep(3)
    try:
        chans = {k: rt.jrpc(p, 'getChannels')['result'] for k, p in PORTS.items()}
        cids = {k: v[0]['channelId'].encode() for k, v in chans.items()}
        print('channel ids:', cids)
        reqs = {k: make_reqs(cids[k]) for k in PORTS}
        run(reqs, cids)
    finally:
        done.set()
        for t in pushers:
            t.join()
        for p in procs.values():
            p.send_signal(2)
        for p in procs.values():
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()


def make_reqs(cid):
    if True:
        reqs = [
            ('root', get(b'/')),
            ('html', get(b'/html')),
            ('html/', get(b'/html/')),
            ('html/index.html', get(b'/html/index.html')),
            ('pls', get(b'/pls/' + cid)),
            ('pls-missing', get(b'/pls/0123456789ABCDEF0123456789ABCDEF')),
            ('stream-missing', get(b'/stream/0123456789ABCDEF0123456789ABCDEF.flv')),
            ('api1', get(b'/api/1')),
            ('public-off', get(b'/public/')),
            ('assets-css', get(b'/assets/css/peercast.css')),
            ('assets-ico', get(b'/assets/images/favicon.ico')),
            ('assets-missing', get(b'/assets/nothing.png')),
            ('assets-trav', get(b'/assets/../html/en/index.html')),
            ('assets-ims', get(b'/assets/css/peercast.css', b'If-Modified-Since: Fri, 01 Jan 2100 00:00:00 GMT\r\n')),
            ('html-trav', get(b'/html/../peercast.ini')),
            ('html-png', get(b'/html/en/nothing.png')),
            ('viewxml', get(b'/admin?cmd=viewxml')),
            ('admin-bad', get(b'/admin?cmd=nosuch')),
            ('admin-redirect', get(b'/admin?cmd=redirect&url=example.com/a%20b')),
            ('admin-dump', get(b'/admin?cmd=dump_hitlists')),
            ('admin-chanfeedlog', get(b'/admin?cmd=chanfeedlog&index=3')),
            ('admin-stop_servent-bad', get(b'/admin?cmd=stop_servent&servent_id=x')),
            ('admin-stop_servent-404', get(b'/admin?cmd=stop_servent&servent_id=9999')),
            ('admin-speedtest-bad', get(b'/admin?cmd=delete_speedtest&index=-1')),
            ('admin-keep', get(b'/admin?cmd=keep&id=' + cid)),
            ('admin-keep2', get(b'/admin?cmd=keep&id=' + cid, b'Referer: http://x/y\r\n')),
            ('admin-update-nochan', get(b'/admin?cmd=update_channel_info&id=00')),
            ('admin-chooselang', get(b'/admin?cmd=chooseLanguage&htmlPath=ja', b'Referer: http://h/html/en/settings.html\r\n')),
            ('admin-chooselang-bad', get(b'/admin?cmd=chooseLanguage&htmlPath=../x')),
            ('admin-chooselang-back', get(b'/admin?cmd=chooseLanguage&htmlPath=en')),
            ('admin-appearance', get(b'/admin?cmd=customizeAppearance&preferredTheme=dark&x=1')),
            ('admin-rtmp-bad', get(b'/admin?cmd=control_rtmp&action=zzz')),
            ('cmd-noq', get(b'/cmd?x=1')),
            ('cmd-help', get(b'/cmd?q=help')),
            ('cmd-echo', get(b'/cmd?q=echo+-v+a+%22b+c%22')),
            ('cmd-expr', get(b'/cmd?q=expr+servMgr.serverName')),
            ('cmd-expr2', get(b'/cmd?q=expr+%28%3D%3D+1+1%29')),
            ('cmd-bad', get(b'/cmd?q=nosuch')),
            ('cmd-empty', get(b'/cmd?q=%20')),
            ('cmd-chan-ls', get(b'/cmd?q=chan+ls')),
            ('cmd-chan-show-bad', get(b'/cmd?q=chan+show+ZZZ')),
            ('cmd-chan', get(b'/cmd?q=chan')),
            ('cmd-flag-ls', get(b'/cmd?q=flag+ls')),
            ('cmd-flag-bad', get(b'/cmd?q=flag+set+nosuch+true')),
            ('cmd-sleep-usage', get(b'/cmd?q=sleep')),
            ('cmd-echo-badopt', get(b'/cmd?q=echo+--zzz')),
            ('cmd-nslookup', get(b'/cmd?q=nslookup+127.0.0.1')),
            ('cmd-help-chan', get(b'/cmd?q=help+chan')),
            ('bad-method', lambda port: raw(port, b'FOO / HTTP/1.0\r\n\r\n')),
            ('post-bad', post(b'/zzz', b'')),
            ('post-badline', lambda port: raw(port, b'POST /a b HTTP/1.0\r\n\r\n')),
            ('giv-bad', lambda port: raw(port, b'GIV /0123456789ABCDEF0123456789ABCDEF\r\n\r\n')),
            ('jrpc-nolen', lambda port: raw(port, b'POST /api/1 HTTP/1.0\r\nHost: 127.0.0.1:%d\r\n\r\n' % port)),
            ('jrpc-short', lambda port: raw(port, b'POST /api/1 HTTP/1.0\r\nHost: 127.0.0.1:%d\r\nContent-Length: 50\r\n\r\n{}' % port)),
            ('jrpc-cross', post(b'/api/1', b'{}', b'Origin: http://evil.example\r\n')),
            ('jrpc-parse', post(b'/api/1', b'{zz')),
            ('jrpc-version', jrpc('getVersionInfo')),
            ('jrpc-settings', jrpc('getSettings')),
            ('jrpc-status', jrpc('getStatus')),
            ('jrpc-channels', jrpc('getChannels')),
            ('jrpc-chinfo', jrpc('getChannelInfo', [cid.decode()])),
            ('jrpc-chstatus', jrpc('getChannelStatus', [cid.decode()])),
            ('jrpc-chconns', jrpc('getChannelConnections', [cid.decode()])),
            ('jrpc-relaytree', jrpc('getChannelRelayTree', [cid.decode()])),
            ('jrpc-logsettings', jrpc('getLogSettings')),
            ('jrpc-notif', jrpc('getNotificationMessages')),
            ('jrpc-yps', jrpc('getYellowPages')),
            ('jrpc-ypch', jrpc('getYPChannels')),
            ('jrpc-plugins', jrpc('getPlugins')),
            ('jrpc-state', jrpc('getState', [['servMgr', 'chanMgr', 'stats', 'notificationBuffer', 'ypList']])),
            ('jrpc-storage-set', jrpc('setServerStorageItem', ['k1', 'v1'])),
            ('jrpc-storage-get', jrpc('getServerStorageItem', ['k1'])),
            ('jrpc-nochan', jrpc('getChannelInfo', ['00'])),
            ('jrpc-nomethod', jrpc('zzz')),
            ('jrpc-bump', jrpc('bumpChannel', [cid.decode()])),
            ('jrpc-setsettings', jrpc('setSettings', [{'maxRelays': 3, 'maxDirects': 4}])),
            ('jrpc-settings2', jrpc('getSettings')),
        ]
        for lang in (b'en', b'ja'):
            for page in (b'index.html', b'broadcast.html', b'chanfilters.html', b'channels.html', b'connections.html', b'console.html',
                         b'editinfo.html', b'flags.html', b'login.html', b'logout.html', b'notifications.html', b'relays.html',
                         b'rtmp.html', b'settings.html', b'speedtest.html', b'viewlog.html', b'head.html?id=' + cid,
                         b'relayinfo.html?id=' + cid, b'connections.html?id=' + cid, b'editinfo.html?id=' + cid,
                         b'play.html?id=' + cid, b'play.html', b'relayinfo.html', b'index.html?fragment=channels'):
                reqs.append(('page ' + (lang + b'/' + page.replace(cid, b'ID')).decode(), get(b'/html/' + lang + b'/' + page)))
        return reqs


def run(reqs, cids):
    if True:
        exact = masked = 0
        diffs = []
        for i, (name, _) in enumerate(reqs['cxx']):
            out = {}
            for k, p in PORTS.items():
                r = reqs[k][i][1](p)
                c = cids[k]
                r = r.replace(c, b'CHANNELID')
                uuid = b'-'.join([c[0:8], c[8:12], c[12:16], c[16:20], c[20:32]]).lower()
                r = r.replace(uuid, b'CHANNELUUID').replace(c.lower(), b'channelid').replace(c[:7], b'CHANPFX')
                r = r.replace(b'serverdiff/' + k.encode(), b'serverdiff/X')
                r = re.sub(rb'auth=[0-9a-f]{32}', b'auth=TOKEN', r)
                r = re.sub(rb'[0-9A-F]{32}', b'HEXID', r)
                out[k] = normalize(r, p, [])
            if out['cxx'] == out['rs']:
                exact += 1
            elif mask_numbers(out['cxx']) == mask_numbers(out['rs']):
                masked += 1
            else:
                diffs.append((name, out))
        print('requests: %d, same: %d, same except numbers: %d, different: %d' % (len(reqs['cxx']), exact, masked, len(diffs)))
        os.makedirs(rt.WORK + '/serverdiff/out', exist_ok=True)
        for name, out in diffs:
            fn = re.sub(r'[^A-Za-z0-9._-]', '_', name)
            for k in out:
                open(rt.WORK + '/serverdiff/out/%s.%s' % (fn, k), 'wb').write(out[k])
            a, b = out['cxx'], out['rs']
            i = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
            print('DIFF %-40s at %d: cxx=%r rs=%r' % (name, i, a[max(0, i - 40):i + 60], b[max(0, i - 40):i + 60]))


if __name__ == '__main__':
    main()
