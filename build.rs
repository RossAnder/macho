//! Draws every logo at compile time, coloured and plain. The art never changes between
//! runs, so macho copies it out of the binary instead of drawing it on every start. The
//! drawing code is the real one, compiled into this script from `src/`.
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

#[path = "src/draw.rs"]
mod draw;

fn lines(art: &draw::Art) -> Vec<&str> {
    (0..art.lines.len()).filter_map(|i| art.lines.line(i)).collect()
}

fn main() {
    let mut src = String::from("&[\n");
    for l in logo::LOGOS {
        let [color, plain] = [true, false].map(|color| l.render(color));
        let _ =
            writeln!(src, "    ({:?}, &{:?}, &{:?}, {}),", l.name, lines(&color), lines(&plain), color.width);
    }
    src.push_str("]\n");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("art.rs");
    std::fs::write(out, src).expect("OUT_DIR is writable");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // rustc links musl's own crt1.o into musl targets, and its `_start` clashes with ours.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
            println!(
                "cargo::error=macho needs no libc, so build it for the gnu target (drop `--target {}`); the binary is static either way",
                std::env::var("TARGET").unwrap_or_default()
            );
        }
        for arg in ["-nostartfiles", "-nostdlib", "-static"] {
            println!("cargo::rustc-link-arg-bins={arg}");
        }
    }
    for path in ["build.rs", "logos", "src/draw.rs", "src/logo.rs", "src/term.rs"] {
        println!("cargo::rerun-if-changed={path}");
    }
}
