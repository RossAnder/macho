//! Lays the logo and the info column side by side.

use std::fmt::Write;

use crate::info::{Memory, System};
use crate::logo::Logo;
use crate::term::{BOLD, Block, DIM, fg_code};

const MARGIN: &str = "  ";
const GAP: &str = "   ";
const KEY_WIDTH: usize = 8;

pub fn render(sys: &System, logo: &Logo, color: bool) -> String {
    let art = logo.render(color);
    let info = info_block(sys, color, fg_code(logo.accent()));

    let height = art.lines.len().max(info.len());
    let art_top = (height - art.lines.len()) / 2;
    let info_top = (height - info.len()) / 2;

    let mut out = String::with_capacity(
        art.lines.text_len() + info.text_len() + height * (MARGIN.len() + GAP.len() + art.width + 1) + 2,
    );
    out.push('\n');
    for row in 0..height {
        out.push_str(MARGIN);
        match row.checked_sub(art_top).and_then(|i| art.lines.line(i)) {
            Some(line) => out.push_str(line),
            None => out.extend(std::iter::repeat_n(' ', art.width)),
        }
        if let Some(line) = row.checked_sub(info_top).and_then(|i| info.line(i)) {
            out.push_str(GAP);
            out.push_str(line);
        }
        out.push('\n');
    }
    out.push('\n');
    out
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
        let _ = write!(b, "{key:<KEY_WIDTH$}");
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
        let _ = write!(b, "{count} ");
        b.paint(manager, &[DIM]);
    }
}

fn write_uptime(b: &mut Block, secs: u64) {
    let (days, hours, mins) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    let _ = match (days, hours) {
        (0, 0) => write!(b, "{mins}m"),
        (0, _) => write!(b, "{hours}h {mins}m"),
        _ => write!(b, "{days}d {hours}h {mins}m"),
    };
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
    let _ = write!(b, "{}.{} / {}.{} GiB ", used / 10, used % 10, total / 10, total % 10);
    b.sgr(&[fg_code(level)]);
    let _ = write!(b, "{percent}%");
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
