//! Deterministic initial userspace stack metadata for the Spider-rs milestone.

use super::memory::VirtAddr;

pub const SPIDER_INIT_PATH: &str = "/sbin/spider-rs";
pub const INITIAL_STACK_WORDS: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialStackLayout {
    pub argc: usize,
    pub argv0_ptr: VirtAddr,
    pub argv_null: usize,
    pub envp_null: usize,
    pub auxv_null: usize,
    pub stack_pointer: VirtAddr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackLayoutError {
    StackTooSmall,
    NonCanonicalStack,
}

pub fn build_initial_stack_layout(
    stack_bottom: VirtAddr,
    stack_top: VirtAddr,
) -> Result<InitialStackLayout, StackLayoutError> {
    if stack_top.0 <= stack_bottom.0 || stack_top.0 >= 0x0000_8000_0000_0000 {
        return Err(StackLayoutError::NonCanonicalStack);
    }
    let string_bytes = SPIDER_INIT_PATH.len() + 1;
    let words_bytes = INITIAL_STACK_WORDS * core::mem::size_of::<u64>();
    let total = string_bytes + words_bytes;
    if stack_top.0 - stack_bottom.0 < total as u64 {
        return Err(StackLayoutError::StackTooSmall);
    }
    let argv0 = stack_top.0 - string_bytes as u64;
    let sp = (argv0 - words_bytes as u64) & !0xf;
    Ok(InitialStackLayout {
        argc: 1,
        argv0_ptr: VirtAddr(argv0),
        argv_null: 0,
        envp_null: 0,
        auxv_null: 0,
        stack_pointer: VirtAddr(sp),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_stack_layout_is_deterministic() {
        let layout =
            build_initial_stack_layout(VirtAddr(0x7000_0000), VirtAddr(0x7000_4000)).unwrap();
        assert_eq!(layout.argc, 1);
        assert_eq!(layout.argv_null, 0);
        assert_eq!(layout.envp_null, 0);
        assert_eq!(layout.auxv_null, 0);
        assert_eq!(layout.stack_pointer.0 & 0xf, 0);
    }
}
