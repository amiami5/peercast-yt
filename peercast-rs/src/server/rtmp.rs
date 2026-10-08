//! 入力元の rtmp:// (core/common/rtmp.cpp の `RTMPClientStream`)。librtmp を C ABI で呼ぶ。

use std::os::raw::{c_char, c_int, c_void};

use super::error::{Error, Result};
use super::stream::{Stat, Stream};

#[link(name = "rtmp")]
extern "C" {
    fn RTMP_Alloc() -> *mut c_void;
    fn RTMP_Init(r: *mut c_void);
    fn RTMP_SetupURL(r: *mut c_void, url: *mut c_char) -> c_int;
    fn RTMP_Connect(r: *mut c_void, cp: *mut c_void) -> c_int;
    fn RTMP_Read(r: *mut c_void, buf: *mut c_char, size: c_int) -> c_int;
    fn RTMP_Close(r: *mut c_void);
    fn RTMP_Free(r: *mut c_void);
}

/// `RTMPClientStream`
pub struct RtmpStream {
    r: *mut c_void,
    /// librtmp は URL の文字列を指したまま使う (空白などに NUL を書き込みもする) ので、
    /// NUL で終わるバイト列として閉じるまで持っておく
    url: Vec<u8>,
    eof: bool,
    stat: Stat,
}

// librtmp のセッションは、持っているスレッドだけが使う
unsafe impl Send for RtmpStream {}

impl RtmpStream {
    /// `open`
    pub fn open(url: &[u8]) -> Result<RtmpStream> {
        if url.contains(&0) {
            return Err(Error::stream("RTMP_SetupURL"));
        }
        let url = [url, &[0]].concat();
        // SAFETY: RTMP_Alloc が返したものを RTMP_Init で初期化し、Drop で RTMP_Close と RTMP_Free をする
        let r = unsafe { RTMP_Alloc() };
        if r.is_null() {
            return Err(Error::stream("RTMP_Alloc"));
        }
        unsafe { RTMP_Init(r) };
        let mut s = RtmpStream { r, url, eof: false, stat: Stat::default() };
        // SAFETY: URL の文字列は s.url が持ち、s より長く生きる (s を動かしても Vec の中身は動かない)。
        // librtmp は長さを変えずに中へ NUL を書き込むので、書き込めるポインタを渡す
        if unsafe { RTMP_SetupURL(s.r, s.url.as_mut_ptr() as *mut c_char) } == 0 {
            return Err(Error::stream("RTMP_SetupURL"));
        }
        if unsafe { RTMP_Connect(s.r, std::ptr::null_mut()) } == 0 {
            return Err(Error::stream("RTMP_Connect"));
        }
        Ok(s)
    }
}

impl Stream for RtmpStream {
    /// バッファーが埋まるまで読む
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < buf.len() {
            let want = (buf.len() - done).min(c_int::MAX as usize) as c_int;
            // SAFETY: buf[done..] の want バイトに書く
            let n = unsafe { RTMP_Read(self.r, buf[done..].as_mut_ptr() as *mut c_char, want) };
            if n == 0 {
                self.eof = true;
                return Err(Error::stream("End of stream"));
            }
            if n < 0 {
                // C++ 版は -1 を読んだ量として扱っていた (docs/cpp-known-issues.md)
                self.eof = true;
                return Err(Error::stream("RTMP_Read"));
            }
            self.stat.update(n as u32, 0);
            done += n as usize;
        }
        Ok(done)
    }

    fn write(&mut self, _data: &[u8]) -> Result<()> {
        Err(Error::stream("Stream can`t write"))
    }

    fn eof(&mut self) -> Result<bool> {
        Ok(self.eof)
    }

    fn stat(&self) -> Option<&Stat> {
        Some(&self.stat)
    }
}

impl Drop for RtmpStream {
    fn drop(&mut self) {
        // SAFETY: open で作ったもの。ここでだけ解放する
        unsafe {
            RTMP_Close(self.r);
            RTMP_Free(self.r);
        }
    }
}
