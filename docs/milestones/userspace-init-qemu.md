# Milestone: Userspace Init in QEMU

## Minimum achieved by this pass

- Full audit document added.
- Platform Registry query helpers added.
- Duplicate PCI rescans removed from AMD platform decisions.
- Boot phases remain honest: Userspace Loader and Spider-rs are Stub until real userspace entry exists.
- `make qemu-spider` stages a no_std Spider-rs PID1 ELF and invokes the existing QEMU boot path while keeping Spider-rs honest Stub until ring-3 entry exists.

## Target remaining

- Load `/sbin/spider-rs` from root FS.
- Map ELF PT_LOAD segments into user memory.
- Schedule PID 1 on MTSS.
- Enter ring 3 and service `write/getpid/yield/exit` syscalls.
- Mark Spider-rs Online only after userspace prints `Spider-rs PID 1 online`.
