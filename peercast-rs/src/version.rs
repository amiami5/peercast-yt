//! バージョン (C++ 版の core/common/version2.h)。バージョンを上げるときはこのファイルだけを直す。
//!
//! 文字列の `0.1218` と `YT50` は、下の数と同じにしておく (テストで確かめる)。
//! `-rs2` は Rust 版の版で、PCP で送る数には入らない。

/// `PCX_AGENT`: HTTP の Server・User-Agent と、PCP の helo の agnt
pub const PCX_AGENT: &str = "PeerCast/0.1218 (YT50-rs2)";
/// `PCX_VERSTRING`: `peercast --version` などに出す
pub const PCX_VERSTRING: &str = "v0.1218 YT50-rs2";

/// `PCP_CLIENT_VERSION`
pub const PCP_CLIENT_VERSION: u32 = 1218;
/// `PCP_CLIENT_VERSION_VP`
pub const PCP_CLIENT_VERSION_VP: u32 = 27;
/// `PCP_CLIENT_VERSION_EX_PREFIX`
pub const PCP_CLIENT_VERSION_EX_PREFIX: &[u8; 2] = b"YT";
/// `PCP_CLIENT_VERSION_EX_NUMBER`
pub const PCP_CLIENT_VERSION_EX_NUMBER: u32 = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_match_numbers() {
        let ver = format!("0.{}", PCP_CLIENT_VERSION);
        let ex = format!("{}{}", std::str::from_utf8(PCP_CLIENT_VERSION_EX_PREFIX).unwrap(), PCP_CLIENT_VERSION_EX_NUMBER);
        assert!(PCX_AGENT.starts_with(&format!("PeerCast/{} ({}-", ver, ex)), "{}", PCX_AGENT);
        assert!(PCX_VERSTRING.starts_with(&format!("v{} {}-", ver, ex)), "{}", PCX_VERSTRING);
    }
}
