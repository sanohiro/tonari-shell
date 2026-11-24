use dlmalloc::GlobalDlmalloc;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
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

// Word expansion, parsing and function calls tend to release and immediately
// request the same small blocks. Bound retained space per thread and return
// anything beyond that limit directly to the shared, fork-safe allocator.
const SIZES: [usize; 10] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512];
const DEPTH: usize = 16;

struct Bin {
    used: Cell<usize>,
    blocks: [Cell<*mut u8>; DEPTH],
}

impl Bin {
    const fn new() -> Self {
        Self {
            used: Cell::new(0),
            blocks: [const { Cell::new(std::ptr::null_mut()) }; DEPTH],
        }
    }

    fn take(&self) -> *mut u8 {
        let n = self.used.get();
        if n == 0 {
            return std::ptr::null_mut();
        }
        self.used.set(n - 1);
        self.blocks[n - 1].get()
    }

    fn put(&self, block: *mut u8) -> bool {
        let n = self.used.get();
        if n == DEPTH {
            return false;
        }
        self.blocks[n].set(block);
        self.used.set(n + 1);
        true
    }
}

struct Cache([Bin; SIZES.len()]);
thread_local! { static CACHE: Cache = const { Cache([const { Bin::new() }; SIZES.len()]) }; }

impl Drop for Cache {
    fn drop(&mut self) {
        for (i, bin) in self.0.iter_mut().enumerate() {
            let layout = rounded(i);
            for entry in &mut bin.blocks[..*bin.used.get_mut()] {
                unsafe { GlobalDlmalloc.dealloc(*entry.get_mut(), layout) };
            }
        }
    }
}

fn class(layout: Layout) -> Option<usize> {
    if layout.align() > 16 {
        return None;
    }
    Some(match layout.size() {
        0..=16 => 0,
        17..=32 => 1,
        33..=48 => 2,
        49..=64 => 3,
        65..=96 => 4,
        97..=128 => 5,
        129..=192 => 6,
        193..=256 => 7,
        257..=384 => 8,
        385..=512 => 9,
        _ => return None,
    })
}

fn rounded(class: usize) -> Layout {
    unsafe { Layout::from_size_align_unchecked(SIZES[class], 16) }
}

// The allocation's layout determines ownership, including after a resize.
unsafe impl GlobalAlloc for ShellHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() >= LARGE {
            return unsafe { System.alloc(layout) };
        }
        prepare();
        if let Some(i) = class(layout) {
            let block = CACHE
                .try_with(|cache| cache.0[i].take())
                .unwrap_or(std::ptr::null_mut());
            if !block.is_null() {
                return block;
            }
            return unsafe { GlobalDlmalloc.alloc(rounded(i)) };
        }
        unsafe { GlobalDlmalloc.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() >= LARGE {
            return unsafe { System.alloc_zeroed(layout) };
        }
        if class(layout).is_some() {
            let block = unsafe { self.alloc(layout) };
            if !block.is_null() {
                unsafe { block.write_bytes(0, layout.size()) };
            }
            return block;
        }
        prepare();
        unsafe { GlobalDlmalloc.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() >= LARGE {
            unsafe { System.dealloc(ptr, layout) }
        } else if let Some(i) = class(layout) {
            if !CACHE.try_with(|cache| cache.0[i].put(ptr)).unwrap_or(false) {
                unsafe { GlobalDlmalloc.dealloc(ptr, rounded(i)) };
            }
        } else {
            unsafe { GlobalDlmalloc.dealloc(ptr, layout) }
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        match (layout.size() >= LARGE, size >= LARGE) {
            (true, true) => unsafe { System.realloc(ptr, layout, size) },
            (false, false) => {
                let next = unsafe { Layout::from_size_align_unchecked(size, layout.align()) };
                let from = class(layout).map_or(layout, rounded);
                let to = class(next).map_or(next, rounded);
                if from == to {
                    return ptr;
                }
                if from.align() == to.align() {
                    return unsafe { GlobalDlmalloc.realloc(ptr, from, to.size()) };
                }
                unsafe { self.move_block(ptr, layout, next) }
            }
            _ => unsafe {
                self.move_block(
                    ptr,
                    layout,
                    Layout::from_size_align_unchecked(size, layout.align()),
                )
            },
        }
    }
}

impl ShellHeap {
    unsafe fn move_block(&self, ptr: *mut u8, old: Layout, next: Layout) -> *mut u8 {
        let dest = unsafe { self.alloc(next) };
        if !dest.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, dest, old.size().min(next.size()));
                self.dealloc(ptr, old);
            }
        }
        dest
    }
}
