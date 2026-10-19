//! Renders the coloured logos at compile time. The art never changes between runs, so
//! macho copies it out of the binary instead of dithering it on every start. The
//! renderer is the real one, compiled into this script from `src/`.
//!
//! It also passes the link flags for macho's own runtime (see `src/rt.rs`) here rather
//! than in `.cargo/config.toml`, which `cargo install` ignores.

extern crate alloc;

use std::fmt::Write;
use std::path::PathBuf;

#[allow(dead_code)]
#[path = "src/term.rs"]
mod term;

#[allow(dead_code)]
#[path = "src/logo.rs"]
mod logo;

fn main() {
    let mut src = String::from("&[\n");
    for l in logo::LOGOS {
        let art = l.render(true);
        let text = (0..art.lines.len()).filter_map(|i| art.lines.line(i)).collect::<Vec<_>>().join("\n");
        let _ = writeln!(src, "    ({:?}, {text:?}, {}),", l.name, art.width);
    }
    src.push_str("]\n");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("art.rs");
    std::fs::write(out, src).expect("OUT_DIR is writable");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        for arg in ["-nostartfiles", "-nostdlib", "-static"] {
            println!("cargo::rustc-link-arg-bins={arg}");
        }
    }
    for path in ["build.rs", "logos", "src/logo.rs", "src/term.rs"] {
        println!("cargo::rerun-if-changed={path}");
    }
}
