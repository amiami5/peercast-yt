//! 管理画面のコンソールのコマンド (core/common/commands.cpp) の、引数を解釈する部分。

use std::collections::BTreeMap;

use crate::strutil::split_limit;

fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `parse_options`: `--名前[=値]` と `-名前` のオプションと、それ以外 (位置引数) に分ける。
/// `--` より後ろは全部位置引数。知らないオプションはエラー (`FormatException` のメッセージ)。
#[allow(clippy::type_complexity)]
pub fn parse_options(args: &[Vec<u8>], names: &[Vec<u8>]) -> Result<(BTreeMap<Vec<u8>, Vec<u8>>, Vec<Vec<u8>>), Vec<u8>> {
    let mut options = BTreeMap::new();
    let mut positionals = Vec::new();
    let known = |n: &[u8]| names.iter().any(|x| x.as_slice() == n);

    for (i, arg) in args.iter().enumerate() {
        if arg.as_slice() == b"--" {
            positionals.extend(args[i + 1..].iter().cloned());
            break;
        }
        // C++ 版は std::string の [] で読むので、長さを超えたところは '\0'
        if arg.first() != Some(&b'-') {
            positionals.push(arg.clone());
        } else if arg.get(1) == Some(&b'-') {
            // 長いオプション
            let mut v = split_limit(arg, b"=", 2).unwrap();
            if !known(&v[0]) {
                return Err([&b"Unknown long option: "[..], until_nul(&v[0])].concat());
            }
            let value = if v.len() == 2 { v.pop().unwrap() } else { Vec::new() };
            options.insert(v.swap_remove(0), value);
        } else {
            // 短いオプション
            if !known(arg) {
                return Err([&b"Unknown short option: "[..], until_nul(arg)].concat());
            }
            options.insert(arg.clone(), Vec::new());
        }
    }
    Ok((options, positionals))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<Vec<u8>> {
        s.iter().map(|x| x.as_bytes().to_vec()).collect()
    }

    #[test]
    fn options() {
        let names = v(&["--help", "-n", "--count"]);
        let (o, p) = parse_options(&v(&["a", "--count=3", "-n", "--help", "b", "--", "-x", "--y"]), &names).unwrap();
        assert_eq!(o.get(&b"--count"[..].to_vec()), Some(&b"3".to_vec()));
        assert_eq!(o.get(&b"-n"[..].to_vec()), Some(&vec![]));
        assert_eq!(o.get(&b"--help"[..].to_vec()), Some(&vec![]));
        assert_eq!(p, v(&["a", "b", "-x", "--y"]));
        assert_eq!(parse_options(&v(&["--nope=1"]), &names).unwrap_err(), b"Unknown long option: --nope");
        assert_eq!(parse_options(&v(&["-z"]), &names).unwrap_err(), b"Unknown short option: -z");
        assert_eq!(parse_options(&v(&["-"]), &names).unwrap_err(), b"Unknown short option: -");
        assert_eq!(parse_options(&v(&[""]), &names).unwrap().1, v(&[""]));
        // 値の中の '=' はそのまま
        let (o, _) = parse_options(&v(&["--count=a=b"]), &names).unwrap();
        assert_eq!(o.get(&b"--count"[..].to_vec()), Some(&b"a=b".to_vec()));
    }
}
