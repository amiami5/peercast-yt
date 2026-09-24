# -*- coding: utf-8 -*-
import configparser, re, urllib.request, urllib.error, urllib.parse, html
import ipaddress, socket

def print_bad_request(message):
  print("Status: 400 Bad Request")
  print("Content-Type: text/plain")
  print("")
  print(message)

# ---------------------------------------------------------------------
# 入力の検証と SSRF 対策
#
# fqdn, category, board_num, thread_id は利用者が指定する値で、そのまま URL に
# 埋め込まれる。"127.0.0.1:7144/admin?cmd=shutdown#" のような値を渡されると、
# PeerCast 自身の管理画面 (localhost は認証不要) や、内部ネットワーク、クラウド
# のメタデータサーバー (169.254.169.254) などにこのスクリプトから要求が送ら
# れてしまう。文字種を制限し、接続先が公開アドレスであることを確認する。

_HOST_RE = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?(?::[0-9]{1,5})?$")
_NAME_RE = re.compile(r"^[A-Za-z0-9_.-]{1,64}$")
_NUM_RE  = re.compile(r"^[0-9]{1,20}$")

MAX_DOWNLOAD_SIZE = 16 * 1024 * 1024
DOWNLOAD_TIMEOUT = 15

def check_params(fqdn, category, board_num = "", thread_id = None):
  """不正なら理由 (文字列) を、問題なければ None を返す。"""
  if not _HOST_RE.match(fqdn):
    return "bad fqdn"
  if not _NAME_RE.match(category) or category in (".", ".."):
    return "bad category"
  if board_num != "" and not _NUM_RE.match(board_num):
    return "bad board_num"
  if thread_id is not None and not _NUM_RE.match(thread_id):
    return "bad id"
  return None

def _check_public_host(hostname, port):
  try:
    infos = socket.getaddrinfo(hostname, port, proto = socket.IPPROTO_TCP)
  except socket.gaierror:
    raise urllib.error.URLError("cannot resolve host")
  if not infos:
    raise urllib.error.URLError("cannot resolve host")
  for info in infos:
    ip = ipaddress.ip_address(info[4][0].split("%")[0])
    if getattr(ip, "ipv4_mapped", None):
      ip = ip.ipv4_mapped
    if not ip.is_global:
      raise urllib.error.URLError("access to non-public address is not allowed")

def _check_url(url):
  parts = urllib.parse.urlsplit(url)
  if parts.scheme not in ("http", "https"):
    raise urllib.error.URLError("unsupported scheme")
  if parts.hostname is None or parts.username is not None:
    raise urllib.error.URLError("bad url")
  port = parts.port or (443 if parts.scheme == "https" else 80)
  _check_public_host(parts.hostname, port)

class _SafeRedirectHandler(urllib.request.HTTPRedirectHandler):
  # リダイレクト先が内部アドレスや http/https 以外でないことを確認する。
  def redirect_request(self, req, fp, code, msg, headers, newurl):
    _check_url(urllib.parse.urljoin(req.full_url, newurl))
    return super().redirect_request(req, fp, code, msg, headers, newurl)

def safe_urlopen(url_or_request, data = None):
  """urllib.request.urlopen の代わり。公開アドレス以外への接続を拒否する。"""
  url = url_or_request if isinstance(url_or_request, str) else url_or_request.full_url
  _check_url(url)
  opener = urllib.request.build_opener(_SafeRedirectHandler)
  return opener.open(url_or_request, data, timeout = DOWNLOAD_TIMEOUT)

class Board:

  def __init__(self, fqdn, category, board_num):
    error = check_params(fqdn, category, board_num)
    if error is not None:
      raise ValueError(error)
    self.fqdn = fqdn
    self.shitaraba = "jbbs.shitaraba.net" in fqdn
    self.category = category
    self.board_num = board_num
    self.resmax = 1000
    self.urlpath = category + "/" + board_num if len(board_num) != 0 else category

    self.__settings_url = ("http://{0}/bbs/api/setting.cgi/{1}" if self.shitaraba else "http://{0}/{1}/SETTING.TXT").format(self.fqdn, self.urlpath)
    self.__thread_list_url = "http://{0}/{1}/subject.txt".format(self.fqdn, self.urlpath)
    self.external_encoding = "EUC-JP" if self.shitaraba else "CP932"

  def dat_url(self, thread_num):
    return ("http://{0}/bbs/rawmode.cgi/{1}/{2}/" if self.shitaraba else "http://{0}/{1}/dat/{2}.dat").format(self.fqdn, self.urlpath, thread_num)

  def settings(self):
    try:
      str = self.download(self.__settings_url)
    except urllib.error.HTTPError:
      config = configparser.ConfigParser()
      config.read_string("[DEFAULT]\n" + "ERROR = [Settings download error]\n")
      return config.defaults()
    try:
      str = str.decode(self.external_encoding)
    except:
      str = str.decode("UTF-8")
    return self.__parse_settings(str)

  def thread_list(self):
    try:
      str = self.download(self.__thread_list_url)
    except urllib.error.HTTPError:
      # したらばでスレッドがまだないときはsubject.txtが404になる。
      str = b""
    return str.decode(self.external_encoding)

  def thread(self, thread_num):
    return next((t for t in self.threads() if t.id == thread_num), None)

  def threads(self):
    threads = []
    lines = self.thread_list().splitlines()
    p = re.compile(r"^(\d+)\.cgi,(.+?)\((\d+)\)$" if self.shitaraba else r"^(\d+)\.dat<>(.+?)\s\((\d+)\)$")
    for i, line in enumerate(lines):
      if line == "": # したらばでスレッドがない場合、空行だけになる。
        continue
      m = p.match(line)
      self.resmax = max(self.resmax, int(m.group(3)))
      threads.append(Thread(self, m.group(1), html.unescape(m.group(2)), m.group(3)))
    if self.shitaraba and len(threads) > 1:
      threads.pop(len(threads) - 1)
    return threads

  def download(self, url):
    response = safe_urlopen(url)
    data = response.read(MAX_DOWNLOAD_SIZE + 1)
    if len(data) > MAX_DOWNLOAD_SIZE:
      raise urllib.error.URLError("response too large")
    return data

  def __parse_settings(self, string):
    config = configparser.ConfigParser()
    config.read_string("[DEFAULT]\n" + string)
    return config.defaults()

class Post:

  @classmethod
  def from_line(cls, line, shitaraba):
    if shitaraba:
      lines = line.split('<>', 6)
      return cls(lines[0], lines[1], lines[2], lines[3], lines[4])
    else:
      lines = line.split('<>', 4)
      return cls(0, lines[0], lines[1], lines[2], lines[3])

  def __init__(self, no, name, mail, date, body):
    self.no = int(no)
    self.name = name
    self.mail = mail
    self.date = date
    self.body = body

class Thread:

  def __init__(self, board, id, title, last = 1):
    self.board = board
    self.id = id
    self.title = title
    self.last = int(last)

  def dat_url(self):
    return self.board.dat_url(self.id)

  def posts(self, r):
    lines = self.dat_for_range(r).splitlines()
    for i, line in enumerate(lines):
      lines[i] = Post.from_line(line, self.board.shitaraba)
      if lines[i].no == 0:
        lines[i].no = i+r.start
      self.last = max(lines[i].no, self.last)
    return lines

  def dat_for_range(self, r):
    if self.board.shitaraba:
      if r.stop >= self.board.resmax:
        query = "{0}-".format(r.start)
      else:
        query = "{0}-{1}".format(r.start, r.stop)
      url = self.dat_url() + query
      return self.board.download(url).decode(self.board.external_encoding, 'replace')
    else:
      url = self.dat_url()
      lines = self.board.download(url).decode(self.board.external_encoding, 'replace').splitlines()
      return "".join(map(lambda line: line + "\n", lines[r.start-1:]))
