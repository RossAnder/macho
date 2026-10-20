//! Linux system calls, made directly: macho links no libc. Wrappers return `None` (or
//! zero) on any error, because every probe that uses them fails soft anyway.

use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;
use core::ffi::CStr;

#[cfg(target_arch = "x86_64")]
mod nr {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const WRITEV: usize = 20;
    pub const CLOSE: usize = 3;
    pub const MMAP: usize = 9;
    pub const UNAME: usize = 63;
    pub const GETPPID: usize = 110;
    pub const GETDENTS64: usize = 217;
    pub const CLOCK_GETTIME: usize = 228;
    pub const EXIT_GROUP: usize = 231;
    pub const OPENAT: usize = 257;
    pub const STATX: usize = 332;
}

#[cfg(target_arch = "aarch64")]
mod nr {
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const WRITEV: usize = 66;
    pub const CLOSE: usize = 57;
    pub const MMAP: usize = 222;
    pub const UNAME: usize = 160;
    pub const GETPPID: usize = 173;
    pub const GETDENTS64: usize = 61;
    pub const CLOCK_GETTIME: usize = 113;
    pub const EXIT_GROUP: usize = 94;
    pub const OPENAT: usize = 56;
    pub const STATX: usize = 291;
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("macho makes raw system calls, and knows them for x86_64 and aarch64 only");

/// Makes system call `n`. The kernel returns a negated errno in -4095..=-1 on failure.
///
/// # Safety
/// The arguments must be valid for the call: pointers to memory of the size and
/// mutability it expects.
#[cfg(target_arch = "x86_64")]
unsafe fn syscall(n: usize, a: [usize; 6]) -> isize {
    let ret;
    // SAFETY: the caller vouches for the arguments; the kernel clobbers rcx and r11.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n as isize => ret,
            in("rdi") a[0], in("rsi") a[1], in("rdx") a[2], in("r10") a[3], in("r8") a[4], in("r9") a[5],
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Makes system call `n`. The kernel returns a negated errno in -4095..=-1 on failure.
///
/// # Safety
/// The arguments must be valid for the call: pointers to memory of the size and
/// mutability it expects.
#[cfg(target_arch = "aarch64")]
unsafe fn syscall(n: usize, a: [usize; 6]) -> isize {
    let ret;
    // SAFETY: the caller vouches for the arguments.
    unsafe {
        asm!(
            "svc 0",
            in("x8") n,
            inlateout("x0") a[0] as isize => ret,
            in("x1") a[1], in("x2") a[2], in("x3") a[3], in("x4") a[4], in("x5") a[5],
            options(nostack),
        );
    }
    ret
}

fn ok(ret: isize) -> Option<usize> {
    usize::try_from(ret).ok()
}

const AT_FDCWD: isize = -100;

/// Joins `parts` into a NUL-terminated path, or `None` if one of them contains a NUL.
pub fn path(parts: &[&[u8]]) -> Option<CString> {
    let mut buf = Vec::with_capacity(parts.iter().map(|p| p.len()).sum::<usize>() + 1);
    parts.iter().for_each(|p| buf.extend_from_slice(p));
    CString::new(buf).ok()
}

/// An open file descriptor, closed on drop.
pub struct Fd(usize);

impl Fd {
    pub fn open(path: &CStr) -> Option<Self> {
        // SAFETY: `path` is NUL-terminated and outlives the call; flags 0 is O_RDONLY.
        ok(unsafe { syscall(nr::OPENAT, [AT_FDCWD as usize, path.as_ptr() as usize, 0, 0, 0, 0]) }).map(Self)
    }

    /// One `read` into `buf`'s spare capacity, which it extends. Returns the bytes read.
    pub fn read_into(&self, buf: &mut Vec<u8>) -> Option<usize> {
        let spare = buf.spare_capacity_mut();
        // SAFETY: `spare` is valid for writes of its length.
        let n =
            ok(unsafe { syscall(nr::READ, [self.0, spare.as_mut_ptr() as usize, spare.len(), 0, 0, 0]) })?;
        // SAFETY: the kernel initialised the first `n` spare bytes.
        unsafe { buf.set_len(buf.len() + n) };
        Some(n)
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: closing a descriptor we own.
        unsafe { syscall(nr::CLOSE, [self.0, 0, 0, 0, 0, 0]) };
    }
}

/// Reads up to `max` bytes of a file's start, in one `read`. Buffers are not zeroed
/// first, and the allocator gives back what the read did not use.
fn read_start(path: &CStr, max: usize) -> Option<Vec<u8>> {
    let fd = Fd::open(path)?;
    let mut buf = Vec::with_capacity(max);
    fd.read_into(&mut buf)?;
    buf.shrink_to_fit();
    Some(buf)
}

/// Reads a small file whole, in one `read`. Proc and sys files report a size of 0 but
/// deliver everything that fits; `None` if the file is bigger than the buffer.
pub fn read_small(path: &CStr) -> Option<String> {
    const MAX: usize = 8192;
    read_start(path, MAX).filter(|buf| buf.len() < MAX).and_then(|buf| String::from_utf8(buf).ok())
}

/// Reads up to `max` bytes of a file's start, in one `read`.
pub fn read_prefix(path: &CStr, max: usize) -> Option<Vec<u8>> {
    read_start(path, max)
}

const STREAM: usize = 16384;

/// Calls `f` with each line of a file of any size, streamed through a fixed buffer, until
/// `f` returns false. Lines longer than the buffer are skipped.
pub fn for_each_line(path: &CStr, mut f: impl FnMut(&[u8]) -> bool) {
    let Some(fd) = Fd::open(path) else { return };
    let mut buf = Vec::with_capacity(STREAM);
    let mut overlong = false;
    loop {
        if fd.read_into(&mut buf).is_none_or(|n| n == 0) {
            if !buf.is_empty() && !overlong {
                f(&buf);
            }
            return;
        }
        let mut start = 0;
        while let Some(i) = buf[start..].iter().position(|&b| b == b'\n') {
            if !overlong && !f(&buf[start..start + i]) {
                return;
            }
            overlong = false;
            start += i + 1;
        }
        buf.drain(..start);
        if buf.len() == STREAM {
            buf.clear();
            overlong = true;
        }
    }
}

/// Counts the non-overlapping occurrences of `needle` in a file of any size.
pub fn count_in_file(path: &CStr, needle: &[u8]) -> usize {
    let Some(fd) = Fd::open(path) else { return 0 };
    let mut buf = Vec::with_capacity(STREAM);
    let mut count = 0;
    // Carrying the last `needle.len() - 1` bytes finds matches across reads, and that
    // tail is too short to hold a match already counted.
    let keep = needle.len().saturating_sub(1);
    while fd.read_into(&mut buf).is_some_and(|n| n > 0) {
        let mut i = 0;
        while let Some(at) = buf[i..].windows(needle.len()).position(|w| w == needle) {
            count += 1;
            i += at + needle.len();
        }
        buf.drain(..buf.len().saturating_sub(keep).max(i));
    }
    count
}

/// Calls `f` with the name and `d_type` of each entry in a directory.
pub fn for_each_entry(path: &CStr, mut f: impl FnMut(&[u8], u8)) {
    let Some(fd) = Fd::open(path) else { return };
    // On the heap: this much stack would cost a fresh page even when the directory is never
    // listed. u64s, so the records inside are 8-byte aligned.
    let mut buf = Vec::<u64>::with_capacity(512);
    let size = buf.capacity() * size_of::<u64>();
    loop {
        // SAFETY: `buf` is valid for writes of `size` bytes.
        let ret = unsafe { syscall(nr::GETDENTS64, [fd.0, buf.as_mut_ptr() as usize, size, 0, 0, 0]) };
        let Some(n) = ok(ret).filter(|&n| n > 0) else { return };
        // SAFETY: the kernel initialised the first `n` bytes.
        let bytes = unsafe { core::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), n) };
        // linux_dirent64: ino u64, off i64, reclen u16, type u8, then the NUL-terminated name.
        let mut at = 0;
        while at + 19 < n {
            let reclen = usize::from(u16::from_ne_bytes([bytes[at + 16], bytes[at + 17]]));
            let Some(record) = bytes.get(at..at + reclen).filter(|r| r.len() > 19) else { return };
            let name = &record[19..];
            f(&name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())], record[18]);
            at += reclen;
        }
    }
}

pub const DT_DIR: u8 = 4;

pub struct Stat {
    pub nlink: u32,
    pub is_dir: bool,
}

pub fn stat(path: &CStr) -> Option<Stat> {
    #[repr(C, align(8))]
    struct Statx([u8; 256]);
    const STATX_TYPE: usize = 1;
    const STATX_NLINK: usize = 4;
    let mut buf = Statx([0; 256]);
    // SAFETY: `path` is NUL-terminated and `buf` is a struct statx's size and alignment.
    let ret = unsafe {
        syscall(
            nr::STATX,
            [
                AT_FDCWD as usize,
                path.as_ptr() as usize,
                0,
                STATX_TYPE | STATX_NLINK,
                buf.0.as_mut_ptr() as usize,
                0,
            ],
        )
    };
    ok(ret)?;
    // struct statx: stx_nlink is a u32 at byte 16, stx_mode a u16 at byte 28.
    let nlink = u32::from_ne_bytes(buf.0[16..20].try_into().ok()?);
    let mode = u16::from_ne_bytes(buf.0[28..30].try_into().ok()?);
    Some(Stat { nlink, is_dir: mode & 0o170000 == 0o040000 })
}

/// The host name, kernel release and machine, from one `uname`.
pub fn uname() -> Option<[String; 3]> {
    let mut buf = [0u8; 6 * 65];
    // SAFETY: `buf` is a struct utsname's size: six 65-byte fields.
    ok(unsafe { syscall(nr::UNAME, [buf.as_mut_ptr() as usize, 0, 0, 0, 0, 0]) })?;
    let field = |i: usize| {
        let f = &buf[i * 65..(i + 1) * 65];
        String::from_utf8_lossy(&f[..f.iter().position(|&b| b == 0).unwrap_or(65)]).into_owned()
    };
    Some([field(1), field(2), field(4)])
}

/// Seconds since boot, suspend included: the clock behind `/proc/uptime`.
pub fn uptime_secs() -> Option<u64> {
    const CLOCK_BOOTTIME: usize = 7;
    let mut ts = [0i64; 2];
    // SAFETY: `ts` is a 64-bit struct timespec.
    ok(unsafe { syscall(nr::CLOCK_GETTIME, [CLOCK_BOOTTIME, ts.as_mut_ptr() as usize, 0, 0, 0, 0]) })?;
    u64::try_from(ts[0]).ok()
}

pub fn getppid() -> u32 {
    // SAFETY: getppid takes no arguments and cannot fail.
    unsafe { syscall(nr::GETPPID, [0; 6]) as u32 }
}

/// Fresh zeroed pages for the allocator, or null.
pub fn mmap(len: usize) -> *mut u8 {
    const PROT_READ_WRITE: usize = 3;
    const MAP_PRIVATE_ANONYMOUS: usize = 0x22;
    // SAFETY: an anonymous mapping touches no existing memory.
    let ret = unsafe { syscall(nr::MMAP, [0, len, PROT_READ_WRITE, MAP_PRIVATE_ANONYMOUS, usize::MAX, 0]) };
    ok(ret).map_or(core::ptr::null_mut(), |addr| addr as *mut u8)
}

/// Writes all of `bytes`, giving up on the first error (such as a closed pipe).
pub fn write_all(fd: usize, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        // SAFETY: `bytes` is valid for reads of its length.
        let Some(n) = ok(unsafe { syscall(nr::WRITE, [fd, bytes.as_ptr() as usize, bytes.len(), 0, 0, 0]) })
        else {
            return;
        };
        bytes = &bytes[n..];
    }
}

/// Writes all of `parts` with `writev`, so they need not be copied into one buffer first,
/// giving up on the first error.
pub fn write_all_vectored(fd: usize, parts: &[&[u8]]) {
    #[repr(C)]
    struct IoVec {
        base: *const u8,
        len: usize,
    }
    const IOV_MAX: usize = 1024;
    let mut iov: Vec<IoVec> =
        parts.iter().filter(|p| !p.is_empty()).map(|p| IoVec { base: p.as_ptr(), len: p.len() }).collect();
    let mut first = 0;
    while first < iov.len() {
        let rest = &iov[first..];
        // SAFETY: each entry describes a live slice, and `rest` holds at least the count.
        let ret =
            unsafe { syscall(nr::WRITEV, [fd, rest.as_ptr() as usize, rest.len().min(IOV_MAX), 0, 0, 0]) };
        let Some(mut n) = ok(ret).filter(|&n| n > 0) else { return };
        // Skip what was written, which may end partway through an entry.
        while n > 0 {
            let v = &mut iov[first];
            if n < v.len {
                (v.base, v.len, n) = (v.base.wrapping_add(n), v.len - n, 0);
            } else {
                (n, first) = (n - v.len, first + 1);
            }
        }
    }
}

pub fn exit(code: i32) -> ! {
    // SAFETY: exit_group ends the process and never returns.
    unsafe { syscall(nr::EXIT_GROUP, [code as usize, 0, 0, 0, 0, 0]) };
    unreachable!("exit_group returned")
}

/// The environment block: NULL-terminated pointers to `KEY=value` C strings.
static mut ENVP: *const *const u8 = core::ptr::null();

/// # Safety
/// `envp` must be the process's environment block, which lives as long as the process.
#[cfg_attr(test, allow(dead_code))]
pub unsafe fn init_env(envp: *const *const u8) {
    // SAFETY: called once at startup, before anything reads ENVP.
    unsafe { ENVP = envp };
}

/// An environment variable's raw value, which may be empty.
#[cfg(not(test))]
pub fn var_bytes(key: &str) -> Option<&'static [u8]> {
    let key = key.as_bytes();
    // SAFETY: ENVP is set at startup and its strings live as long as the process.
    let mut entry = unsafe { ENVP };
    if entry.is_null() {
        return None;
    }
    // SAFETY: the block ends with a NULL pointer and each entry with a NUL. Matching
    // stops at the first differing byte, and an entry's NUL differs from every key byte,
    // so no read passes the end of an entry.
    unsafe {
        loop {
            let s = *entry;
            if s.is_null() {
                return None;
            }
            entry = entry.add(1);
            // Nearly every entry differs in its first byte, so test that before looping.
            if *s != key[0] {
                continue;
            }
            let mut i = 1;
            while i < key.len() && *s.add(i) == key[i] {
                i += 1;
            }
            if i == key.len() && *s.add(i) == b'=' {
                return Some(CStr::from_ptr(s.add(i + 1).cast()).to_bytes());
            }
        }
    }
}

/// Tests run under std's runtime, which never hands us the environment block.
#[cfg(test)]
pub fn var_bytes(key: &str) -> Option<&'static [u8]> {
    use std::os::unix::ffi::OsStringExt;
    std::env::var_os(key).map(|v| &*v.into_vec().leak())
}

/// An environment variable's value, if it is set and UTF-8. It may be empty.
pub fn var(key: &str) -> Option<&'static str> {
    var_bytes(key).and_then(|v| core::str::from_utf8(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn c(path: &std::path::Path) -> CString {
        CString::new(path.to_str().unwrap()).unwrap()
    }

    #[test]
    fn small_reads_match_std_up_to_the_buffer() {
        let dir = std::env::temp_dir().join(format!("macho-sys-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for len in [0, 10, 8191, 8192] {
            let path = dir.join(len.to_string());
            fs::write(&path, "x".repeat(len)).unwrap();
            let ours = read_small(&c(&path));
            assert_eq!(read_prefix(&c(&path), 100).unwrap(), "x".repeat(len.min(100)).as_bytes());
            assert_eq!(ours, (len < 8192).then(|| "x".repeat(len)), "{len} bytes");
        }
        fs::remove_dir_all(dir).unwrap();
        assert!(read_small(c"/proc/meminfo").unwrap().contains("MemAvailable:"));
        assert_eq!(read_small(c"/nonexistent/macho"), None);
    }

    #[test]
    fn streaming_counts_match_whole_file_counts() {
        let path = std::env::temp_dir().join(format!("macho-stream-{}", std::process::id()));
        // Long enough to cross several buffer refills, with matches straddling them.
        let text: String =
            (0..5000).map(|i| format!("P:pkg{i}\nV:1.{i}\n<string>installed</string>\n")).collect();
        fs::write(&path, &text).unwrap();
        let p = &c(&path);
        let mut lines = 0;
        for_each_line(p, |l| {
            lines += usize::from(l.starts_with(b"P:"));
            true
        });
        assert_eq!(lines, 5000);
        assert_eq!(count_in_file(p, b"<string>installed</string>"), 5000);
        assert_eq!(count_in_file(p, b"V:1.4999\n"), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn paths_join_and_refuse_nul() {
        assert_eq!(path(&[b"/proc/", b"42", b"/stat"]).unwrap().as_bytes(), b"/proc/42/stat");
        assert_eq!(path(&[b"a\0b"]), None);
    }

    #[test]
    fn directories_list_and_stat() {
        let mut names = Vec::new();
        for_each_entry(c"/proc/self", |name, kind| {
            names.push((String::from_utf8_lossy(name).into_owned(), kind))
        });
        assert!(names.iter().any(|(n, k)| n == "task" && *k == DT_DIR));
        assert!(names.iter().any(|(n, _)| n == "stat"));
        let st = stat(c"/proc/self/task").unwrap();
        assert!(st.is_dir && st.nlink >= 2);
        assert!(!stat(c"/proc/self/stat").unwrap().is_dir);
    }

    #[test]
    fn uname_and_uptime_match_proc() {
        let [host, release, machine] = uname().unwrap();
        let proc = |p| fs::read_to_string(p).unwrap().trim().to_owned();
        assert_eq!(host, proc("/proc/sys/kernel/hostname"));
        assert_eq!(release, proc("/proc/sys/kernel/osrelease"));
        assert_eq!(machine, std::env::consts::ARCH);
        let uptime: f64 = proc("/proc/uptime").split(' ').next().unwrap().parse().unwrap();
        assert!((uptime_secs().unwrap() as f64 - uptime).abs() <= 1.0);
        assert_eq!(getppid(), std::os::unix::process::parent_id());
    }
}
