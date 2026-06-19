#![cfg_attr(all(not(test), target_os = "none"), no_std)]
#![cfg_attr(all(not(test), target_os = "none"), feature(alloc_error_handler))]

//! Spider-rs userspace PID 1 service manager for GNU/Mirage.
//!
//! Spider-rs is intentionally userspace. The Supervisor authorizes its launch,
//! the userspace ELF loader validates/maps it, and MTSS schedules it as PID 1.
//! The kernel must never call Spider-rs as a Rust function.

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(target_os = "none")]
mod allocator {
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;
    use core::ptr::null_mut;
    use core::sync::atomic::{AtomicUsize, Ordering};

    const HEAP_SIZE: usize = 128 * 1024;

    struct BootstrapHeap(UnsafeCell<[u8; HEAP_SIZE]>);

    unsafe impl Sync for BootstrapHeap {}

    pub struct BootstrapAllocator {
        heap: BootstrapHeap,
        next: AtomicUsize,
    }

    impl BootstrapAllocator {
        pub const fn new() -> Self {
            Self {
                heap: BootstrapHeap(UnsafeCell::new([0; HEAP_SIZE])),
                next: AtomicUsize::new(0),
            }
        }
    }

    unsafe impl GlobalAlloc for BootstrapAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let align = layout.align().max(1);
            let size = layout.size();
            if size == 0 {
                return align as *mut u8;
            }

            let mut current = self.next.load(Ordering::Relaxed);
            loop {
                let aligned = (current + align - 1) & !(align - 1);
                let Some(end) = aligned.checked_add(size) else {
                    return null_mut();
                };
                if end > HEAP_SIZE {
                    return null_mut();
                }
                match self.next.compare_exchange(
                    current,
                    end,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let heap = (*self.heap.0.get()).as_mut_ptr();
                        return heap.add(aligned);
                    }
                    Err(next) => current = next,
                }
            }
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // Early Spider userspace uses a monotonic bootstrap heap. Long-lived
            // daemons keep allocations for their process lifetime until Mirage
            // grows a userspace heap service or mmap-backed allocator.
        }
    }

    #[global_allocator]
    static GLOBAL_ALLOCATOR: BootstrapAllocator = BootstrapAllocator::new();

    #[alloc_error_handler]
    fn alloc_error(_layout: Layout) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
}

pub mod log;
pub mod service;
pub mod start;
pub mod syscall;
pub mod target;
pub mod units;

pub mod graph;
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod manager;
pub mod parser;
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub mod process;

pub use graph::{DependencyError, StartupPlan};
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use manager::{ServiceOutcome, SpiderManager};
pub use parser::{parse_unit, UnitParseError};
#[cfg(all(feature = "host-tools", not(target_os = "none")))]
pub use process::{Pid, ProcessSpawner, SpawnError, StubSpawner};

pub use units::{default_units, UnitDescriptor, UnitKind, UnitState};
