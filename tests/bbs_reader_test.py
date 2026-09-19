# ui/cgi-bin/bbs_reader.py の入力検証と SSRF 対策のテスト。
#
#   python3 -m unittest tests/bbs_reader_test.py
#
# ネットワークには接続しない (IP リテラルだけを使う)。

import os, sys, unittest, urllib.error

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ui", "cgi-bin"))
import bbs_reader


class CheckParams(unittest.TestCase):
  def test_valid(self):
    self.assertIsNone(bbs_reader.check_params("5ch.net", "news4vip"))
    self.assertIsNone(bbs_reader.check_params("jbbs.shitaraba.net", "game", "12345", "1234567890"))
    self.assertIsNone(bbs_reader.check_params("example.com:8080", "a_b-c.d"))

  def test_bad_fqdn(self):
    for fqdn in ["", "127.0.0.1:7144/admin?cmd=shutdown#", "user@evil.example",
                 "evil.example/", "evil.example?x", "evil.example#", "a b",
                 "[::1]", "-a.example", "a.example-", "a.example:99999999",
                 "a.example\r\nX: y"]:
      self.assertEqual("bad fqdn", bbs_reader.check_params(fqdn, "news"), fqdn)

  def test_bad_category(self):
    for category in ["", ".", "..", "a/b", "a?b", "a#b", "a b", "../../x", "x" * 65]:
      self.assertEqual("bad category", bbs_reader.check_params("5ch.net", category), category)

  def test_bad_board_num_and_id(self):
    self.assertEqual("bad board_num", bbs_reader.check_params("5ch.net", "x", "1/2"))
    self.assertEqual("bad board_num", bbs_reader.check_params("5ch.net", "x", "abc"))
    self.assertEqual("bad id", bbs_reader.check_params("5ch.net", "x", "", "12/34"))
    self.assertEqual("bad id", bbs_reader.check_params("5ch.net", "x", "", ""))

  def test_board_rejects_bad_params(self):
    with self.assertRaises(ValueError):
      bbs_reader.Board("127.0.0.1:7144/admin?cmd=shutdown#", "x", "")


class SafeUrlopen(unittest.TestCase):
  def assert_blocked(self, url):
    with self.assertRaises(urllib.error.URLError):
      bbs_reader.safe_urlopen(url)

  def test_loopback_and_private_addresses_blocked(self):
    for url in ["http://127.0.0.1/", "http://127.0.0.1:7144/admin?cmd=shutdown",
                "http://localhost/", "http://10.0.0.1/", "http://192.168.1.1/",
                "http://172.16.0.1/", "http://169.254.169.254/latest/meta-data/",
                "http://0.0.0.0/", "http://[::1]/", "http://[::ffff:127.0.0.1]/",
                "http://[fe80::1]/"]:
      self.assert_blocked(url)

  def test_non_http_schemes_blocked(self):
    for url in ["file:///etc/passwd", "ftp://example.com/", "gopher://example.com/"]:
      self.assert_blocked(url)

  def test_credentials_in_url_blocked(self):
    self.assert_blocked("http://user:pass@8.8.8.8/")

  def test_public_address_passes_the_check(self):
    # 接続はしない。検証だけ通ること。
    bbs_reader._check_url("http://8.8.8.8/")
    bbs_reader._check_url("https://8.8.8.8:8443/x")


if __name__ == "__main__":
  unittest.main()
