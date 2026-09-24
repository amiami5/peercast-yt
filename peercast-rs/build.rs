//! ビルドの日時 (C++ 版の `__DATE__ " " __TIME__`、たとえば "Sep  4 2026 11:04:19") を
//! `PEERCAST_BUILD_DATE_TIME` として渡す。`date` コマンドがなければ渡さない ("unknown" になる)。

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let mut cmd = Command::new("date");
    cmd.env("LC_ALL", "C");
    // 再現できるビルドのために SOURCE_DATE_EPOCH があればその日時にする
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        cmd.arg("-d").arg(format!("@{}", epoch));
    }
    cmd.arg("+%b %e %Y %H:%M:%S");
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let s = s.trim_end();
                if !s.is_empty() {
                    println!("cargo:rustc-env=PEERCAST_BUILD_DATE_TIME={}", s);
                }
            }
        }
    }
}
