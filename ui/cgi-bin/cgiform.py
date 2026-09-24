# -*- coding: utf-8 -*-
# cgi.FieldStorage の代わり (標準ライブラリの cgi モジュールは Python 3.13 で削除された)。
#
# PeerCast は CGI スクリプトを GET でしか呼ばない (REQUEST_METHOD=GET) ので、QUERY_STRING だけを読む。
# cgi.FieldStorage と同じく、値が空の引数は無いものとして扱い、'+' は空白、%XX は UTF-8 として
# (不正なバイトは置換文字にして) 読む。同じ名前が複数あるときは最初の値を使う
# (cgi.FieldStorage はリストを返し、.value で例外になっていた)。

import os
import urllib.parse


class Field:
  def __init__(self, value):
    self.value = value


class FieldStorage:
  def __init__(self, query_string=None):
    if query_string is None:
      query_string = os.environ.get("QUERY_STRING", "")
    self._fields = {}
    for key, value in urllib.parse.parse_qsl(query_string, keep_blank_values=False,
                                             encoding="utf-8", errors="replace"):
      if key not in self._fields:
        self._fields[key] = Field(value)

  def __contains__(self, key):
    return key in self._fields

  def __getitem__(self, key):
    return self._fields[key]

  def getvalue(self, key, default=None):
    f = self._fields.get(key)
    return f.value if f is not None else default
