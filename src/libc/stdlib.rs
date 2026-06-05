//! C allocation, conversion, process termination, and environment runtime exports.

use core::ffi::{c_int, c_void};
use core::mem;
use core::ptr;

use crate::kernel::memory::{MemoryProtection, KERNEL_PROCESS_ID};
use crate::kernel::syscall::{
    dispatch_kernel_memory_syscall, SyscallContext, SyscallNumber, SYSCALL_MAX_ARGS,
};

const EINVAL: c_int = 22;
const ENOMEM: c_int = 12;

fn memory_syscall(number: SyscallNumber, args: [u64; SYSCALL_MAX_ARGS]) -> u64 {
    let context = SyscallContext::new(KERNEL_PROCESS_ID, None, args);
    dispatch_kernel_memory_syscall(number, context)
}

fn syscall_malloc(size: usize) -> *mut c_void {
    memory_syscall(SyscallNumber::Malloc, [size as u64, 0, 0, 0, 0, 0]) as *mut c_void
}

fn syscall_free(ptr: *mut c_void) {
    let _ = memory_syscall(SyscallNumber::Free, [ptr as u64, 0, 0, 0, 0, 0]);
}

fn syscall_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    memory_syscall(
        SyscallNumber::Realloc,
        [ptr as u64, size as u64, 0, 0, 0, 0],
    ) as *mut c_void
}

fn syscall_malloc_aligned(size: usize, alignment: usize) -> *mut c_void {
    memory_syscall(
        SyscallNumber::MallocAligned,
        [size as u64, alignment as u64, 0, 0, 0, 0],
    ) as *mut c_void
}

fn syscall_mmap(length: usize, protection: MemoryProtection) -> *mut c_void {
    memory_syscall(
        SyscallNumber::Mmap,
        [length as u64, protection.bits() as u64, 0, 0, 0, 0],
    ) as *mut c_void
}

fn syscall_munmap(addr: *mut c_void, length: usize) -> c_int {
    let result = memory_syscall(
        SyscallNumber::Munmap,
        [addr as u64, length as u64, 0, 0, 0, 0],
    );
    if result == 0 {
        0
    } else {
        -1
    }
}
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total = match nmemb.checked_mul(size) {
        Some(total) => total,
        None => return ptr::null_mut(),
    };

    if total == 0 {
        return ptr::null_mut();
    }

    let block = malloc(total);
    if !block.is_null() {
        ptr::write_bytes(block as *mut u8, 0, total);
    }
    block
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    syscall_realloc(ptr, size)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn reallocarray(ptr: *mut c_void, nmemb: usize, size: usize) -> *mut c_void {
    match nmemb.checked_mul(size) {
        Some(total) => realloc(ptr, total),
        None => ptr::null_mut(),
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    if alignment == 0 || !alignment.is_power_of_two() {
        return ptr::null_mut();
    }
    if alignment < mem::size_of::<usize>() {
        return ptr::null_mut();
    }
    if size == 0 || size % alignment != 0 {
        return ptr::null_mut();
    }
    syscall_malloc_aligned(size, alignment)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn posix_memalign(
    memptr: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> c_int {
    if memptr.is_null() {
        return EINVAL;
    }

    if alignment == 0 || alignment % mem::size_of::<usize>() != 0 || !alignment.is_power_of_two() {
        *memptr = ptr::null_mut();
        return EINVAL;
    }

    if size == 0 {
        *memptr = ptr::null_mut();
        return 0;
    }

    let block = syscall_malloc_aligned(size, alignment);
    if block.is_null() {
        *memptr = ptr::null_mut();
        ENOMEM
    } else {
        *memptr = block;
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memalign(alignment: usize, size: usize) -> *mut c_void {
    if alignment == 0 || !alignment.is_power_of_two() {
        return ptr::null_mut();
    }
    let adjusted = alignment.max(mem::size_of::<usize>());
    if size == 0 {
        return ptr::null_mut();
    }
    syscall_malloc_aligned(size, adjusted)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    syscall_malloc(size)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if !ptr.is_null() {
        syscall_free(ptr);
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mmap(
    _addr: *mut c_void,
    length: usize,
    prot: c_int,
    _flags: c_int,
    _fd: c_int,
    _offset: usize,
) -> *mut c_void {
    let protection = MemoryProtection::from_bits(prot as u32);
    syscall_mmap(length, protection)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, length: usize) -> c_int {
    if addr.is_null() {
        -1
    } else {
        syscall_munmap(addr, length)
    }
}
