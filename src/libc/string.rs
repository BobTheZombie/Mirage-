//! C string and memory runtime exports.

use core::cmp;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;

use super::stdlib::malloc;

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn bzero(ptr: *mut c_void, len: usize) {
    memset(ptr, 0, len);
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn bcopy(src: *const c_void, dest: *mut c_void, len: usize) {
    memmove(dest, src, len);
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn bcmp(lhs: *const c_void, rhs: *const c_void, len: usize) -> c_int {
    if memcmp(lhs, rhs, len) == 0 {
        0
    } else {
        1
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0usize;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
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

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return ptr::null_mut();
    }

    let len = strlen(s);
    let Some(total) = len.checked_add(1) else {
        return ptr::null_mut();
    };

    let dest = malloc(total) as *mut c_char;
    if dest.is_null() {
        return ptr::null_mut();
    }
    ptr::copy_nonoverlapping(s, dest, total);
    dest
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strndup(s: *const c_char, n: usize) -> *mut c_char {
    if s.is_null() {
        return ptr::null_mut();
    }

    let len = cmp::min(strnlen(s, n), n);
    let Some(total) = len.checked_add(1) else {
        return ptr::null_mut();
    };

    let dest = malloc(total) as *mut c_char;
    if dest.is_null() {
        return ptr::null_mut();
    }
    ptr::copy_nonoverlapping(s, dest, len);
    *dest.add(len) = 0;
    dest
}
