# Mirage userspace ELF loader

The initial userspace loader lives at `src/kernel/userspace/elf_loader.rs`. It validates static ELF64 programs before MTSS may create a userspace task for them.

## Supported format

Current validation accepts only:

- ELF magic `0x7fELF`;
- ELFCLASS64;
- little-endian encoding;
- `ET_EXEC`;
- `EM_X86_64`;
- `PT_LOAD` segments;
- canonical user virtual addresses below `0x0000_8000_0000_0000`;
- entry point inside a loadable segment.

The loader records segment mapping math for `p_vaddr`, `p_filesz`, `p_memsz`, and `p_flags`. Dynamic linking is intentionally out of scope for this milestone; Spider-rs must be a static userspace ELF when real loading is wired.

## Memory handoff

`src/kernel/userspace/memory.rs` exposes the minimum API expected by the loader milestone:

```rust
create_user_address_space() -> Result<AddressSpaceId, MmError>
map_user_region(address_space, user_va, phys, len, flags) -> Result<(), MmError>
allocate_user_stack(address_space, size) -> Result<UserStack, MmError>
```

The wrapper enforces user/canonical address checks and requires the user flag. Kernel memory must never be mapped user-accessible.

## Initial stack

`src/kernel/userspace/abi.rs` currently defines a deterministic simplified stack layout:

- `argc = 1`;
- `argv[0] = "/sbin/spider-rs"`;
- empty `envp`;
- null auxv terminator.

This is not a complete System V ABI stack yet, but it is stable and tested.

## Honest boot status

`load_elf_from_file("/sbin/spider-rs")` currently returns `RootFsReadUnavailable` because the root filesystem byte-read API for this loader is not wired. The boot path therefore marks Spider-rs `Stub` with an exact reason instead of claiming userspace execution.
