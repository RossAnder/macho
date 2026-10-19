//! Logos as small pixel bitmaps, drawn with half-block characters so each terminal
//! cell carries two vertical pixels.
//!
//! Bitmaps live in `logos/<name>.txt`: `.` is empty and `1`–`9` index into the
//! logo's `colors`, which are ANSI palette indices so the art follows the terminal
//! theme. In shaded logos, normal colours 1–7 dither into their bright twins down
//! the height of their slot; flat logos carry their own lighting in the bitmap.

use crate::term::{Block, DEFAULT_BG, fg_code};

pub struct Logo {
    pub name: &'static str,
    /// os-release `ID` values this logo stands for.
    ids: &'static [&'static str],
    colors: &'static [u8],
    shaded: bool,
    art: &'static str,
}

macro_rules! logo {
    ($name:literal, ids: $ids:expr, colors: $colors:expr) => {
        logo!($name, ids: $ids, colors: $colors, shaded: true)
    };
    ($name:literal, ids: $ids:expr, colors: $colors:expr, shaded: $shaded:expr) => {
        Logo {
            name: $name,
            ids: &$ids,
            colors: &$colors,
            shaded: $shaded,
            art: include_str!(concat!("../logos/", $name, ".txt")),
        }
    };
}

const FALLBACK: &str = "linux";

const LOGOS: &[Logo] = &[
    logo!("arch", ids: ["arch", "archarm"], colors: [4]),
    logo!("alpine", ids: ["alpine"], colors: [4]),
    logo!("debian", ids: ["debian"], colors: [1]),
    logo!("endeavouros", ids: ["endeavouros"], colors: [5]),
    logo!("fedora", ids: ["fedora"], colors: [4]),
    logo!("nixos", ids: ["nixos"], colors: [4]),
    logo!("ubuntu", ids: ["ubuntu"], colors: [1]),
    logo!("void", ids: ["void"], colors: [2]),
    logo!("linux", ids: [], colors: [3, 8, 15]),
    logo!("windows", ids: [], colors: [4, 1, 2, 3, 0]),
    logo!("egg", ids: [], colors: [3, 11, 15, 7, 8, 0], shaded: false),
];

pub fn by_name(name: &str) -> Option<&'static Logo> {
    LOGOS.iter().find(|l| l.name.eq_ignore_ascii_case(name))
}

pub fn names() -> impl Iterator<Item = &'static str> {
    LOGOS.iter().map(|l| l.name)
}

/// Picks the logo for the most specific matching os-release id, falling back to Tux.
pub fn detect(os_ids: &[String]) -> &'static Logo {
    os_ids
        .iter()
        .find_map(|id| LOGOS.iter().find(|l| l.ids.contains(&id.as_str())))
        .or_else(|| by_name(FALLBACK))
        .expect("the fallback logo is in the table")
}

impl Logo {
    /// The colour used to accent text alongside this logo.
    pub fn accent(&self) -> u8 {
        self.colors[0]
    }

    /// Renders the bitmap to terminal lines that are all exactly `width` cells wide.
    pub fn render(&self, color: bool) -> Art {
        let rows: Vec<&[u8]> = self.art.lines().map(str::as_bytes).collect();
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let slot_at = |y: usize, x: usize| match rows.get(y)?.get(x)? {
            d @ b'1'..=b'9' => Some(usize::from(d - b'1')),
            _ => None,
        };

        // Each slot shades over its own rows, so separate shapes (the Windows panes)
        // each get a full gradient.
        let mut spans = [(usize::MAX, 0); 9];
        for (y, row) in rows.iter().enumerate() {
            for x in 0..row.len() {
                if let Some(slot) = slot_at(y, x) {
                    let span = &mut spans[slot];
                    *span = (span.0.min(y), span.1.max(y));
                }
            }
        }

        let pixel = |y: usize, x: usize| -> Option<u8> {
            let slot = slot_at(y, x)?;
            let base = *self.colors.get(slot)?;
            let (top, bottom) = spans[slot];
            let shades = self.shaded && (1..=7).contains(&base);
            Some(if shades && faded(x, y, y - top, bottom - top + 1) { base + 8 } else { base })
        };

        let height = rows.len().div_ceil(2);
        let mut lines = Block::new(color, height * width * if color { 8 } else { 3 }, height);
        for y in (0..rows.len()).step_by(2) {
            let mut pen = Pen::default();
            for x in 0..width {
                let (glyph, fg, bg) = match (pixel(y, x), pixel(y + 1, x)) {
                    (None, None) => (' ', pen.fg, None),
                    (Some(t), None) => ('▀', Some(t), None),
                    (None, Some(b)) => ('▄', Some(b), None),
                    (Some(t), Some(b)) if t == b || !color => ('█', Some(t), None),
                    (Some(t), Some(b)) => ('▀', Some(t), Some(b)),
                };
                pen.set(&mut lines, fg, bg);
                lines.push_char(glyph);
            }
            if pen != Pen::default() {
                lines.reset();
            }
            lines.end_line();
        }
        Art { lines, width }
    }
}

/// Ordered (Bayer) dither at offset `depth` into a span of `height` rows: solid base
/// colour over the top third, solid shade over the bottom third, and a pixel-art
/// blend between. `x`/`y` are absolute so the pattern lines up across slots.
fn faded(x: usize, y: usize, depth: usize, height: usize) -> bool {
    const BAYER: [[usize; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let level = ((3 * depth).saturating_sub(height) * 16 / height).min(16);
    BAYER[y % 4][x % 4] < level
}

/// Rendered logo lines, each exactly `width` cells wide.
pub struct Art {
    pub lines: Block,
    pub width: usize,
}

/// Tracks the active colours so only changes are emitted.
#[derive(Default, PartialEq)]
struct Pen {
    fg: Option<u8>,
    bg: Option<u8>,
}

impl Pen {
    fn set(&mut self, out: &mut Block, fg: Option<u8>, bg: Option<u8>) {
        if fg != self.fg {
            if let Some(c) = fg {
                out.sgr(&[fg_code(c)]);
            }
            self.fg = fg;
        }
        if bg != self.bg {
            out.sgr(&[bg.map_or(DEFAULT_BG, |c| fg_code(c) + 10)]);
            self.bg = bg;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_logo_renders_rectangular() {
        for logo in LOGOS {
            let art = logo.render(false);
            assert!(art.width > 0, "{} is empty", logo.name);
            for i in 0..art.lines.len() {
                let line = art.lines.line(i).unwrap();
                assert_eq!(line.chars().count(), art.width, "{} has a ragged line", logo.name);
            }
        }
    }

    #[test]
    fn every_colour_slot_is_defined() {
        for logo in LOGOS {
            let max = logo.art.bytes().filter(u8::is_ascii_digit).max().unwrap_or(b'1');
            assert!(usize::from(max - b'0') <= logo.colors.len(), "{} uses an undefined colour", logo.name);
        }
    }

    #[test]
    fn detection_prefers_id_then_id_like() {
        let ids = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(detect(&ids(&["cachyos", "arch"])).name, "arch");
        assert_eq!(detect(&ids(&["endeavouros", "arch"])).name, "endeavouros");
        assert_eq!(detect(&ids(&["linuxmint", "ubuntu", "debian"])).name, "ubuntu");
        assert_eq!(detect(&ids(&["plan9"])).name, FALLBACK);
        assert_eq!(detect(&[]).name, FALLBACK);
    }
}
