//! Lays the logo and the info column side by side.

use alloc::vec::Vec;

use crate::info::{Memory, System};
use crate::logo::Logo;
use crate::num::Decimal;
use crate::sys;
use crate::term::{BOLD, Block, DIM, fg_code};

const MARGIN: &str = "  ";
const GAP: &str = "   ";
const KEY_WIDTH: usize = 8;

/// Every logo as `build.rs` drew it: name, coloured lines, plain lines, width in cells.
static PRERENDERED: &[(&str, &[&str], &[&str], usize)] = include!(concat!(env!("OUT_DIR"), "/art.rs"));

/// Writes the fetch to `fd`. The art and the info column go out as they are, side by side,
/// without first being copied into one buffer.
pub fn print(fd: usize, sys: &System, logo: &Logo, color: bool) {
    let (art, width) = prerendered(logo, color);
    let info = info_block(sys, color, fg_code(logo.accent()));

    let height = art.len().max(info.len());
    let art_top = (height - art.len()) / 2;
    let info_top = (height - info.len()) / 2;

    let mut parts: Vec<&[u8]> = Vec::with_capacity(height * 5 + 2);
    parts.push(b"\n");
    for row in 0..height {
        parts.push(MARGIN.as_bytes());
        match row.checked_sub(art_top).and_then(|i| art.get(i)) {
            Some(line) => parts.push(line.as_bytes()),
            None => {
                for x in (0..width).step_by(SPACES.len()) {
                    parts.push(&SPACES.as_bytes()[..(width - x).min(SPACES.len())]);
                }
            }
        }
        if let Some(line) = row.checked_sub(info_top).and_then(|i| info.line(i)) {
            parts.push(GAP.as_bytes());
            parts.push(line.as_bytes());
        }
        parts.push(b"\n");
    }
    parts.push(b"\n");
    sys::write_all_vectored(fd, &parts);
}

/// Blank art for rows below or above a short logo.
const SPACES: &str = "                                ";

fn prerendered(logo: &Logo, color: bool) -> (&'static [&'static str], usize) {
    let (_, colored, plain, width) =
        PRERENDERED.iter().find(|&&(name, ..)| name == logo.name).expect("build.rs draws every logo");
    (if color { colored } else { plain }, *width)
}

fn push_num(b: &mut Block, n: u64) {
    b.push(Decimal::new(n).as_str());
}

enum Value<'a> {
    Text(&'a str),
    Uptime(u64),
    Packages(&'a [(&'static str, usize)]),
    Memory(&'a Memory),
}

fn info_block(sys: &System, color: bool, accent: u8) -> Block {
    let mut b = Block::new(color, 1024, 16);

    b.paint(&sys.user, &[BOLD, accent]);
    b.paint("@", &[DIM]);
    b.paint(&sys.host, &[BOLD, accent]);
    b.end_line();
    let rule_len = sys.user.chars().count() + 1 + sys.host.chars().count();
    b.sgr(&[fg_code(8)]);
    (0..rule_len).for_each(|_| b.push_char('─'));
    b.reset();
    b.end_line();

    let fields = [
        ("os", sys.os.as_deref().map(Value::Text)),
        ("kernel", sys.kernel.as_deref().map(Value::Text)),
        ("uptime", sys.uptime_secs.map(Value::Uptime)),
        ("packages", (!sys.packages.is_empty()).then_some(Value::Packages(&sys.packages))),
        ("shell", sys.shell.as_deref().map(Value::Text)),
        ("wm", sys.desktop.as_deref().map(Value::Text)),
        ("terminal", sys.terminal.as_deref().map(Value::Text)),
        ("cpu", sys.cpu.as_deref().map(Value::Text)),
        ("memory", sys.memory.as_ref().map(Value::Memory)),
    ];
    for (key, value) in fields {
        let Some(value) = value else { continue };
        b.sgr(&[accent]);
        b.push(key);
        (key.len()..KEY_WIDTH).for_each(|_| b.push_char(' '));
        b.reset();
        b.push_char(' ');
        match value {
            Value::Text(text) => b.push(text),
            Value::Uptime(secs) => write_uptime(&mut b, secs),
            Value::Packages(packages) => write_packages(&mut b, packages),
            Value::Memory(mem) => write_memory(&mut b, mem),
        }
        b.end_line();
    }

    if b.color() {
        b.end_line();
        write_palette(&mut b);
    }
    b
}

/// Two rows of swatches: the normal colours above their bright counterparts.
fn write_palette(b: &mut Block) {
    for offset in [0, 8] {
        for i in 0..8 {
            b.sgr(&[fg_code(offset + i)]);
            b.push("███ ");
        }
        b.reset();
        b.end_line();
    }
}

fn write_packages(b: &mut Block, packages: &[(&str, usize)]) {
    for (i, &(manager, count)) in packages.iter().enumerate() {
        if i > 0 {
            b.paint(" · ", &[DIM]);
        }
        push_num(b, count as u64);
        b.push_char(' ');
        b.paint(manager, &[DIM]);
    }
}

fn write_uptime(b: &mut Block, secs: u64) {
    let (days, hours, mins) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    if days > 0 {
        push_num(b, days);
        b.push("d ");
    }
    if days > 0 || hours > 0 {
        push_num(b, hours);
        b.push("h ");
    }
    push_num(b, mins);
    b.push_char('m');
}

fn write_memory(b: &mut Block, mem: &Memory) {
    // Tenths of a GiB, rounded, in integer arithmetic.
    let gib10 = |kib: u64| (kib * 10 + (1 << 19)) >> 20;
    let (used, total) = (gib10(mem.used_kib), gib10(mem.total_kib));
    let percent = mem.used_kib * 100 / mem.total_kib.max(1);
    let level = match percent {
        0..=59 => 2,
        60..=84 => 3,
        _ => 1,
    };
    for (i, tenths) in [used, total].into_iter().enumerate() {
        b.push(if i == 0 { "" } else { " / " });
        push_num(b, tenths / 10);
        b.push_char('.');
        push_num(b, tenths % 10);
    }
    b.push(" GiB ");
    b.sgr(&[fg_code(level)]);
    push_num(b, percent);
    b.push_char('%');
    b.reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(f: impl FnOnce(&mut Block)) -> String {
        let mut b = Block::default();
        f(&mut b);
        b.end_line();
        b.line(0).unwrap().to_owned()
    }

    #[test]
    fn prerendered_logos_match_the_renderer() {
        for logo in crate::logo::LOGOS {
            for color in [true, false] {
                let ((lines, width), live) = (prerendered(logo, color), logo.render(color));
                assert_eq!(width, live.width, "{}", logo.name);
                assert_eq!(lines.len(), live.lines.len(), "{}", logo.name);
                for (i, &line) in lines.iter().enumerate() {
                    assert_eq!(Some(line), live.lines.line(i), "{} line {i}", logo.name);
                }
            }
        }
    }

    #[test]
    fn uptime_drops_leading_zero_units() {
        assert_eq!(rendered(|b| write_uptime(b, 59)), "0m");
        assert_eq!(rendered(|b| write_uptime(b, 3_660)), "1h 1m");
        assert_eq!(rendered(|b| write_uptime(b, 90_061)), "1d 1h 1m");
    }

    #[test]
    fn memory_rounds_to_tenths_of_a_gib() {
        let mem = Memory { used_kib: 8_912_896, total_kib: 32_822_444 };
        assert_eq!(rendered(|b| write_memory(b, &mem)), "8.5 / 31.3 GiB 27%");
    }

    #[test]
    fn packages_join_with_dots() {
        let packages = [("pacman", 1065), ("flatpak", 12)];
        assert_eq!(rendered(|b| write_packages(b, &packages)), "1065 pacman · 12 flatpak");
    }
}
