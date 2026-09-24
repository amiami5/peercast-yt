//! プレイリスト (core/common/playlist.cpp の `PlayList`)

use super::chaninfo::{self as ci, ChanInfo};
use super::error::Result;
use super::peercast::Peercast;
use super::pcstr;
use super::stream::{Stream, StreamExt};

pub const T_NONE: i32 = 0;
/// SHOUTcast のプレイリスト
pub const T_SCPLS: i32 = 1;
pub const T_PLS: i32 = 2;
pub const T_RAM: i32 = 3;

/// `PlayList`
pub struct PlayList {
    pub ty: i32,
    pub max_urls: usize,
    pub urls: Vec<Vec<u8>>,
    pub titles: Vec<Vec<u8>>,
}

impl PlayList {
    pub fn new(ty: i32, max: usize) -> PlayList {
        PlayList { ty, max_urls: max, urls: Vec::new(), titles: Vec::new() }
    }

    /// `addURL` (`::String::set` と同じく 255 バイトまで)
    pub fn add_url(&mut self, url: &[u8], title: &[u8]) {
        if self.urls.len() < self.max_urls {
            self.urls.push(pcstr::cut(url));
            self.titles.push(pcstr::cut(title));
        }
    }

    /// `addChannel`
    pub fn add_channel(&mut self, pc: &Peercast, path: &[u8], info: &ChanInfo) {
        let nid = if ci::is_set(&info.id) { ci::id_str(&info.id).into_bytes() } else { info.name.data.clone() };
        let url = [path, b"/stream/", &nid, &info.type_ext(), b"?auth=", &pc.chanmgr.auth_token(&info.id)].concat();
        self.add_url(&url, &info.name.data);
    }

    /// `getPlayListType`
    pub fn type_for(content_type: &[u8]) -> i32 {
        if content_type == ci::T_OGM {
            T_RAM
        } else {
            T_PLS
        }
    }

    /// `read`: 読めたところまでを残す (ソケットの終わりはうまく扱えないので、エラーは無視する)
    pub fn read(&mut self, s: &mut dyn Stream) {
        let _ = match self.ty {
            T_SCPLS => self.read_scpls(s),
            T_PLS => self.read_pls(s),
            _ => Ok(()),
        };
    }

    /// `readSCPLS`: 空の行で終わる
    fn read_scpls(&mut self, s: &mut dyn Stream) -> Result<()> {
        loop {
            let line = s.read_line_buf(256)?;
            if line.is_empty() {
                return Ok(());
            }
            if line.len() >= 4 && line[..4].eq_ignore_ascii_case(b"file") {
                if let Some(i) = line.iter().position(|&c| c == b'=') {
                    self.add_url(&line[i + 1..], b"");
                }
            }
        }
    }

    /// `readPLS`: 空の行で終わる
    fn read_pls(&mut self, s: &mut dyn Stream) -> Result<()> {
        loop {
            let line = s.read_line_buf(256)?;
            if line.is_empty() {
                return Ok(());
            }
            if line[0] != b'#' {
                self.add_url(&line, b"");
            }
        }
    }

    /// `write`
    pub fn write(&self, out: &mut dyn Stream) -> Result<()> {
        match self.ty {
            T_SCPLS => {
                out.write_line("[playlist]")?;
                out.write_line("")?;
                out.write_line(format!("NumberOfEntries={}", self.urls.len()))?;
                for (i, (u, t)) in self.urls.iter().zip(&self.titles).enumerate() {
                    out.write_line([format!("File{}=", i + 1).as_bytes(), u].concat())?;
                    out.write_line([format!("Title{}=", i + 1).as_bytes(), t].concat())?;
                    out.write_line(format!("Length{}=-1", i + 1))?;
                }
                out.write_line("Version=2")
            }
            T_PLS | T_RAM => {
                for u in &self.urls {
                    out.write_line(u)?;
                }
                Ok(())
            }
            _ => Err(super::error::Error::stream("unsupported playlist type for writing")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::stream::StringStream;

    #[test]
    fn read_and_write() {
        let mut s = StringStream::from(&b"[playlist]\r\nFile1=http://a/\nfile2=http://b/\n\nFile3=http://c/\n"[..]);
        let mut p = PlayList::new(T_SCPLS, 10);
        p.read(&mut s);
        // 最初の行 "[playlist]" は空でないので続き、空の行で止まる
        assert_eq!(p.urls, vec![b"http://a/".to_vec(), b"http://b/".to_vec()]);

        let mut s = StringStream::from(&b"#x\nhttp://a/\nhttp://b/"[..]);
        let mut p = PlayList::new(T_PLS, 1);
        p.read(&mut s);
        assert_eq!(p.urls, vec![b"http://a/".to_vec()]);

        let mut p = PlayList::new(T_SCPLS, 5);
        p.add_url(b"u", b"t");
        let mut out = StringStream::new();
        p.write(&mut out).unwrap();
        assert_eq!(out.str(), &b"[playlist]\r\n\r\nNumberOfEntries=1\r\nFile1=u\r\nTitle1=t\r\nLength1=-1\r\nVersion=2\r\n"[..]);
    }
}
