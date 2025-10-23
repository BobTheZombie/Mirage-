use core::ffi::{c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::kernel::memory::{self, MemoryProtection};

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest_bytes = dest as *mut u8;
    let src_bytes = src as *const u8;

    let mut i = 0;
    while i < n {
        *dest_bytes.add(i) = *src_bytes.add(i);
        i += 1;
    }

    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest_bytes = dest as *mut u8;
    let src_bytes = src as *const u8;

    if dest_bytes == src_bytes as *mut u8 || n == 0 {
        return dest;
    }

    let dest_addr = dest_bytes as usize;
    let src_addr = src_bytes as usize;
    let src_end = src_addr.saturating_add(n);

    if dest_addr < src_addr || dest_addr >= src_end {
        let mut i = 0;
        while i < n {
            *dest_bytes.add(i) = *src_bytes.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i != 0 {
            i -= 1;
            *dest_bytes.add(i) = *src_bytes.add(i);
        }
    }

    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut c_void, value: c_int, n: usize) -> *mut c_void {
    let dest_bytes = dest as *mut u8;
    let byte = (value & 0xFF) as u8;

    let mut i = 0;
    while i < n {
        *dest_bytes.add(i) = byte;
        i += 1;
    }

    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(lhs: *const c_void, rhs: *const c_void, n: usize) -> c_int {
    let left = lhs as *const u8;
    let right = rhs as *const u8;

    let mut i = 0;
    while i < n {
        let a = *left.add(i);
        let b = *right.add(i);
        if a != b {
            return (a as i32 - b as i32) as c_int;
        }
        i += 1;
    }

    0
}

#[no_mangle]
pub unsafe extern "C" fn memchr(ptr: *const c_void, value: c_int, n: usize) -> *mut c_void {
    let bytes = ptr as *const u8;
    let target = (value & 0xFF) as u8;

    let mut i = 0;
    while i < n {
        if *bytes.add(i) == target {
            return bytes.add(i) as *mut c_void;
        }
        i += 1;
    }

    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn bzero(ptr: *mut c_void, len: usize) {
    memset(ptr, 0, len);
}

#[no_mangle]
pub unsafe extern "C" fn bcopy(src: *const c_void, dest: *mut c_void, len: usize) {
    memmove(dest, src, len);
}

#[no_mangle]
pub unsafe extern "C" fn bcmp(lhs: *const c_void, rhs: *const c_void, len: usize) -> c_int {
    if memcmp(lhs, rhs, len) == 0 {
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0usize;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

#[no_mangle]
pub unsafe extern "C" fn strnlen(s: *const c_char, max_len: usize) -> usize {
    let mut len = 0usize;
    while len < max_len {
        if *s.add(len) == 0 {
            break;
        }
        len += 1;
    }
    len
}

fn to_unsigned(byte: c_char) -> u8 {
    byte as u8
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(lhs: *const c_char, rhs: *const c_char) -> c_int {
    let mut idx = 0usize;
    loop {
        let a = *lhs.add(idx);
        let b = *rhs.add(idx);
        let a_u = to_unsigned(a);
        let b_u = to_unsigned(b);

        if a_u != b_u {
            return (a_u as c_int) - (b_u as c_int);
        }

        if a == 0 {
            return 0;
        }

        idx += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(lhs: *const c_char, rhs: *const c_char, n: usize) -> c_int {
    if n == 0 {
        return 0;
    }

    let mut idx = 0usize;
    while idx < n {
        let a = *lhs.add(idx);
        let b = *rhs.add(idx);
        let a_u = to_unsigned(a);
        let b_u = to_unsigned(b);

        if a_u != b_u {
            return (a_u as c_int) - (b_u as c_int);
        }

        if a == 0 {
            return 0;
        }

        idx += 1;
    }

    0
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut idx = 0usize;
    loop {
        let byte = *src.add(idx);
        *dest.add(idx) = byte;
        if byte == 0 {
            break;
        }
        idx += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut idx = 0usize;
    while idx < n {
        let byte = *src.add(idx);
        *dest.add(idx) = byte;
        idx += 1;
        if byte == 0 {
            while idx < n {
                *dest.add(idx) = 0;
                idx += 1;
            }
            break;
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strcat(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut dest_len = 0usize;
    while *dest.add(dest_len) != 0 {
        dest_len += 1;
    }

    let mut idx = 0usize;
    loop {
        let byte = *src.add(idx);
        *dest.add(dest_len + idx) = byte;
        if byte == 0 {
            break;
        }
        idx += 1;
    }

    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncat(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut dest_len = 0usize;
    while *dest.add(dest_len) != 0 {
        dest_len += 1;
    }

    let mut idx = 0usize;
    while idx < n {
        let byte = *src.add(idx);
        if byte == 0 {
            *dest.add(dest_len + idx) = 0;
            return dest;
        }
        *dest.add(dest_len + idx) = byte;
        idx += 1;
    }

    *dest.add(dest_len + idx) = 0;
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let target = (c & 0xFF) as u8;
    let mut idx = 0usize;

    loop {
        let byte = *s.add(idx) as u8;
        if byte == target {
            return s.add(idx) as *mut c_char;
        }
        if byte == 0 {
            if target == 0 {
                return s.add(idx) as *mut c_char;
            }
            return ptr::null_mut();
        }
        idx += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let target = (c & 0xFF) as u8;
    let mut last: *mut c_char = ptr::null_mut();
    let mut idx = 0usize;

    loop {
        let byte = *s.add(idx) as u8;
        if byte == target {
            last = s.add(idx) as *mut c_char;
        }
        if byte == 0 {
            if target == 0 {
                return s.add(idx) as *mut c_char;
            }
            return last;
        }
        idx += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    if *needle == 0 {
        return haystack as *mut c_char;
    }

    let mut h_idx = 0usize;
    loop {
        let current = *haystack.add(h_idx);
        if current == 0 {
            return ptr::null_mut();
        }

        if current == *needle {
            let mut h_iter = h_idx;
            let mut n_iter = 0usize;

            loop {
                let needle_byte = *needle.add(n_iter);
                if needle_byte == 0 {
                    return haystack.add(h_idx) as *mut c_char;
                }
                let hay_byte = *haystack.add(h_iter);
                if hay_byte != needle_byte {
                    break;
                }
                h_iter += 1;
                n_iter += 1;
            }
        }

        h_idx += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    memory::malloc(size)
        .map(|ptr| ptr.as_ptr() as *mut c_void)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if let Some(non_null) = NonNull::new(ptr as *mut u8) {
        let _ = memory::free(non_null);
    }
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    _addr: *mut c_void,
    length: usize,
    prot: c_int,
    _flags: c_int,
    _fd: c_int,
    _offset: usize,
) -> *mut c_void {
    let protection = MemoryProtection::from_bits(prot as u32);
    memory::mmap(length, protection)
        .map(|region| region.as_ptr() as *mut c_void)
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn munmap(addr: *mut c_void, length: usize) -> c_int {
    match NonNull::new(addr as *mut u8) {
        Some(ptr) => {
            if memory::munmap_ptr(ptr, length) {
                0
            } else {
                -1
            }
        }
        None => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::memory::{PROT_READ, PROT_WRITE};
    use core::ffi::c_char;
    use std::vec::Vec;

    fn c_str(bytes: &[u8]) -> Vec<c_char> {
        let mut v = bytes.iter().map(|b| *b as c_char).collect::<Vec<_>>();
        v.push(0);
        v
    }

    #[test]
    fn memcpy_roundtrip() {
        let src = [1u8, 2, 3, 4, 5];
        let mut dest = [0u8; 5];
        unsafe {
            memcpy(
                dest.as_mut_ptr() as *mut c_void,
                src.as_ptr() as *const c_void,
                src.len(),
            );
        }
        assert_eq!(src, dest);
    }

    #[test]
    fn memmove_overlap() {
        let mut data = [1u8, 2, 3, 4, 5];
        unsafe {
            let ptr = data.as_mut_ptr();
            memmove(ptr.add(1) as *mut c_void, ptr as *const c_void, 4);
        }
        assert_eq!(&data, &[1, 1, 2, 3, 4]);
    }

    #[test]
    fn strlen_counts_bytes() {
        let s = c_str(b"hello");
        unsafe {
            assert_eq!(strlen(s.as_ptr()), 5);
        }
    }

    #[test]
    fn strcmp_orders_strings() {
        let a = c_str(b"apple");
        let b = c_str(b"apricot");
        unsafe {
            assert!(strcmp(a.as_ptr(), b.as_ptr()) < 0);
            assert_eq!(strcmp(a.as_ptr(), a.as_ptr()), 0);
        }
    }

    #[test]
    fn strstr_finds_substring() {
        let hay = c_str(b"kernel security");
        let needle = c_str(b"security");
        unsafe {
            let ptr = strstr(hay.as_ptr(), needle.as_ptr());
            assert!(!ptr.is_null());
            assert_eq!(strlen(ptr), 8);
        }
    }

    #[test]
    fn malloc_roundtrip() {
        unsafe {
            let ptr = malloc(128);
            assert!(!ptr.is_null());
            free(ptr);
        }
    }

    #[test]
    fn mmap_and_munmap_cycle() {
        unsafe {
            let prot = (PROT_READ | PROT_WRITE) as c_int;
            let region = mmap(ptr::null_mut(), 4096, prot, 0, -1, 0);
            assert!(!region.is_null());
            assert_eq!(munmap(region, 4096), 0);
        }
    }
}
