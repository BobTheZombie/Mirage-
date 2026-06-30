# Full Boot Mode

`full-boot` enables the rootfs, supervisor, boot runtime validation, MTSS, userspace loader, and PID1 handoff sequence in `src/main.rs`.

Supported explicit build forms:

```sh
make qemu-kernel-full
MIRAGE_FULL_BOOT=1 make qemu-kernel
```

A full-boot kernel prints `BOOT MODE [FULL-BOOT]` early. A non-full-boot kernel prints `BOOT MODE [NON-FULL-BOOT: PID1 disabled]` and must not claim PID1 execution.
