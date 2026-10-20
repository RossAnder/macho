//! A bump allocator over a static arena. A fetch allocates a few kilobytes and exits,
//! so freeing is mostly wasted work. Only the newest allocation can grow, shrink or be
//! freed in place, which covers growing strings and scratch buffers; anything that
//! doesn't fit gets fresh pages.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sys;

const SIZE: usize = 64 * 1024;

#[repr(C, align(16))]
struct Arena(UnsafeCell<[u8; SIZE]>);

// SAFETY: the atomic cursor hands each allocation a range no other allocation shares.
unsafe impl Sync for Arena {}

static ARENA: Arena = Arena(UnsafeCell::new([0; SIZE]));
static NEXT: AtomicUsize = AtomicUsize::new(0);

pub struct Bump;

fn base() -> usize {
    ARENA.0.get() as usize
}

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut next = NEXT.load(Ordering::Relaxed);
        loop {
            let start = (base() + next).next_multiple_of(layout.align()) - base();
            let end = start + layout.size();
            if end > SIZE {
                // Pages come page-aligned, which covers any alignment a fetch asks for.
                return sys::mmap(layout.size());
            }
            match NEXT.compare_exchange_weak(next, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (base() + start) as *mut u8,
                Err(now) => next = now,
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let start = (ptr as usize).wrapping_sub(base());
        if start < SIZE {
            // Freeing the newest allocation winds the cursor back over it.
            let _ = NEXT.compare_exchange(start + layout.size(), start, Ordering::Relaxed, Ordering::Relaxed);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let start = (ptr as usize).wrapping_sub(base());
        let old_end = start + layout.size();
        if start < SIZE && start + new_size <= SIZE {
            // The newest allocation ends at the cursor, so it can grow without moving.
            if NEXT.compare_exchange(old_end, start + new_size, Ordering::Relaxed, Ordering::Relaxed).is_ok()
            {
                return ptr;
            }
        }
        // SAFETY: the caller's guarantees for `layout` carry over to the new size.
        let new = unsafe { self.alloc(Layout::from_size_align_unchecked(new_size, layout.align())) };
        if !new.is_null() {
            // SAFETY: both blocks are valid for the smaller of the two sizes and distinct.
            unsafe { core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size)) };
        }
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_aligned_and_disjoint() {
        let layouts = [Layout::new::<u8>(), Layout::new::<u64>(), Layout::from_size_align(24, 16).unwrap()];
        let ptrs = layouts.map(|layout| (unsafe { Bump.alloc(layout) }, layout));
        for (i, &(ptr, layout)) in ptrs.iter().enumerate() {
            assert_eq!(ptr as usize % layout.align(), 0);
            for &(other, other_layout) in &ptrs[i + 1..] {
                let (a, b) = (ptr as usize, other as usize);
                assert!(a + layout.size() <= b || b + other_layout.size() <= a, "allocations overlap");
            }
        }
    }

    #[test]
    fn oversized_allocations_get_their_own_pages() {
        let layout = Layout::from_size_align(SIZE + 1, 8).unwrap();
        let ptr = unsafe { Bump.alloc(layout) };
        assert!(!ptr.is_null() && !(base()..base() + SIZE).contains(&(ptr as usize)));
        unsafe { ptr.add(SIZE).write(1) };
    }

    #[test]
    fn the_newest_allocation_frees_and_shrinks_in_place() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let a = unsafe { Bump.alloc(layout) };
        unsafe { Bump.dealloc(a, layout) };
        assert_eq!(unsafe { Bump.alloc(layout) }, a);
        let b = unsafe { Bump.realloc(a, layout, 16) };
        assert_eq!(b, a);
        let next = unsafe { Bump.alloc(layout) };
        assert!(next as usize <= a as usize + 16 + 8, "the shrunk tail was not reused");
    }

    #[test]
    fn growth_keeps_contents() {
        let mut s = String::new();
        for i in 0..20_000 {
            s.push(char::from(b'a' + (i % 26) as u8));
        }
        assert!(s.bytes().enumerate().all(|(i, b)| b == b'a' + (i % 26) as u8));
    }
}
