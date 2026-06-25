# Mirage vs. Zinnia Reference Notes

Zinnia is a useful Rust OS reference for Limine handoff normalization, initramfs/module discovery, scheduler queues, syscall dispatch, VFS bootstrap, driver registration, and logging sinks. Mirage must not become Zinnia.

## License/provenance

The inspected Zinnia repository is GPL-2.0 at commit `9c964e184874eee4ef05a7bb10200b06915e0dad`. Mirage changes derived from this audit are independent reimplementations of general OS patterns. No Zinnia code was copied.

## Key differences to preserve

| Area | Zinnia reference | Mirage rule |
| --- | --- | --- |
| Boot payload | Limine files become initramfs or RAM disks | `/spider-rt` bootstrap runtime remains mandatory and separate from normal rootfs |
| Init | command-line/default init path | `/spider-rt/sbin/spider-rs` is PID1 and launches `spider-rsd` |
| Scheduler | kernel scheduler owns process/task queues | MTSS owns portable scheduling and lifecycle; kernel owns CPU mechanisms |
| Policy | kernel-integrated POSIX policy | Supervisor owns launch authorization, service policy, recovery, and capabilities |
| Drivers | loadable modules and in-kernel device model | supervised driver services are preferred; kernel drivers are limited to early/core mechanisms |
| UI/logging | log sinks and panic output | serial may be verbose; framebuffer milestone UI stays concise |

## Adopted Mirage-native improvements

* ELF loader preflight now rejects overlapping PT_LOAD page ranges before userspace admission.
* PID1 initial stack metadata uses the real mounted `/spider-rt/sbin/spider-rs` path.
* Boot Runtime validation explicitly requires a flagged entry plus `spider-rs` and `spider-rsd` lookups through mounted paths.

## Non-adopted areas

Mirage did not adopt Zinnia's initramfs policy, scheduler implementation, syscall ABI, VFS implementation, module loader, USB/xHCI code, or logging code. These remain Mirage-native work items.
