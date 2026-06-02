//! Backward-compatible facade for Rust runtime C ABI exports.

pub use crate::libc::stdlib::{
    aligned_alloc, calloc, free, malloc, memalign, mmap, munmap, posix_memalign, realloc,
    reallocarray,
};
pub use crate::libc::string::{
    bcmp, bcopy, bzero, memchr, memcmp, memcpy, memmove, memset, strcat, strchr, strcmp, strcpy,
    strdup, strlen, strncat, strncmp, strncpy, strndup, strnlen, strrchr, strstr,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::memory::{PROT_READ, PROT_WRITE};
    use core::ffi::{c_char, c_int, c_void};
    use core::ptr;
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

    #[test]
    fn calloc_zeroes_memory() {
        unsafe {
            let ptr = calloc(4, 8) as *mut u8;
            assert!(!ptr.is_null());
            for i in 0..32 {
                assert_eq!(*ptr.add(i), 0);
            }
            free(ptr as *mut c_void);
        }
    }

    #[test]
    fn realloc_grows_buffer() {
        unsafe {
            let ptr = malloc(16) as *mut u8;
            assert!(!ptr.is_null());
            for i in 0..16 {
                *ptr.add(i) = i as u8;
            }
            let new_ptr = realloc(ptr as *mut c_void, 64) as *mut u8;
            assert!(!new_ptr.is_null());
            for i in 0..16 {
                assert_eq!(*new_ptr.add(i), i as u8);
            }
            free(new_ptr as *mut c_void);
        }
    }

    #[test]
    fn aligned_alloc_alignment() {
        unsafe {
            let ptr = aligned_alloc(64, 128);
            assert!(!ptr.is_null());
            assert_eq!((ptr as usize) % 64, 0);
            free(ptr);
        }
    }

    #[test]
    fn posix_memalign_returns_aligned_pointer() {
        unsafe {
            let mut out: *mut c_void = ptr::null_mut();
            let result = posix_memalign(&mut out as *mut _, 32, 128);
            assert_eq!(result, 0);
            assert!(!out.is_null());
            assert_eq!((out as usize) % 32, 0);
            free(out);
        }
    }

    #[test]
    fn memalign_allows_non_multiple_size() {
        unsafe {
            let ptr = memalign(32, 48);
            assert!(!ptr.is_null());
            assert_eq!((ptr as usize) % 32, 0);
            free(ptr);
        }
    }

    #[test]
    fn strdup_clones_input() {
        let original = c_str(b"gcc");
        unsafe {
            let dup = strdup(original.as_ptr());
            assert!(!dup.is_null());
            assert_eq!(strcmp(dup, original.as_ptr()), 0);
            free(dup as *mut c_void);
        }
    }

    #[test]
    fn strndup_respects_max_length() {
        let original = c_str(b"compiler");
        unsafe {
            let dup = strndup(original.as_ptr(), 4);
            assert!(!dup.is_null());
            assert_eq!(strlen(dup), 4);
            assert_eq!(*dup.add(4), 0);
            free(dup as *mut c_void);
        }
    }
}
