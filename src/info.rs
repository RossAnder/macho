//! System information, read straight from `/proc`, `/sys` and the environment.
//!
//! Nothing here spawns a process: every field is a handful of syscalls at most.
//! Each probe is independent and fails soft, so a missing source just drops a line.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::CStr;

use crate::num::Decimal;
use crate::sys;

const ARCH: &str = if cfg!(target_arch = "x86_64") { "x86_64" } else { "aarch64" };

pub struct System {
    pub user: String,
    pub host: String,
    pub os: Option<String>,
    /// `ID` followed by each `ID_LIKE` entry from os-release, most specific first.
    pub os_ids: Vec<String>,
    pub kernel: Option<String>,
    pub uptime_secs: Option<u64>,
    pub packages: Vec<(&'static str, usize)>,
    pub shell: Option<String>,
    pub desktop: Option<String>,
    pub terminal: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<Memory>,
}

pub struct Memory {
    pub used_kib: u64,
    pub total_kib: u64,
}

impl System {
    pub fn gather() -> Self {
        let (os, os_ids) = os_release();
        let (shell, terminal) = shell_and_terminal();
        let [node, kernel, _] = sys::uname().map_or([None, None, None], |fields| fields.map(Some));
        Self {
            user: user(),
            host: host(node),
            os,
            os_ids,
            kernel: kernel.filter(|k| !k.is_empty()),
            uptime_secs: sys::uptime_secs(),
            packages: packages(),
            shell,
            desktop: desktop(),
            terminal,
            cpu: cpu(),
            memory: memory(),
        }
    }
}

fn read_trimmed(path: &CStr) -> Option<String> {
    let s = sys::read_small(path)?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn var(key: &str) -> Option<String> {
    sys::var(key).filter(|v| !v.is_empty()).map(str::to_owned)
}

fn user() -> String {
    var("USER").or_else(|| var("LOGNAME")).or_else(user_from_passwd).unwrap_or_else(|| "user".into())
}

fn user_from_passwd() -> Option<String> {
    let status = sys::read_small(c"/proc/self/status")?;
    let uid = status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?;
    let mut user = None;
    sys::for_each_line(c"/etc/passwd", |line| {
        let mut fields = line.split(|&b| b == b':');
        let name = fields.next();
        if fields.nth(1) == Some(uid.as_bytes()) {
            user = name.and_then(|n| core::str::from_utf8(n).ok()).map(str::to_owned);
        }
        user.is_none()
    });
    user
}

fn host(node: Option<String>) -> String {
    node.filter(|n| !n.is_empty() && n != "(none)")
        .or_else(|| read_trimmed(c"/etc/hostname"))
        .or_else(|| var("HOSTNAME"))
        .unwrap_or_else(|| "localhost".into())
}

fn os_release() -> (Option<String>, Vec<String>) {
    let Some(text) = sys::read_small(c"/etc/os-release").or_else(|| sys::read_small(c"/usr/lib/os-release"))
    else {
        return (None, Vec::new());
    };

    let (mut pretty, mut name, mut id, mut like) = (None, None, None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        match key.trim() {
            "PRETTY_NAME" => pretty = Some(value),
            "NAME" => name = Some(value),
            "ID" => id = Some(value),
            "ID_LIKE" => like = Some(value),
            _ => {}
        }
    }

    let os = pretty.or(name).map(|n| [n, " ", ARCH].concat());
    let ids = id
        .into_iter()
        .chain(like.into_iter().flat_map(str::split_whitespace))
        .map(str::to_ascii_lowercase)
        .collect();
    (os, ids)
}

fn packages() -> Vec<(&'static str, usize)> {
    let home = sys::var_bytes("HOME").unwrap_or_default();
    let in_home = |rest: &[u8]| sys::path(&[home, rest]);
    let counts = [
        ("pacman", count_dirs(c"/var/lib/pacman/local")),
        ("dpkg", count_matching_lines(c"/var/lib/dpkg/status", |l| l == b"Status: install ok installed")),
        ("apk", count_matching_lines(c"/lib/apk/db/installed", |l| l.starts_with(b"P:"))),
        ("xbps", xbps()),
        (
            "flatpak",
            count_dirs(c"/var/lib/flatpak/app")
                + in_home(b"/.local/share/flatpak/app").map_or(0, |p| count_dirs(&p)),
        ),
        // Each element of a `nix profile` manifest lists its store paths.
        (
            "nix",
            in_home(b"/.nix-profile/manifest.json").map_or(0, |p| sys::count_in_file(&p, b"\"storePaths\"")),
        ),
    ];
    counts.into_iter().filter(|&(_, n)| n > 0).collect()
}

fn count_dirs(path: &CStr) -> usize {
    let Some(st) = sys::stat(path).filter(|st| st.is_dir) else { return 0 };
    // A directory's link count is 2 + its subdirectories on most filesystems, which
    // saves listing it. btrfs always reports 1, so fall back to counting entries.
    if let Some(subdirs) = st.nlink.checked_sub(2) {
        return subdirs as usize;
    }
    let mut count = 0;
    sys::for_each_entry(path, |name, kind| {
        count += usize::from(kind == sys::DT_DIR && name != b"." && name != b"..");
    });
    count
}

fn count_matching_lines(path: &CStr, pred: impl Fn(&[u8]) -> bool) -> usize {
    let mut count = 0;
    sys::for_each_line(path, |line| {
        count += usize::from(pred(line));
        true
    });
    count
}

fn xbps() -> usize {
    let mut pkgdb = None;
    sys::for_each_entry(c"/var/db/xbps", |name, _| {
        if pkgdb.is_none() && name.starts_with(b"pkgdb-") && name.ends_with(b".plist") {
            pkgdb = sys::path(&[b"/var/db/xbps/", name]);
        }
    });
    pkgdb.map_or(0, |path| sys::count_in_file(&path, b"<string>installed</string>"))
}

const SHELLS: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ksh", "mksh", "oksh", "tcsh", "csh", "nu", "elvish", "xonsh",
    "ion", "yash", "pwsh", "murex", "oil", "osh",
];

/// Processes that sit between a shell and its terminal without being either.
const PASSTHROUGH: &[&str] = &["sudo", "doas", "su", "run0", "login", "script", "env", "nohup"];

struct Proc {
    name: String,
    ppid: u32,
}

fn proc_info(pid: u32) -> Option<Proc> {
    // The main thread's stat carries the same name and parent, without the kernel summing
    // CPU times over every thread first (terminals are usually multi-threaded).
    let pid = Decimal::new(pid.into());
    let stat =
        sys::read_small(&sys::path(&[b"/proc/", pid.as_bytes(), b"/task/", pid.as_bytes(), b"/stat"])?)?;
    // `comm` is parenthesised and may itself contain spaces or parens.
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let ppid = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
    Some(Proc { name: normalise_comm(&stat[open + 1..close]), ppid })
}

fn proc_name(pid: u32) -> Option<String> {
    let pid = Decimal::new(pid.into());
    let comm = sys::read_small(&sys::path(&[b"/proc/", pid.as_bytes(), b"/comm"])?)?;
    Some(normalise_comm(comm.strip_suffix('\n').unwrap_or(&comm)))
}

fn normalise_comm(comm: &str) -> String {
    let name = comm.trim_start_matches('.');
    let name = name.strip_suffix("-wrapped").unwrap_or(name);
    let name = name.split(':').next().unwrap_or(name); // "tmux: server"
    match name {
        "gnome-terminal-" | "gnome-terminal-server" => "gnome-terminal",
        "wezterm-gui" => "wezterm",
        "footclient" => "foot",
        n if n.starts_with("sshd") => "ssh",
        n => n,
    }
    .to_owned()
}

/// Walks up the process tree: the first shell ancestor is the shell, and the first
/// real process above it is the terminal. Anything below the shell (e.g. `cargo run`)
/// is ignored.
fn shell_and_terminal() -> (Option<String>, Option<String>) {
    let mut shell = None;
    let mut terminal = None;
    let mut pid = sys::getppid();

    for _ in 0..16 {
        if pid <= 1 {
            break;
        }
        // Above the shell the walk usually ends at the next process, which needs only its
        // name: `comm` is cheaper for the kernel to produce than `stat`.
        if shell.is_some() {
            let Some(name) = proc_name(pid) else { break };
            if !SHELLS.contains(&name.as_str()) && !PASSTHROUGH.contains(&name.as_str()) {
                if !matches!(name.as_str(), "systemd" | "init") {
                    terminal = Some(name);
                }
                break;
            }
        }
        let Some(proc) = proc_info(pid) else { break };
        if SHELLS.contains(&proc.name.as_str()) {
            shell.get_or_insert(proc.name);
        } else if shell.is_some() && !PASSTHROUGH.contains(&proc.name.as_str()) {
            if !matches!(proc.name.as_str(), "systemd" | "init") {
                terminal = Some(proc.name);
            }
            break;
        }
        pid = proc.ppid;
    }

    let shell = shell.or_else(|| var("SHELL").map(|s| basename(&s).to_owned()));
    let terminal = terminal.or_else(|| var("TERM_PROGRAM")).or_else(|| var("TERM"));
    (shell, terminal)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn desktop() -> Option<String> {
    let de = var("XDG_CURRENT_DESKTOP")
        .or_else(|| var("XDG_SESSION_DESKTOP"))
        .or_else(|| var("DESKTOP_SESSION"))
        .map(|value| {
            // XDG_CURRENT_DESKTOP may list several ("GNOME:ubuntu"); DESKTOP_SESSION may be a path.
            let first = value.split_once(':').map_or(value.as_str(), |(first, _)| first);
            match basename(first) {
                "KDE" | "plasma" => "KDE Plasma".to_owned(),
                name => name.to_owned(),
            }
        })
        .or_else(|| {
            [
                ("HYPRLAND_INSTANCE_SIGNATURE", "Hyprland"),
                ("NIRI_SOCKET", "niri"),
                ("SWAYSOCK", "sway"),
                ("I3SOCK", "i3"),
            ]
            .into_iter()
            .find(|(key, _)| sys::var_bytes(key).is_some())
            .map(|(_, name)| name.to_owned())
        })?;

    match var("XDG_SESSION_TYPE").as_deref() {
        Some(session @ ("wayland" | "x11")) => Some([de.as_str(), " (", session, ")"].concat()),
        _ => Some(de),
    }
}

fn cpu() -> Option<String> {
    let model = cpu_brand().or_else(cpu_model_from_proc)?;
    let name = clean_cpu_name(&model);
    Some(match online_cpus() {
        Some(n) => [name.as_str(), " (", Decimal::new(n as u64).as_str(), ")"].concat(),
        None => name,
    })
}

/// The brand string straight from the processor: no syscalls at all.
#[cfg(target_arch = "x86_64")]
fn cpu_brand() -> Option<String> {
    use core::arch::x86_64::__cpuid;

    if __cpuid(0x8000_0000).eax < 0x8000_0004 {
        return None;
    }
    let mut bytes = [0; 48];
    for (leaf, chunk) in (0x8000_0002..=0x8000_0004).zip(bytes.chunks_mut(16)) {
        let r = __cpuid(leaf);
        for (reg, out) in [r.eax, r.ebx, r.ecx, r.edx].into_iter().zip(chunk.chunks_mut(4)) {
            out.copy_from_slice(&reg.to_le_bytes());
        }
    }
    let brand = String::from_utf8_lossy(&bytes);
    let brand = brand.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!brand.is_empty()).then(|| brand.to_owned())
}

#[cfg(not(target_arch = "x86_64"))]
fn cpu_brand() -> Option<String> {
    None
}

fn cpu_model_from_proc() -> Option<String> {
    // The model sits in the first processor's block, so one read is enough and spares
    // the kernel from formatting every core.
    let text = sys::read_prefix(c"/proc/cpuinfo", 4096)?;
    String::from_utf8_lossy(&text).lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(key.trim(), "model name" | "Hardware" | "Model").then(|| value.trim().to_owned())
    })
}

/// Counts CPUs from a kernel range list such as `0-7,16-23`.
fn online_cpus() -> Option<usize> {
    let list = sys::read_small(c"/sys/devices/system/cpu/online")?;
    list.trim().split(',').try_fold(0, |total, range| {
        let (lo, hi) = range.split_once('-').unwrap_or((range, range));
        let (lo, hi): (usize, usize) = (lo.parse().ok()?, hi.parse().ok()?);
        Some(total + hi.checked_sub(lo)? + 1)
    })
}

fn clean_cpu_name(raw: &str) -> String {
    let mut name = raw.to_owned();
    for noise in ["(R)", "(r)", "(TM)", "(tm)", " CPU", " Processor"] {
        remove_all(&mut name, noise);
    }
    // Drop marketing tails: "@ 3.60GHz", "with Radeon Graphics", "16-Core".
    for tail in [" @", " with "] {
        if let Some(i) = find(&name, tail) {
            name.truncate(i);
        }
    }
    name.split_whitespace().filter(|w| !w.ends_with("-Core")).collect::<Vec<_>>().join(" ")
}

/// `haystack.find(needle)` for a needle that starts with an ASCII byte. Scanning for that
/// byte skips the set-up of std's general substring search, which dominates on strings
/// this short.
fn find(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    let mut from = 0;
    while let Some(i) = h.get(from..)?.iter().position(|&b| b == n[0]) {
        if h[from + i..].starts_with(n) {
            return Some(from + i);
        }
        from += i + 1;
    }
    None
}

/// `*s = s.replace(needle, "")`, in place.
fn remove_all(s: &mut String, needle: &str) {
    let mut from = 0;
    while let Some(i) = find(&s[from..], needle) {
        from += i;
        s.replace_range(from..from + needle.len(), "");
    }
}

fn memory() -> Option<Memory> {
    // Both fields lead the file, so the scan stops after a few lines.
    let info = sys::read_prefix(c"/proc/meminfo", 8192)?;
    let (mut total_kib, mut available) = (None, None);
    for line in info.split(|&b| b == b'\n') {
        let Some(colon) = line.iter().position(|&b| b == b':') else { continue };
        let field = match &line[..colon] {
            b"MemTotal" => &mut total_kib,
            b"MemAvailable" => &mut available,
            _ => continue,
        };
        let value = line[colon + 1..].split(u8::is_ascii_whitespace).find(|w| !w.is_empty());
        *field = value.and_then(|v| core::str::from_utf8(v).ok()?.parse().ok());
        if total_kib.is_some() && available.is_some() {
            break;
        }
    }
    let (total_kib, available): (u64, u64) = (total_kib?, available?);
    Some(Memory { used_kib: total_kib.saturating_sub(available), total_kib })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_names_lose_marketing_noise() {
        assert_eq!(clean_cpu_name("AMD Ryzen 9 7950X 16-Core Processor"), "AMD Ryzen 9 7950X");
        assert_eq!(clean_cpu_name("Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz"), "Intel Core i7-8700K");
        assert_eq!(
            clean_cpu_name("AMD Ryzen 7 7840U w/ Radeon 780M Graphics"),
            "AMD Ryzen 7 7840U w/ Radeon 780M Graphics"
        );
        assert_eq!(clean_cpu_name("AMD Ryzen 5 5600G with Radeon Graphics"), "AMD Ryzen 5 5600G");
    }

    #[test]
    fn cpu_name_cleaning_matches_str_replace() {
        fn reference(raw: &str) -> String {
            let mut name = raw.to_owned();
            for noise in ["(R)", "(r)", "(TM)", "(tm)", " CPU", " Processor"] {
                name = name.replace(noise, "");
            }
            for tail in [" @", " with "] {
                if let Some(i) = name.find(tail) {
                    name.truncate(i);
                }
            }
            name.split_whitespace().filter(|w| !w.ends_with("-Core")).collect::<Vec<_>>().join(" ")
        }
        // Random strings from the characters the patterns are made of, to hit overlaps
        // and matches that removals create.
        let alphabet: Vec<char> = "()RrTMtm CPUProcesr@wih-x".chars().collect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let len = (seed >> 59) as usize;
            let s: String = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    alphabet[(seed % alphabet.len() as u64) as usize]
                })
                .collect();
            assert_eq!(clean_cpu_name(&s), reference(&s), "{s:?}");
        }
    }

    #[test]
    fn comm_names_are_normalised() {
        assert_eq!(normalise_comm(".kitty-wrapped"), "kitty");
        assert_eq!(normalise_comm("tmux: server"), "tmux");
        assert_eq!(normalise_comm("gnome-terminal-"), "gnome-terminal");
        assert_eq!(normalise_comm("sshd-session"), "ssh");
    }

    #[test]
    fn count_dirs_counts_subdirectories_only() {
        let dir = std::env::temp_dir().join(format!("macho-dirs-{}", std::process::id()));
        let c = |p: std::path::PathBuf| alloc::ffi::CString::new(p.to_str().unwrap()).unwrap();
        for sub in ["a", "b", "c"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("file"), "").unwrap();
        assert_eq!(count_dirs(&c(dir.clone())), 3);
        assert_eq!(count_dirs(&c(dir.join("file"))), 0);
        assert_eq!(count_dirs(c"/nonexistent/macho"), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn arch_matches_std() {
        assert_eq!(ARCH, std::env::consts::ARCH);
    }
}
