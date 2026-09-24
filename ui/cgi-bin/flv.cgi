#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os, re, subprocess, sys

import cgiform

if __name__ == "__main__":
  form = cgiform.FieldStorage()

  for param in ["id", "preset", "audio_codec", "type"]:
    if param not in form:
      print("Status: 400 Bad Request\n")
      sys.exit()

  id = form["id"].value
  preset = form["preset"].value
  audio_codec = form["audio_codec"].value
  server_port = os.getenv("SERVER_PORT", "")

  # このスクリプトは認証なしで (LAN 内から) 呼べるので、値を厳密に検証する。
  # id は 32 桁の 16 進数 (チャンネル ID)、preset と audio_codec は英数字と
  # '_' だけ。
  if (not re.fullmatch(r"[0-9A-Fa-f]{32}", id) or
      not re.fullmatch(r"[A-Za-z0-9_]{1,32}", preset) or
      not re.fullmatch(r"[A-Za-z0-9_]{1,32}", audio_codec) or
      not re.fullmatch(r"[0-9]{1,5}", server_port)):
    print("Status: 400 Bad Request\n")
    sys.exit()

  # PeerCast 自身へは常にループバックで接続する。Host ヘッダー由来の
  # SERVER_NAME を使うと、任意のホストから ffmpeg に取得させることができて
  # しまう (SSRF)。ffmpeg はこのスクリプトと同じマシンで動いている。
  server_name = "127.0.0.1"

  try:
    r = int(form["bitrate"].value) # チャンネルのビットレートを映像ビットレートとする。
  except (KeyError, ValueError):
    r = 0
  if r <= 0 or r > 100000:
    # 正常なビットレートが渡されなかった場合は 500Kbps にする。
    r = 500

  protocol = "http"

  print("Content-Type: video/x-flv\n", flush=True)
  subprocess.call(["ffmpeg",
    "-nostdin",
    "-v", "-8", # quiet
    "-y",       # confirm overwriting
    "-i", "{0}://{1}:{2}/stream/{3}".format(protocol, server_name, server_port, id),
    "-strict", "-2",
    "-acodec", audio_codec,
    "-async", "1",
    "-ar", "44100",
    "-vcodec", "libx264",
    "-x264-params", "bitrate={0}:vbv-maxrate={0}:vbv-bufsize={1}".format(r, 2*r),
    "-preset", preset,
    "-f", "flv",
    "-"])        # to stdout
