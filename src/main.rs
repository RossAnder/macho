//! The macho binary: the process entry point, allocator and panic handler around
//! [`macho::run`]. build.rs links it without libc's startup files (see `rt`).

// `test = false` keeps it out of `cargo test`, but `clippy --all-targets` still checks
// it as a test, with std.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

#[cfg(not(test))]
mod rt;

#[cfg(not(test))]
#[global_allocator]
static ALLOC: macho::bump::Bump = macho::bump::Bump;
