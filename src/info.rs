//! System information, read straight from `/proc`, `/sys` and the environment.
//!
//! Nothing here spawns a process: every field is a handful of syscalls at most.
//! Each probe is independent and fails soft, so a missing source just drops a line.

use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

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
        Self {
            user: user(),
            host: host(),
            os,
            os_ids,
            kernel: read_trimmed("/proc/sys/kernel/osrelease"),
            uptime_secs: uptime(),
            packages: packages(),
            shell,
            desktop: desktop(),
            terminal,
            cpu: cpu(),
            memory: memory(),
        }
    }
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn user() -> String {
    var("USER").or_else(|| var("LOGNAME")).or_else(user_from_passwd).unwrap_or_else(|| "user".into())
}

fn user_from_passwd() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let uid = status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?;
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        (fields.nth(1)? == uid).then(|| name.to_owned())
    })
}

fn host() -> String {
    read_trimmed("/proc/sys/kernel/hostname")
        .or_else(|| read_trimmed("/etc/hostname"))
        .or_else(|| var("HOSTNAME"))
        .unwrap_or_else(|| "localhost".into())
}

fn os_release() -> (Option<String>, Vec<String>) {
    let Ok(text) =
        fs::read_to_string("/etc/os-release").or_else(|_| fs::read_to_string("/usr/lib/os-release"))
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

    let os = pretty.or(name).map(|n| format!("{n} {}", env::consts::ARCH));
    let ids = id
        .into_iter()
        .chain(like.into_iter().flat_map(str::split_whitespace))
        .map(str::to_ascii_lowercase)
        .collect();
    (os, ids)
}

fn uptime() -> Option<u64> {
    let s = fs::read_to_string("/proc/uptime").ok()?;
    s.split(['.', ' ']).next()?.parse().ok()
}

fn packages() -> Vec<(&'static str, usize)> {
    let home = var("HOME").unwrap_or_default();
    let counts = [
        ("pacman", count_dirs("/var/lib/pacman/local")),
        ("dpkg", count_matching_lines("/var/lib/dpkg/status", |l| l == "Status: install ok installed")),
        ("apk", count_matching_lines("/lib/apk/db/installed", |l| l.starts_with("P:"))),
        ("xbps", xbps()),
        (
            "flatpak",
            count_dirs("/var/lib/flatpak/app") + count_dirs(format!("{home}/.local/share/flatpak/app")),
        ),
        ("nix", nix_profile(&home)),
    ];
    counts.into_iter().filter(|&(_, n)| n > 0).collect()
}

fn count_dirs(path: impl AsRef<Path>) -> usize {
    let path = path.as_ref();
    let Ok(meta) = fs::metadata(path) else { return 0 };
    // A directory's link count is 2 + its subdirectories on most filesystems, which
    // saves listing it. btrfs always reports 1, so fall back to counting entries.
    if let Some(subdirs) = meta.nlink().checked_sub(2) {
        return usize::try_from(subdirs).unwrap_or(0);
    }
    fs::read_dir(path).map_or(0, |entries| {
        entries.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).count()
    })
}

fn count_matching_lines(path: &str, pred: impl Fn(&str) -> bool) -> usize {
    fs::read_to_string(path).map_or(0, |s| s.lines().filter(|l| pred(l)).count())
}

fn xbps() -> usize {
    let Ok(entries) = fs::read_dir("/var/db/xbps") else { return 0 };
    entries
        .filter_map(Result::ok)
        .find(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("pkgdb-") && name.ends_with(".plist")
        })
        .and_then(|e| fs::read_to_string(e.path()).ok())
        .map_or(0, |s| s.matches("<string>installed</string>").count())
}

/// Packages in the user's `nix profile` (each element of manifest.json lists its store paths).
fn nix_profile(home: &str) -> usize {
    fs::read_to_string(format!("{home}/.nix-profile/manifest.json"))
        .map_or(0, |s| s.matches("\"storePaths\"").count())
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
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `comm` is parenthesised and may itself contain spaces or parens.
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let ppid = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
    Some(Proc { name: normalise_comm(&stat[open + 1..close]), ppid })
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
    let mut pid = std::os::unix::process::parent_id();

    for _ in 0..16 {
        if pid <= 1 {
            break;
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
            .find(|(key, _)| env::var_os(key).is_some())
            .map(|(_, name)| name.to_owned())
        })?;

    match var("XDG_SESSION_TYPE").as_deref() {
        Some(session @ ("wayland" | "x11")) => Some(format!("{de} ({session})")),
        _ => Some(de),
    }
}

fn cpu() -> Option<String> {
    let model = cpu_brand().or_else(cpu_model_from_proc)?;
    let name = clean_cpu_name(&model);
    Some(match online_cpus() {
        Some(n) => format!("{name} ({n})"),
        None => name,
    })
}

/// The brand string straight from the processor: no syscalls at all.
#[cfg(target_arch = "x86_64")]
fn cpu_brand() -> Option<String> {
    use std::arch::x86_64::__cpuid;

    if __cpuid(0x8000_0000).eax < 0x8000_0004 {
        return None;
    }
    let mut bytes = Vec::with_capacity(48);
    for leaf in 0x8000_0002..=0x8000_0004 {
        let r = __cpuid(leaf);
        for reg in [r.eax, r.ebx, r.ecx, r.edx] {
            bytes.extend_from_slice(&reg.to_le_bytes());
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
    let mut buf = [0; 4096];
    let n = File::open("/proc/cpuinfo").ok()?.read(&mut buf).ok()?;
    String::from_utf8_lossy(&buf[..n]).lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(key.trim(), "model name" | "Hardware" | "Model").then(|| value.trim().to_owned())
    })
}

/// Counts CPUs from a kernel range list such as `0-7,16-23`.
fn online_cpus() -> Option<usize> {
    let list = fs::read_to_string("/sys/devices/system/cpu/online").ok()?;
    list.trim().split(',').try_fold(0, |total, range| {
        let (lo, hi) = range.split_once('-').unwrap_or((range, range));
        let (lo, hi): (usize, usize) = (lo.parse().ok()?, hi.parse().ok()?);
        Some(total + hi.checked_sub(lo)? + 1)
    })
}

fn clean_cpu_name(raw: &str) -> String {
    let mut name = raw.to_owned();
    for noise in ["(R)", "(r)", "(TM)", "(tm)", " CPU", " Processor"] {
        name = name.replace(noise, "");
    }
    // Drop marketing tails: "@ 3.60GHz", "with Radeon Graphics", "16-Core".
    for tail in [" @", " with "] {
        if let Some(i) = name.find(tail) {
            name.truncate(i);
        }
    }
    name.split_whitespace().filter(|w| !w.ends_with("-Core")).collect::<Vec<_>>().join(" ")
}

fn memory() -> Option<Memory> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let field = |key: &str| -> Option<u64> {
        info.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let total_kib = field("MemTotal")?;
    let available = field("MemAvailable")?;
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
    fn comm_names_are_normalised() {
        assert_eq!(normalise_comm(".kitty-wrapped"), "kitty");
        assert_eq!(normalise_comm("tmux: server"), "tmux");
        assert_eq!(normalise_comm("gnome-terminal-"), "gnome-terminal");
        assert_eq!(normalise_comm("sshd-session"), "ssh");
    }
}
