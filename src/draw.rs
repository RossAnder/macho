//! Draws logos as terminal text. build.rs runs this to prerender every logo, so the
//! binary only copies the result out; the tests run it to check that prerendered art.

use alloc::vec;
use alloc::vec::Vec;

use crate::logo::Logo;
use crate::term::{Block, fg_code};

const DEFAULT_FG: u8 = 39;
const DEFAULT_BG: u8 = 49;

impl Logo {
    /// Renders the bitmap to terminal lines that are all exactly `width` cells wide.
    pub fn render(&self, color: bool) -> Art {
        let (pixels, width, rows) = self.pixels();
        let pixel = |y: usize, x: usize| if y < rows { pixels[y * width + x] } else { None };

        let height = rows.div_ceil(2);
        let mut lines = Block::new(color, height * width * if color { 8 } else { 3 }, height);
        for y in (0..rows).step_by(2) {
            let cells: Vec<_> =
                (0..width).map(|x| ways_to_draw(pixel(y, x), pixel(y + 1, x), color)).collect();
            let mut pen = Pen::DEFAULT;
            for (glyph, next) in cheapest_line(&cells) {
                pen.change_to(next, &mut lines);
                lines.push_char(glyph);
            }
            if pen != Pen::DEFAULT {
                lines.reset();
            }
            lines.end_line();
        }
        Art { lines, width }
    }

    /// Every pixel's palette index (`None` where empty), row by row, with the width and
    /// the number of rows.
    fn pixels(&self) -> (Vec<Option<u8>>, usize, usize) {
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

        // Resolve every pixel to a palette index once. The fade level depends only on the
        // slot and the row, so it is worked out per row rather than per pixel.
        let mut pixels = vec![None; rows.len() * width];
        for (y, row) in rows.iter().enumerate() {
            let levels =
                spans.map(
                    |(top, bottom)| {
                        if y < top || y > bottom { 0 } else { fade_level(y - top, bottom - top + 1) }
                    },
                );
            for x in 0..row.len() {
                let Some(slot) = slot_at(y, x) else { continue };
                let Some(&base) = self.colors.get(slot) else { continue };
                let shades = self.shaded && (1..=7).contains(&base);
                pixels[y * width + x] =
                    Some(if shades && BAYER[y % 4][x % 4] < levels[slot] { base + 8 } else { base });
            }
        }
        (pixels, width, rows.len())
    }
}

/// The terminal's own colour, which a pen starts with.
const DEFAULT: u8 = u8::MAX;
/// Whichever colour the pen already holds.
const ANY: u8 = u8::MAX - 1;

/// A glyph and the foreground and background it needs to show a cell's two pixels.
type Way = (char, u8, u8);

/// The ways to draw a cell with pixel `top` above `bottom`. Two different colours can go
/// either way up, which lets the cheapest line flip glyphs instead of changing colours.
fn ways_to_draw(top: Option<u8>, bottom: Option<u8>, color: bool) -> ([Way; 2], usize) {
    let none = (' ', ANY, DEFAULT);
    match (top, bottom) {
        (None, None) => ([none, none], 1),
        (Some(t), None) => ([('▀', t, DEFAULT), none], 1),
        (None, Some(b)) => ([('▄', b, DEFAULT), none], 1),
        (Some(t), Some(b)) if t == b || !color => ([('█', t, ANY), none], 1),
        (Some(t), Some(b)) => ([('▀', t, b), ('▄', b, t)], 2),
    }
}

/// Picks a way to draw each cell so the line spends the fewest bytes on colour changes,
/// including the reset at its end: a shortest path over the pen's states.
fn cheapest_line(cells: &[([Way; 2], usize)]) -> Vec<(char, Pen)> {
    struct Step {
        pen: Pen,
        cost: usize,
        from: usize,
        glyph: char,
    }
    let mut layers = vec![vec![Step { pen: Pen::DEFAULT, cost: 0, from: 0, glyph: ' ' }]];
    for (ways, n) in cells {
        let mut next: Vec<Step> = Vec::new();
        for (from, step) in layers[layers.len() - 1].iter().enumerate() {
            for &(glyph, fg, bg) in &ways[..*n] {
                let pen = Pen {
                    fg: if fg == ANY { step.pen.fg } else { fg },
                    bg: if bg == ANY { step.pen.bg } else { bg },
                };
                let cost = step.cost + step.pen.change_cost(pen);
                match next.iter_mut().find(|s| s.pen == pen) {
                    Some(s) if s.cost <= cost => {}
                    Some(s) => *s = Step { pen, cost, from, glyph },
                    None => next.push(Step { pen, cost, from, glyph }),
                }
            }
        }
        layers.push(next);
    }
    let reset = |pen: Pen| if pen == Pen::DEFAULT { 0 } else { "\x1b[0m".len() };
    let last = layers[layers.len() - 1].iter().enumerate().min_by_key(|(_, s)| s.cost + reset(s.pen));
    let mut at = last.map_or(0, |(i, _)| i);
    let mut drawn = Vec::with_capacity(cells.len());
    for layer in layers[1..].iter().rev() {
        let step = &layer[at];
        drawn.push((step.glyph, step.pen));
        at = step.from;
    }
    drawn.reverse();
    drawn
}

/// Ordered (Bayer) dither thresholds. A pixel takes the shade when its threshold is below
/// the fade level; indexing by absolute `x`/`y` lines the pattern up across slots.
const BAYER: [[usize; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Fade level (0–16) at offset `depth` into a span of `height` rows: solid base colour
/// over the top third, solid shade over the bottom third, and a dithered blend between.
fn fade_level(depth: usize, height: usize) -> usize {
    ((3 * depth).saturating_sub(height) * 16 / height).min(16)
}

/// Rendered logo lines, each exactly `width` cells wide.
pub struct Art {
    pub lines: Block,
    pub width: usize,
}

/// The active colours, as palette indices or `DEFAULT`, so only changes are emitted.
#[derive(Clone, Copy, PartialEq)]
struct Pen {
    fg: u8,
    bg: u8,
}

impl Pen {
    const DEFAULT: Self = Self { fg: DEFAULT, bg: DEFAULT };

    /// The SGR codes that turn this pen into `next`.
    fn codes(self, next: Self) -> ([u8; 2], usize) {
        let mut codes = [0; 2];
        let mut n = 0;
        if next.fg != self.fg {
            codes[n] = if next.fg == DEFAULT { DEFAULT_FG } else { fg_code(next.fg) };
            n += 1;
        }
        if next.bg != self.bg {
            codes[n] = if next.bg == DEFAULT { DEFAULT_BG } else { fg_code(next.bg) + 10 };
            n += 1;
        }
        (codes, n)
    }

    /// Bytes of the escape sequence that turns this pen into `next`.
    fn change_cost(self, next: Self) -> usize {
        let (codes, n) = self.codes(next);
        let digits: usize = codes[..n].iter().map(|&c| if c >= 100 { 3 } else { 2 }).sum();
        if n == 0 { 0 } else { "\x1b[m".len() + digits + n - 1 }
    }

    fn change_to(&mut self, next: Self, out: &mut Block) {
        let (codes, n) = self.codes(next);
        out.sgr(&codes[..n]);
        *self = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::logo::LOGOS;

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

    /// Replays a coloured line as a terminal would, giving each cell's top and bottom
    /// pixel (`None` for the default background).
    fn decode(line: &str) -> Vec<(Option<u8>, Option<u8>)> {
        let (mut fg, mut bg, mut cells) = (None, None, Vec::new());
        let mut rest = line;
        while let Some(c) = rest.chars().next() {
            if let Some(seq) = rest.strip_prefix("\x1b[") {
                let end = seq.find('m').unwrap();
                for code in seq[..end].split(';').map(|c| c.parse::<u8>().unwrap()) {
                    match code {
                        0 => (fg, bg) = (None, None),
                        39 => fg = None,
                        49 => bg = None,
                        30..=37 => fg = Some(code - 30),
                        90..=97 => fg = Some(code - 90 + 8),
                        40..=47 => bg = Some(code - 40),
                        100..=107 => bg = Some(code - 100 + 8),
                        _ => panic!("unexpected SGR {code}"),
                    }
                }
                rest = &seq[end + 1..];
                continue;
            }
            cells.push(match c {
                ' ' => (bg, bg),
                '▀' => (fg, bg),
                '▄' => (bg, fg),
                '█' => (fg, fg),
                _ => panic!("unexpected glyph {c:?}"),
            });
            rest = &rest[c.len_utf8()..];
        }
        assert_eq!((fg, bg), (None, None), "line leaves colour on: {line:?}");
        cells
    }

    #[test]
    fn coloured_logos_show_exactly_their_pixels() {
        for logo in LOGOS {
            let (pixels, width, rows) = logo.pixels();
            let pixel = |y: usize, x: usize| if y < rows { pixels[y * width + x] } else { None };
            let art = logo.render(true);
            let plain = logo.render(false);
            for i in 0..art.lines.len() {
                let cells = decode(art.lines.line(i).unwrap());
                assert_eq!(cells.len(), art.width, "{} line {i}", logo.name);
                for (x, &cell) in cells.iter().enumerate() {
                    assert_eq!(cell, (pixel(2 * i, x), pixel(2 * i + 1, x)), "{} cell {x},{i}", logo.name);
                }
                // The plain render draws the same shape.
                let shape: Vec<_> = cells.iter().map(|&(t, b)| (t.is_some(), b.is_some())).collect();
                let plain_shape: Vec<_> = plain
                    .lines
                    .line(i)
                    .unwrap()
                    .chars()
                    .map(|c| match c {
                        ' ' => (false, false),
                        '▀' => (true, false),
                        '▄' => (false, true),
                        _ => (true, true),
                    })
                    .collect();
                assert_eq!(shape, plain_shape, "{} line {i}", logo.name);
            }
        }
    }
}
