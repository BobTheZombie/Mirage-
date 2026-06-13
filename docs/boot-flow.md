# Boot flow

```text
bootloader -> seed-rs -> kernel core -> Mirage-dispatch-rs
           -> SupervisorCreated -> RuntimeVfs /spider-rt
           -> Userspace Loader -> MTSS PID 1 task -> Spider-rs
```

Spider-rs is never invoked as a kernel Rust function. If ELF validation, address-space mapping, MTSS scheduling, or ring-3 entry is incomplete, boot must report Spider-rs as Stub/Failed with the exact reason and must not fake Online.
