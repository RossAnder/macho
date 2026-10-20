//! macho's internals, split from the binary so tests can run with std. This is not a
//! stable API.
//!
//! Nothing here links std or libc: `sys` talks to the kernel directly, and the binary
//! supplies the process entry point, allocator and panic handler.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bump;
#[cfg(test)]
mod draw;
mod info;
mod logo;
mod num;
mod render;
pub mod sys;
mod term;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;

const SUCCESS: i32 = 0;
const FAILURE: i32 = 1;

const HELP: &str = "\
macho — a small, fast system fetch

usage: macho [options]

  -l, --logo <name>   use a specific logo (see --list-logos)
      --list-logos    print the available logos
  -h, --help          show this help
  -V, --version       show the version

Colour is disabled when NO_COLOR is set.
";

/// Runs macho with its arguments (not including the program name) and returns the exit
/// status. A closed pipe (`macho | head`) ends the process by SIGPIPE, as for a C tool.
pub fn run<'a>(mut args: impl Iterator<Item = &'a [u8]>) -> i32 {
    let mut forced_logo = None;
    while let Some(arg) = args.next() {
        let Ok(flag) = core::str::from_utf8(arg) else {
            return fail(&format!("unknown argument '{}' (try --help)", String::from_utf8_lossy(arg)));
        };
        match flag {
            "-l" | "--logo" => match args.next().map(core::str::from_utf8) {
                Some(Ok(name)) => forced_logo = Some(name.to_owned()),
                _ => return fail("--logo needs a name (see --list-logos)"),
            },
            "--list-logos" => {
                let names: String = logo::names().flat_map(|name| [name, "\n"]).collect();
                sys::write_all(1, names.as_bytes());
                return SUCCESS;
            }
            "-h" | "--help" => {
                sys::write_all(1, HELP.as_bytes());
                return SUCCESS;
            }
            "-V" | "--version" => {
                sys::write_all(1, concat!("macho ", env!("CARGO_PKG_VERSION"), "\n").as_bytes());
                return SUCCESS;
            }
            _ => match flag.strip_prefix("--logo=") {
                Some(name) => forced_logo = Some(name.to_owned()),
                None => return fail(&format!("unknown argument '{flag}' (try --help)")),
            },
        }
    }

    let sys = info::System::gather();
    let logo = match forced_logo {
        Some(name) => match logo::by_name(&name) {
            Some(logo) => logo,
            None => return fail(&format!("unknown logo '{name}' (try --list-logos)")),
        },
        None => logo::detect(&sys.os_ids),
    };
    let color = sys::var_bytes("NO_COLOR").is_none_or(<[u8]>::is_empty);
    render::print(1, &sys, logo, color);
    SUCCESS
}

fn fail(msg: &str) -> i32 {
    sys::write_all(2, format!("macho: {msg}\n").as_bytes());
    FAILURE
}
