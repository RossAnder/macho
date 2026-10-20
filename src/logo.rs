//! Logos as small pixel bitmaps, drawn with half-block characters so each terminal
//! cell carries two vertical pixels.
//!
//! Bitmaps live in `logos/<name>.txt`: `.` is empty and `1`–`9` index into the
//! logo's `colors`, which are ANSI palette indices so the art follows the terminal
//! theme. In shaded logos, normal colours 1–7 dither into their bright twins down
//! the height of their slot; flat logos carry their own lighting in the bitmap.

use alloc::string::String;

pub struct Logo {
    pub name: &'static str,
    /// os-release `ID` values this logo stands for.
    ids: &'static [&'static str],
    pub(crate) colors: &'static [u8],
    // The bitmap is drawn by `draw`, which only build.rs and the tests compile.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) shaded: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) art: &'static str,
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

pub(crate) const LOGOS: &[Logo] = &[
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
