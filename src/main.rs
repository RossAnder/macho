mod info;
mod logo;
mod render;
mod term;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

const HELP: &str = "\
macho — a small, fast system fetch

usage: macho [options]

  -l, --logo <name>   use a specific logo (see --list-logos)
      --list-logos    print the available logos
  -h, --help          show this help
  -V, --version       show the version

Colour is disabled when NO_COLOR is set.
";

fn main() -> ExitCode {
    let mut forced_logo = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let Some(flag) = arg.to_str() else {
            return fail(&format!("unknown argument '{}' (try --help)", arg.display()));
        };
        match flag {
            "-l" | "--logo" => match args.next().map(OsString::into_string) {
                Some(Ok(name)) => forced_logo = Some(name),
                _ => return fail("--logo needs a name (see --list-logos)"),
            },
            "--list-logos" => {
                logo::names().for_each(|name| println!("{name}"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("macho {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
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
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());

    // A closed pipe (e.g. `macho | head`) is not worth reporting.
    let _ = io::stdout().lock().write_all(render::render(&sys, logo, color).as_bytes());
    ExitCode::SUCCESS
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("macho: {msg}");
    ExitCode::FAILURE
}
