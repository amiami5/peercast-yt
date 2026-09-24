//! OS の機能のうち、Rust の標準ライブラリにないもの (地方時、シグナル、デーモン化、umask、逆引き)。
//! C のライブラリの関数を直接呼ぶので、このモジュールだけ `unsafe` を使う。
//!
//! 構造体の配置が CPU や OS で違うものは使わない (`struct tm` は glibc の配置で、`long` は
//! `c_long`)。Unix 以外では、使えない機能は何もしない。

use std::os::raw::{c_char, c_int, c_long};

/// `struct tm` の欄
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub mday: i32,
    pub mon: i32,
    pub year: i32,
    pub wday: i32,
    pub yday: i32,
    pub isdst: i32,
    /// UTC からのずれ (秒)
    pub gmtoff: i64,
}

#[cfg(unix)]
mod imp {
    use super::*;

    #[repr(C)]
    struct CTm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }

    extern "C" {
        fn localtime_r(t: *const c_long, out: *mut CTm) -> *mut CTm;
        fn signal(sig: c_int, handler: usize) -> usize;
        fn daemon(nochdir: c_int, noclose: c_int) -> c_int;
        fn umask(mask: u32) -> u32;
        fn gethostname(name: *mut c_char, len: usize) -> c_int;
        fn tzset();
        fn getuid() -> u32;
    }

    /// `struct passwd` (glibc と musl の Linux での並び。BSD や macOS は欄が違うので使わない)
    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct Passwd {
        pw_name: *mut c_char,
        pw_passwd: *mut c_char,
        pw_uid: u32,
        pw_gid: u32,
        pw_gecos: *mut c_char,
        pw_dir: *mut c_char,
        pw_shell: *mut c_char,
    }

    #[cfg(target_os = "linux")]
    extern "C" {
        fn getpwuid_r(uid: u32, pwd: *mut Passwd, buf: *mut c_char, buflen: usize, result: *mut *mut Passwd) -> c_int;
    }

    /// `getpwuid(getuid())->pw_dir`: パスワードのデータベースにある自分のホームディレクトリ
    #[cfg(target_os = "linux")]
    pub fn home_dir_of_user() -> Option<Vec<u8>> {
        let mut pwd = Passwd {
            pw_name: std::ptr::null_mut(),
            pw_passwd: std::ptr::null_mut(),
            pw_uid: 0,
            pw_gid: 0,
            pw_gecos: std::ptr::null_mut(),
            pw_dir: std::ptr::null_mut(),
            pw_shell: std::ptr::null_mut(),
        };
        let mut buf = vec![0 as c_char; 16384];
        let mut result: *mut Passwd = std::ptr::null_mut();
        // SAFETY: pwd と buf はこの関数の中で生きていて、buf の大きさを渡す。結果の文字列は buf の中を指す。
        let r = unsafe { getpwuid_r(getuid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
        if r != 0 || result.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        // SAFETY: pw_dir は buf の中の NUL で終わる文字列
        let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) };
        Some(dir.to_bytes().to_vec())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn home_dir_of_user() -> Option<Vec<u8>> {
        None
    }

    pub fn localtime(t: i64) -> Option<Tm> {
        let t = t as c_long;
        let mut tm = CTm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        };
        // SAFETY: 引数はどちらもこの関数の中の有効な値を指す。localtime_r はスレッド安全。
        let r = unsafe {
            tzset();
            localtime_r(&t, &mut tm)
        };
        if r.is_null() {
            return None;
        }
        Some(Tm {
            sec: tm.tm_sec,
            min: tm.tm_min,
            hour: tm.tm_hour,
            mday: tm.tm_mday,
            mon: tm.tm_mon,
            year: tm.tm_year,
            wday: tm.tm_wday,
            yday: tm.tm_yday,
            isdst: tm.tm_isdst,
            gmtoff: tm.tm_gmtoff as i64,
        })
    }

    pub const SIGHUP: c_int = 1;
    pub const SIGINT: c_int = 2;
    pub const SIGTERM: c_int = 15;
    const SIG_DFL: usize = 0;
    const SIG_IGN: usize = 1;

    /// シグナルを受けたら `handler` を呼ぶ (`handler` はシグナルハンドラーの中で呼ばれるので、
    /// アトミック変数を書くことしかしてはいけない)。
    pub fn set_signal(sig: c_int, handler: extern "C" fn(c_int)) {
        // SAFETY: handler は extern "C" の関数で、プログラムの終わりまで有効。
        unsafe {
            signal(sig, handler as usize);
        }
    }

    pub fn default_signal(sig: c_int) {
        // SAFETY: SIG_DFL はどのシグナルにも設定できる。
        unsafe {
            signal(sig, SIG_DFL);
        }
    }

    pub fn ignore_signal(sig: c_int) {
        // SAFETY: SIG_IGN はどのシグナルにも設定できる。
        unsafe {
            signal(sig, SIG_IGN);
        }
    }

    /// `daemon(1, 0)`
    pub fn daemonize() -> bool {
        // SAFETY: 引数は値だけ。
        unsafe { daemon(1, 0) == 0 }
    }

    pub fn set_umask(mask: u32) {
        // SAFETY: 引数は値だけ。
        unsafe {
            umask(mask);
        }
    }

    pub fn hostname() -> Option<Vec<u8>> {
        let mut buf = [0u8; 256];
        // SAFETY: buf は 256 バイト書ける。
        let r = unsafe { gethostname(buf.as_mut_ptr() as *mut c_char, buf.len()) };
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(buf[..n].to_vec())
    }
}

#[cfg(target_os = "linux")]
mod rdns {
    use std::os::raw::{c_char, c_int};

    #[repr(C)]
    struct SockaddrIn6 {
        sin6_family: u16,
        sin6_port: u16,
        sin6_flowinfo: u32,
        sin6_addr: [u8; 16],
        sin6_scope_id: u32,
    }

    #[repr(C)]
    struct SockaddrIn {
        sin_family: u16,
        sin_port: u16,
        sin_addr: [u8; 4],
        sin_zero: [u8; 8],
    }

    const AF_INET: u16 = 2;
    const AF_INET6: u16 = 10;
    const NI_NAMEREQD: c_int = 8;
    const NI_MAXHOST: usize = 1025;

    extern "C" {
        fn getnameinfo(
            sa: *const u8,
            salen: u32,
            host: *mut c_char,
            hostlen: u32,
            serv: *mut c_char,
            servlen: u32,
            flags: c_int,
        ) -> c_int;
    }

    /// `USys::getHostnameByAddress` (`getnameinfo` の `NI_NAMEREQD`)
    pub fn reverse_lookup(ip: &[u8; 16]) -> Option<Vec<u8>> {
        let mut buf = [0u8; NI_MAXHOST];
        let mapped = ip[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
        let r = if mapped {
            let sa = SockaddrIn { sin_family: AF_INET, sin_port: 0, sin_addr: [ip[12], ip[13], ip[14], ip[15]], sin_zero: [0; 8] };
            // SAFETY: sa と buf はこの関数の中で有効で、長さは正しい。
            unsafe {
                getnameinfo(
                    &sa as *const SockaddrIn as *const u8,
                    std::mem::size_of::<SockaddrIn>() as u32,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    NI_NAMEREQD,
                )
            }
        } else {
            let sa = SockaddrIn6 { sin6_family: AF_INET6, sin6_port: 0, sin6_flowinfo: 0, sin6_addr: *ip, sin6_scope_id: 0 };
            // SAFETY: 同上
            unsafe {
                getnameinfo(
                    &sa as *const SockaddrIn6 as *const u8,
                    std::mem::size_of::<SockaddrIn6>() as u32,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    NI_NAMEREQD,
                )
            }
        };
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(buf[..n].to_vec())
    }
}

#[cfg(not(target_os = "linux"))]
mod rdns {
    pub fn reverse_lookup(_ip: &[u8; 16]) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;
    pub fn localtime(t: i64) -> Option<Tm> {
        // 地方時がわからないので UTC
        Some(gmtime(t))
    }
    pub const SIGHUP: c_int = 1;
    pub const SIGINT: c_int = 2;
    pub const SIGTERM: c_int = 15;
    pub fn set_signal(_sig: c_int, _handler: extern "C" fn(c_int)) {}
    pub fn default_signal(_sig: c_int) {}
    pub fn ignore_signal(_sig: c_int) {}
    pub fn daemonize() -> bool {
        false
    }
    pub fn set_umask(_mask: u32) {}
    pub fn hostname() -> Option<Vec<u8>> {
        std::env::var("COMPUTERNAME").ok().map(|s| s.into_bytes())
    }
    pub fn home_dir_of_user() -> Option<Vec<u8>> {
        None
    }
    #[allow(dead_code)]
    fn _unused(_: c_long, _: *const c_char) {}
}

pub use imp::*;
pub use rdns::reverse_lookup;

/// UTC の日時 (`gmtime`)
pub fn gmtime(t: i64) -> Tm {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // 1970-01-01 は木曜日
    let wday = (days + 4).rem_euclid(7);
    // civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    const CUM: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = CUM[(m - 1) as usize] + d - 1 + if leap && m > 2 { 1 } else { 0 };
    Tm {
        sec: (secs % 60) as i32,
        min: ((secs / 60) % 60) as i32,
        hour: (secs / 3600) as i32,
        mday: d as i32,
        mon: (m - 1) as i32,
        year: (y - 1900) as i32,
        wday: wday as i32,
        yday: yday as i32,
        isdst: 0,
        gmtoff: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc() {
        let t = gmtime(784111777); // 1994-11-06 08:49:37 (日曜日)
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.min, t.sec, t.wday), (94, 10, 6, 8, 49, 37, 0));
        let t = gmtime(0);
        assert_eq!((t.year, t.mon, t.mday, t.wday, t.yday), (70, 0, 1, 4, 0));
        let t = gmtime(951782400); // 2000-02-29
        assert_eq!((t.year, t.mon, t.mday, t.yday), (100, 1, 29, 59));
    }

    #[cfg(unix)]
    #[test]
    fn local() {
        let t = localtime(784111777).unwrap();
        let u = gmtime(784111777 + t.gmtoff);
        assert_eq!((t.hour, t.min, t.mday), (u.hour, u.min, u.mday));
    }
}
