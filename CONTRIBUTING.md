# Contributing

Issues and pull requests are welcome. macho stays small on purpose: no runtime
dependencies, no child processes, and one line per fact. A new field earns its place
by being cheap to read and useful at a glance.

## Checks

CI runs these on every push and pull request, so run them before you open one:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` runs on the host's architecture. The binary has assembly for x86-64 and
aarch64 (`src/rt.rs`, `src/sys.rs`), so a change there also needs a build for the
other one; CI builds, tests and runs both.

## How it fits together

The code lives in the library (`src/lib.rs`), which is `no_std` so the binary can link
without the standard library, but builds with std under `cargo test`. The binary
(`src/main.rs`, `src/rt.rs`) adds only what a C runtime would otherwise: the entry
point, the `mem*` functions, the allocator and the panic handler. `build.rs` passes
its link flags, so a plain `cargo build` or `cargo install` produces it.

## Adding a logo

1. Draw the bitmap in `logos/<name>.txt`. The format (`.` for empty, `1`–`9` for
   colour slots, two rows per terminal line) is described at the top of `src/logo.rs`.
   Pad every row to the same width.
2. Add a `logo!` entry to `LOGOS` in `src/logo.rs` with the `os-release` IDs it stands
   for and its colour slots as ANSI palette indices. The first colour also accents the
   info column.
3. `cargo test` checks that every logo renders rectangular and uses only defined
   colours. `build.rs` pre-renders the coloured art with the same code, so there is
   nothing else to regenerate. Then look at it in a light and a dark theme:
   `cargo run -- --logo <name>`.

## Screenshots and benchmarks

The images in `assets/` and the numbers in `docs/` are generated, so regenerate them
instead of editing them:

```sh
scripts/bench.sh          # docs/benchmarks.{md,json}; needs hyperfine
scripts/screenshots.py    # assets/*.png; needs fish, rsvg-convert and IosevkaTerm
```

`bench.sh` times whichever of the other fetch tools are on your `PATH`. Run it on an
idle machine, and keep the date and machine line it writes, because the numbers mean
nothing without them. Run `screenshots.py` after `bench.sh` so the chart matches the table.
