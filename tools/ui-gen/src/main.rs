//! `peercast-ui-gen UI_DIR OUT_DIR`: ブラウザ UI (html/、public/) を作る。中身は lib.rs。

use std::path::Path;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("Usage: peercast-ui-gen UI_DIR OUT_DIR");
        exit(2);
    }
    if let Err(e) = peercast_ui_gen::run(Path::new(&args[0]), Path::new(&args[1])) {
        eprintln!("peercast-ui-gen: {}", e);
        exit(1);
    }
}
