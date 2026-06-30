# QEMU PID1 Boot

Use the explicit full-boot path when testing `spider-rs` PID1:

```sh
make qemu-kernel-full
make iso
MIRAGE_FULL_BOOT=1 MIRAGE_REUSE_IMAGE=0 MIRAGE_ISO_IMAGE=build/mirage.iso tools/run-qemu.sh
```

Expected pre-ring3 markers include `BOOT MODE [FULL-BOOT]`, `BOOT RUNTIME [OK]`, `ROOT FS [OK]`, `Supervisor [Ok]`, `MTSS CORE [READY]`, `MTSS SCHEDULER [READY]`, `PID1 HANDOFF [ALLOWED`, `SPIDER-RS ELF [OK]`, `SPIDER-RS PID1 [CREATED]`, and `SPIDER-RS PID1 [RUNNABLE]`.

Do not use non-full-boot output as PID1 evidence. Non-full-boot prints `BOOT MODE [NON-FULL-BOOT: PID1 disabled]`.
