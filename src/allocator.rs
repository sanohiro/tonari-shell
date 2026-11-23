use dlmalloc::GlobalDlmalloc;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Once;

struct ShellHeap;

#[global_allocator]
static HEAP: ShellHeap = ShellHeap;

// Keep a history search block in the reusable heap. Larger, transient buffers
// go to musl, which can release their mappings independently after a command.
const LARGE: usize = 256 * 1024;

fn prepare() {
    static INIT: Once = Once::new();
    // On Linux Once uses a futex. The atfork registration uses musl's own
    // allocator, so neither operation re-enters this Rust allocator.
    INIT.call_once(|| unsafe { dlmalloc::enable_alloc_after_fork() });
}

// The allocation's layout determines ownership, including after a resize.
unsafe impl GlobalAlloc for ShellHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() >= LARGE {
            return unsafe { System.alloc(layout) };
        }
        prepare();
        unsafe { GlobalDlmalloc.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() >= LARGE {
            return unsafe { System.alloc_zeroed(layout) };
        }
        prepare();
        unsafe { GlobalDlmalloc.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() >= LARGE {
            unsafe { System.dealloc(ptr, layout) }
        } else {
            unsafe { GlobalDlmalloc.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        match (layout.size() >= LARGE, size >= LARGE) {
            (true, true) => unsafe { System.realloc(ptr, layout, size) },
            (false, false) => unsafe { GlobalDlmalloc.realloc(ptr, layout, size) },
            _ => unsafe {
                let replacement =
                    self.alloc(Layout::from_size_align_unchecked(size, layout.align()));
                if !replacement.is_null() {
                    std::ptr::copy_nonoverlapping(ptr, replacement, layout.size().min(size));
                    self.dealloc(ptr, layout);
                }
                replacement
            },
        }
    }
}
