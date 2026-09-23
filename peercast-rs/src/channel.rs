//! チャンネル (core/common/channel.cpp の `Channel`) と `ChanMgr` (chanmgr.cpp) のうち、スレッド
//! やソケット、ほかの管理クラスに触らない部分。
//!
//! ICY のメタデータの解釈、トラッカーへの更新と情報の更新の atom、状態の画面に出す文字列
//! (`renderHexDump`、`getBufferString`)、読み込みの待ち時間、`authToken`、いちばん古い
//! アイドルのチャンネルの選び方。

use crate::chanhit::{self, Hit};
use crate::chaninfo::{self, Info};
use crate::gnuid;
use crate::md5;
use crate::pcp::write::AtomBuf;
use crate::pcp::*;
use crate::pcstring;
use crate::strutil;

/// `processMp3Metadata` の解釈。`StreamTitle` と `StreamUrl` の値 (引用符は付いたまま) を
/// 入力の中の位置 (始まり, 長さ) で返す。同じ名前が何度もあれば最後のもの。
///
/// `名前=値;名前=値;...` を、名前は次の `=` まで、値は次の `;` まで (なければ終わりまで)
/// として読む。名前には `;` が入りうる (C++ 版と同じ)。
pub fn mp3_metadata(s: &[u8]) -> (Option<(usize, usize)>, Option<(usize, usize)>) {
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    let (mut title, mut url) = (None, None);
    let mut cmd = 0;
    loop {
        let eq = match s[cmd..].iter().position(|&c| c == b'=') {
            Some(p) => cmd + p,
            None => break,
        };
        let arg = eq + 1;
        let (arg_end, next) = match s[arg..].iter().position(|&c| c == b';') {
            Some(p) => (arg + p, Some(arg + p + 1)),
            None => (s.len(), None),
        };
        let name = &s[cmd..eq];
        if name == b"StreamTitle" {
            title = Some((arg, arg_end - arg));
        } else if name == b"StreamUrl" {
            url = Some((arg, arg_end - arg));
        }
        match next {
            Some(n) => cmd = n,
            None => break,
        }
    }
    (title, url)
}

fn write_bcst_version(out: &mut AtomBuf) {
    out.int(PCP_BCST_VERSION, PCP_CLIENT_VERSION as i32);
    out.int(PCP_BCST_VERSION_VP, PCP_CLIENT_VERSION_VP as i32);
    out.bytes(PCP_BCST_VERSION_EX_PREFIX, PCP_CLIENT_VERSION_EX_PREFIX);
    out.short(PCP_BCST_VERSION_EX_NUMBER, PCP_CLIENT_VERSION_EX_NUMBER as i16);
}

/// `writeTrackerUpdateAtom` の atom (`hit` は C++ 側が `initLocal` で作ったもの)
pub fn tracker_update_atom(out: &mut AtomBuf, session_id: &[u8; 16], broadcast_id: &[u8; 16], info: &Info, hit: &Hit) {
    out.parent(PCP_BCST, 10);
    out.char(PCP_BCST_GROUP, PCP_BCST_GROUP_ROOT as u8);
    out.char(PCP_BCST_HOPS, 0);
    out.char(PCP_BCST_TTL, 7);
    out.bytes(PCP_BCST_FROM, session_id);
    write_bcst_version(out);
    out.parent(PCP_CHAN, 4);
    out.bytes(PCP_CHAN_ID, &info.id);
    out.bytes(PCP_CHAN_BCID, broadcast_id);
    chaninfo::write_info_atoms(out, info);
    chaninfo::write_track_atoms(out, &info.track);
    chanhit::write_atoms(out, hit, &info.id);
}

/// `updateInfo` で、配信しているチャンネルの情報が変わったときに中継先へ送る atom
pub fn info_update_atom(out: &mut AtomBuf, session_id: &[u8; 16], info: &Info) {
    out.parent(PCP_BCST, 10);
    out.char(PCP_BCST_HOPS, 0);
    out.char(PCP_BCST_TTL, 7);
    out.char(PCP_BCST_GROUP, PCP_BCST_GROUP_RELAYS as u8);
    out.bytes(PCP_BCST_FROM, session_id);
    write_bcst_version(out);
    out.bytes(PCP_BCST_CHANID, &info.id);
    out.parent(PCP_CHAN, 3);
    out.bytes(PCP_CHAN_ID, &info.id);
    chaninfo::write_info_atoms(out, info);
    chaninfo::write_track_atoms(out, &info.track);
}

/// `renderHexDump`: 16 バイトずつ、16 進数と表示できる文字を並べる
pub fn render_hex_dump(input: &[u8]) -> Vec<u8> {
    let mut res = Vec::new();
    for line in input.chunks(16) {
        let hex = strutil::hexdump(line);
        let pad = if line.len() < 16 { 47usize.saturating_sub(hex.len()) } else { 0 };
        res.extend_from_slice(&hex);
        res.extend(std::iter::repeat(b' ').take(pad));
        res.extend_from_slice(b"  ");
        res.extend_from_slice(&strutil::ascii_dump(line, b"."));
        res.push(b'\n');
    }
    res
}

/// printf の `%.2f`。NaN は CPU によらず "-nan" にする (C++ 版は x86 で "-nan"、ARM で "nan")。
fn format_f2(x: f64) -> String {
    if x.is_nan() {
        "-nan".into()
    } else if x.is_infinite() {
        if x < 0.0 { "-inf" } else { "inf" }.into()
    } else {
        format!("{:.2}", x)
    }
}

/// `getBufferString`。`byterate` は受信の平均の速さ (バイト毎秒)、`lens` はバッファにある
/// パケットの長さ。
pub fn buffer_string(byterate: f64, now: u32, last_write_time: u32, lens: &[u32], cont: i32, non_cont: i32) -> Vec<u8> {
    let last_written = now as f64 - last_write_time as f64;
    let time = if last_written < 5.0 { b"< 5 sec".to_vec() } else { pcstring::from_stopwatch(last_written as u32) };

    // std::accumulate(..., 0) は int に足していく
    let sum = lens.iter().fold(0u32, |a, &b| a.wrapping_add(b)) as i32;

    let mut out = Vec::new();
    out.extend_from_slice(b"Length: ");
    out.extend(strutil::group_digits(sum.to_string().as_bytes(), b","));
    out.extend(format!(" bytes ({} sec)\n", format_f2(sum as f64 / byterate)).into_bytes());
    out.extend(format!("Packets: {} (c {} / nc {})\n", lens.len(), cont, non_cont).into_bytes());
    if let (Some(min), Some(max)) = (lens.iter().min(), lens.iter().max()) {
        // sum / lens.size() は size_t (64 ビット) の割り算
        let avg = ((sum as i64 as u64) / lens.len() as u64) as u32 as i32;
        out.extend(format!("Packet length min/avg/max: {}/{}/{}\n", min, avg, max).into_bytes());
    }
    out.extend_from_slice(b"Last written: ");
    out.extend(time);
    out
}

/// `checkReadDelay` で眠る時間 (ミリ秒)。眠らないなら None。
///
/// C++ 版は `bitrate * 1024 / 8` が 0 になる (`bitrate * 1024` が桁あふれする) と 0 で割って
/// いた。Rust 版はそのとき眠らない。
pub fn read_delay_ms(read_delay: bool, len: u32, bitrate: i32) -> Option<u32> {
    if !read_delay || bitrate <= 0 {
        return None;
    }
    let d = bitrate.wrapping_mul(1024) / 8;
    if d == 0 {
        return None;
    }
    Some(len.wrapping_mul(1000) / d as u32)
}

/// `ChanMgr::authSecret`
pub fn auth_secret(broadcast_id: &[u8; 16], id: &[u8; 16]) -> Vec<u8> {
    [&gnuid::to_str(broadcast_id)[..], b":", &gnuid::to_str(id)[..]].concat()
}

/// `ChanMgr::authToken`
pub fn auth_token(broadcast_id: &[u8; 16], id: &[u8; 16]) -> Vec<u8> {
    md5::hexdigest(&auth_secret(broadcast_id, id))
}

/// `ChanMgr::closeOldestIdle` で止めるチャンネル。`idle` はアイドルで止められるものか
/// (動いていて、スレッドがあり、状態が S_IDLE)、`last_idle_time` はアイドルになった時刻。
/// 時刻が 0xffffffff のものは選ばない (C++ 版と同じ)。
pub fn oldest_idle(idle: &[bool], last_idle_time: &[u32]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut t = u32::MAX;
    for (i, (&ok, &lt)) in idle.iter().zip(last_idle_time).enumerate() {
        if ok && lt < t {
            best = Some(i);
            t = lt;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &[u8], p: Option<(usize, usize)>) -> Option<&[u8]> {
        p.map(|(a, n)| &s[a..a + n])
    }

    #[test]
    fn metadata() {
        let s = b"StreamTitle='a=b';StreamUrl='u';";
        let (t, u) = mp3_metadata(s);
        assert_eq!(at(s, t), Some(&b"'a=b'"[..]));
        assert_eq!(at(s, u), Some(&b"'u'"[..]));

        let s = b"x;StreamTitle='t';StreamTitle='t2'";
        let (t, u) = mp3_metadata(s);
        assert_eq!(at(s, t), Some(&b"'t2'"[..])); // 最初の名前は "x;StreamTitle"
        assert_eq!(u, None);

        let s = b"StreamTitle";
        assert_eq!(mp3_metadata(s), (None, None));
        let s = b"StreamTitle=abc\0;StreamUrl=x";
        let (t, u) = mp3_metadata(s);
        assert_eq!(at(s, t), Some(&b"abc"[..]));
        assert_eq!(u, None);
    }

    #[test]
    fn hex_dump() {
        assert_eq!(render_hex_dump(b""), b"");
        let d = render_hex_dump(b"0123456789abcdefAB\x00");
        assert_eq!(
            String::from_utf8(d).unwrap(),
            "30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66  0123456789abcdef\n\
             41 42 00                                         AB.\n"
        );
    }

    #[test]
    fn buffer() {
        let s = buffer_string(0.0, 100, 98, &[], 0, 0);
        assert_eq!(s, b"Length: 0 bytes (-nan sec)\nPackets: 0 (c 0 / nc 0)\nLast written: < 5 sec");
        let s = buffer_string(8.0, 100, 40, &[1000, 1, 1001], 1, 2);
        assert_eq!(
            String::from_utf8(s).unwrap(),
            "Length: 2,002 bytes (250.25 sec)\nPackets: 3 (c 1 / nc 2)\n\
             Packet length min/avg/max: 1/667/1001\nLast written: 1 min, 0 sec"
        );
        let s = buffer_string(0.0, 100, 40, &[5], 0, 1);
        assert!(String::from_utf8(s).unwrap().contains("(inf sec)"));
    }

    /// 乱数の入力でパニックしないこと
    #[test]
    fn fuzz_no_panic() {
        let mut rng = 0x2545f4914f6cdd1du64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..20000 {
            let n = (next() % 40) as usize;
            let s: Vec<u8> = (0..n).map(|_| b"=;'StreamTitleUrl\0x"[(next() % 19) as usize]).collect();
            let (t, u) = mp3_metadata(&s);
            for (p, l) in t.into_iter().chain(u) {
                assert!(p + l <= s.len());
            }
            render_hex_dump(&s);
            let lens: Vec<u32> = (0..(next() % 5)).map(|_| next() as u32).collect();
            let rate = f64::from_bits(next());
            buffer_string(rate, next() as u32, next() as u32, &lens, next() as i32, next() as i32);
            read_delay_ms(true, next() as u32, next() as i32);
        }
    }

    #[test]
    fn misc() {
        assert_eq!(read_delay_ms(false, 1000, 128), None);
        assert_eq!(read_delay_ms(true, 1000, 0), None);
        assert_eq!(read_delay_ms(true, 16384, 128), Some(1000));
        assert_eq!(read_delay_ms(true, 1, 1 << 22), None); // C++ 版は 0 で割る
        assert_eq!(oldest_idle(&[true, false, true, true], &[5, 1, 3, 3]), Some(2));
        assert_eq!(oldest_idle(&[true], &[u32::MAX]), None);
        assert_eq!(auth_token(&[0; 16], &[0; 16]).len(), 32);
    }
}
