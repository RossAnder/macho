//! What a C runtime would otherwise supply: the process entry point, the `mem*`
//! functions the compiler emits calls to, and a panic handler. build.rs links with
//! `-nostartfiles -nostdlib`, so nothing else runs before `entry`.

use core::ffi::{c_char, c_void};
use core::fmt::{self, Write};

use macho::sys;

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    // The kernel leaves argc at the stack pointer; pass that address on, with the stack
    // aligned as the C ABI expects at a call.
    core::arch::naked_asm!("xor ebp, ebp", "mov rdi, rsp", "and rsp, -16", "call {entry}", "ud2", entry = sym entry)
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    // The kernel leaves argc at the (already 16-byte aligned) stack pointer.
    core::arch::naked_asm!("mov x29, xzr", "mov x30, xzr", "mov x0, sp", "bl {entry}", "brk #0", entry = sym entry)
}

/// # Safety
/// `stack` must be the initial stack pointer: argc, argv's pointers and a NULL, then
/// the environment's pointers and a NULL.
unsafe extern "C" fn entry(stack: *const usize) -> ! {
    // SAFETY: the layout above is what the kernel builds for every new process.
    let (argc, argv) = unsafe { (*stack, stack.add(1).cast::<*const u8>()) };
    unsafe { sys::init_env(argv.add(argc + 1)) };
    // SAFETY: argv holds argc NUL-terminated strings that live as long as the process.
    let args = (1..argc).map(|i| unsafe { core::ffi::CStr::from_ptr((*argv.add(i)).cast()) }.to_bytes());
    sys::exit(macho::run(args))
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    struct Stderr;
    impl Write for Stderr {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            sys::write_all(2, s.as_bytes());
            Ok(())
        }
    }
    let _ = writeln!(Stderr, "macho: {}", info.message());
    sys::exit(101)
}

// Landing pads in the prebuilt `alloc` (compiled to unwind) name these two, but nothing
// unwinds: the panic handler exits.

#[unsafe(no_mangle)]
extern "C" fn _Unwind_Resume() -> ! {
    sys::exit(101)
}

#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality() -> ! {
    sys::exit(101)
}

/// compiler_builtins' aarch64 atomics call this from a constructor to look for LSE
/// instructions. Nothing runs constructors here, so they keep to the load/store-exclusive
/// sequences every aarch64 CPU has, and "no capabilities" agrees with that.
#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
extern "C" fn getauxval(_kind: core::ffi::c_ulong) -> core::ffi::c_ulong {
    0
}

// The compiler lowers copies, fills and comparisons to these, so they must not contain
// anything it would turn back into a call to themselves: plain byte loops are out.
// Strings are built from many short copies, so short sizes take a branch or two.

/// Copies `n <= 32` bytes with overlapping unaligned loads, all made before any store,
/// so overlapping ranges are fine.
///
/// # Safety
/// Both ranges must be valid for `n` bytes.
#[inline(always)]
unsafe fn copy_small(d: *mut u8, s: *const u8, n: usize) {
    // SAFETY: every access lies within the first `n` bytes of its range.
    unsafe {
        if n >= 16 {
            let (a, b) = (s.cast::<u128>().read_unaligned(), s.add(n - 16).cast::<u128>().read_unaligned());
            d.cast::<u128>().write_unaligned(a);
            d.add(n - 16).cast::<u128>().write_unaligned(b);
        } else if n >= 8 {
            let (a, b) = (s.cast::<u64>().read_unaligned(), s.add(n - 8).cast::<u64>().read_unaligned());
            d.cast::<u64>().write_unaligned(a);
            d.add(n - 8).cast::<u64>().write_unaligned(b);
        } else if n >= 4 {
            let (a, b) = (s.cast::<u32>().read_unaligned(), s.add(n - 4).cast::<u32>().read_unaligned());
            d.cast::<u32>().write_unaligned(a);
            d.add(n - 4).cast::<u32>().write_unaligned(b);
        } else if n > 0 {
            let (a, b, c) = (*s, *s.add(n / 2), *s.add(n - 1));
            *d = a;
            *d.add(n / 2) = b;
            *d.add(n - 1) = c;
        }
    }
}

/// An empty `asm!` may touch any memory, which stops a loop around it from being
/// recognised as a copy or fill.
#[inline(always)]
fn opaque() {
    // SAFETY: no instructions.
    unsafe { core::arch::asm!("", options(nostack, preserves_flags)) };
}

/// # Safety
/// Both ranges must be valid for `n` bytes, and `d` must not lie inside `s`'s range
/// after its start (a forward copy reads every byte before it can be overwritten).
unsafe fn copy_forward(d: *mut u8, s: *const u8, n: usize) {
    if n <= 32 {
        // SAFETY: as for this function.
        return unsafe { copy_small(d, s, n) };
    }
    #[cfg(target_arch = "x86_64")]
    if n >= 512 {
        // SAFETY: as for this function; rep movsb copies forwards a byte at a time.
        unsafe {
            core::arch::asm!("rep movsb", inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _, options(nostack, preserves_flags));
        }
        return;
    }
    let mut i = 0;
    while n - i > 32 {
        // SAFETY: `i + 32 < n`.
        unsafe { copy_small(d.add(i), s.add(i), 32) };
        opaque();
        i += 32;
    }
    // SAFETY: the remaining `n - i` bytes, at most 32.
    unsafe { copy_small(d.add(i), s.add(i), n - i) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    // SAFETY: the caller guarantees both ranges are valid and disjoint.
    unsafe { copy_forward(dst.cast(), src.cast(), n) };
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst.cast::<u8>(), src.cast::<u8>());
    if n <= 32 || (d as usize).wrapping_sub(s as usize) >= n {
        // SAFETY: short copies load everything first; otherwise `d` is not inside the
        // source after its start.
        unsafe { copy_forward(d, s, n) };
        return dst;
    }
    // Backwards from the end, so no byte is read after it is overwritten. Rare: only
    // shifting data up within a buffer comes here.
    let mut i = n;
    while i > 0 {
        let k = i.min(32);
        i -= k;
        // SAFETY: bytes `i..i + k` of both ranges; all above `i + k` are already copied.
        unsafe { copy_small(d.add(i), s.add(i), k) };
        opaque();
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let d = dst.cast::<u8>();
    let b = u64::from(c as u8) * 0x0101_0101_0101_0101;
    #[cfg(target_arch = "x86_64")]
    if n >= 512 {
        // SAFETY: the caller guarantees the range is valid for writes.
        unsafe {
            core::arch::asm!("rep stosb", inout("rcx") n => _, inout("rdi") d => _, in("al") c as u8, options(nostack, preserves_flags));
        }
        return dst;
    }
    let mut i = 0;
    while n - i >= 16 {
        // SAFETY: `i + 16 <= n`.
        unsafe { d.add(i).cast::<u128>().write_unaligned(u128::from(b) << 64 | u128::from(b)) };
        opaque();
        i += 16;
    }
    let rest = n - i;
    // SAFETY: the last `rest < 16` bytes, written as overlapping pairs.
    unsafe {
        let d = d.add(i);
        if rest >= 8 {
            d.cast::<u64>().write_unaligned(b);
            d.add(rest - 8).cast::<u64>().write_unaligned(b);
        } else if rest >= 4 {
            d.cast::<u32>().write_unaligned(b as u32);
            d.add(rest - 4).cast::<u32>().write_unaligned(b as u32);
        } else if rest > 0 {
            *d = c as u8;
            *d.add(rest / 2) = c as u8;
            *d.add(rest - 1) = c as u8;
        }
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a.cast::<u8>(), b.cast::<u8>());
    let mut i = 0;
    // Skip equal words, then find the first differing byte.
    // SAFETY: the caller guarantees both ranges are valid for reads of `n` bytes.
    while i + 8 <= n
        && unsafe { a.add(i).cast::<u64>().read_unaligned() == b.add(i).cast::<u64>().read_unaligned() }
    {
        i += 8;
    }
    while i < n {
        // SAFETY: as above; volatile keeps this from becoming a call.
        let (x, y) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        i += 1;
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    // SAFETY: forwarded unchanged.
    unsafe { memcmp(a, b, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut n = 0;
    // SAFETY: the caller guarantees a NUL-terminated string.
    while unsafe { s.add(n).read_volatile() } != 0 {
        n += 1;
    }
    n
}
