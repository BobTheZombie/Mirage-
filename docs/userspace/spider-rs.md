# Spider-rs userspace PID 1

Spider-rs is the GNU/Mirage userspace PID 1 service manager. It is systemd-like but Mirage-native: static units and targets come first, dynamic unit parsing can follow later, and all privileged launch authority remains mediated by the Supervisor and capabilities.

Spider-rs lives in `userspace/spider-rs/` and builds as a no_std static userspace ELF for the Mirage target. It runs from `/spider-rt/sbin/spider-rs`, not from the kernel image.

V0 behavior:

1. enter `_start`
2. call `spider_main()`
3. write `Spider-rs PID 1 online` through the syscall ABI
4. load the built-in target table
5. activate `basic.target` then `default.target`
6. enter a yield-based service-manager loop

Spider-rs Online must not be reported until real userspace execution and syscall output are observed.
