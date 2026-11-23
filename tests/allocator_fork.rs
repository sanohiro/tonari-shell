#![cfg(all(target_os = "linux", target_env = "musl"))]

#[path = "../src/allocator.rs"]
mod allocator;

use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn aligned_buffers_keep_their_bytes_when_resized_across_heaps() {
    use std::alloc::{Layout, alloc_zeroed, dealloc, realloc};

    let small = Layout::from_size_align(64 * 1024, 8192).unwrap();
    let large = Layout::from_size_align(2 * 1024 * 1024, 8192).unwrap();
    for (from, to) in [(small, large), (large, small)] {
        unsafe {
            let ptr = alloc_zeroed(from);
            assert!(!ptr.is_null());
            let keep = from.size().min(to.size());
            let prefix = std::slice::from_raw_parts_mut(ptr, keep);
            assert!(prefix.iter().all(|&b| b == 0));
            prefix.fill(0xab);
            let resized = realloc(ptr, from, to.size());
            assert!(!resized.is_null());
            assert_eq!(resized.align_offset(to.align()), 0);
            assert!(
                std::slice::from_raw_parts(resized, keep)
                    .iter()
                    .all(|&b| b == 0xab)
            );
            dealloc(resized, to);
        }
    }
}

#[test]
fn children_can_allocate_while_parent_threads_use_the_heap() {
    let running = AtomicBool::new(true);
    let ready = Barrier::new(5);
    let passed = std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                ready.wait();
                while running.load(Ordering::Relaxed) {
                    std::hint::black_box(vec![1u8; std::hint::black_box(8192)]);
                }
            });
        }
        ready.wait();
        let passed = (0..128).all(|_| {
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                // A lock held by a vanished thread must fail this test promptly.
                unsafe { libc::alarm(3) };
                let bytes = vec![42u8; std::hint::black_box(32768)];
                unsafe { libc::_exit(i32::from(std::hint::black_box(bytes)[0] != 42)) };
            }
            let mut status = -1;
            pid > 0 && unsafe { libc::waitpid(pid, &mut status, 0) } == pid && status == 0
        });
        running.store(false, Ordering::Relaxed);
        passed
    });
    assert!(passed);
}
