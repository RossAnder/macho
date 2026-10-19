//! ANSI styling and a line buffer that keeps a whole block of text in one allocation.

use std::fmt::{self, Write};

pub const BOLD: u8 = 1;
pub const DIM: u8 = 2;
pub const DEFAULT_BG: u8 = 49;

/// SGR foreground code for a 16-colour palette index (add 10 for background).
pub fn fg_code(index: u8) -> u8 {
    match index {
        0..=7 => 30 + index,
        _ => 90 + (index & 7),
    }
}

/// Lines of text stored back to back in one `String`. Styling calls are no-ops when
/// the block was created without colour, so callers never branch on it.
#[derive(Default)]
pub struct Block {
    text: String,
    ends: Vec<usize>,
    color: bool,
}

impl Block {
    pub fn new(color: bool, bytes: usize, lines: usize) -> Self {
        Self { text: String::with_capacity(bytes), ends: Vec::with_capacity(lines), color }
    }

    pub fn color(&self) -> bool {
        self.color
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn text_len(&self) -> usize {
        self.text.len()
    }

    pub fn line(&self, i: usize) -> Option<&str> {
        let end = *self.ends.get(i)?;
        let start = i.checked_sub(1).map_or(0, |p| self.ends[p]);
        Some(&self.text[start..end])
    }

    pub fn end_line(&mut self) {
        self.ends.push(self.text.len());
    }

    pub fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    pub fn push_char(&mut self, c: char) {
        self.text.push(c);
    }

    pub fn sgr(&mut self, codes: &[u8]) {
        if !self.color || codes.is_empty() {
            return;
        }
        self.text.push_str("\x1b[");
        for (i, code) in codes.iter().enumerate() {
            if i > 0 {
                self.text.push(';');
            }
            let _ = write!(self.text, "{code}");
        }
        self.text.push('m');
    }

    pub fn reset(&mut self) {
        if self.color {
            self.text.push_str("\x1b[0m");
        }
    }

    pub fn paint(&mut self, text: &str, codes: &[u8]) {
        self.sgr(codes);
        self.text.push_str(text);
        self.reset();
    }
}

impl Write for Block {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.text.push_str(s);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_lines_round_trip() {
        let mut b = Block::default();
        b.push("one");
        b.end_line();
        b.end_line();
        b.push("three");
        b.end_line();
        assert_eq!(b.len(), 3);
        assert_eq!(
            [b.line(0), b.line(1), b.line(2), b.line(3)],
            [Some("one"), Some(""), Some("three"), None]
        );
    }

    #[test]
    fn styling_is_inert_without_colour() {
        let mut plain = Block::default();
        plain.paint("hi", &[BOLD, fg_code(12)]);
        assert_eq!(plain.text, "hi");

        let mut coloured = Block::new(true, 0, 0);
        coloured.paint("hi", &[BOLD, fg_code(12)]);
        assert_eq!(coloured.text, "\x1b[1;94mhi\x1b[0m");
    }
}
