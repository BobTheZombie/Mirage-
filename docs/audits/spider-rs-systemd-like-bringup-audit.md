# Spider-rs systemd-like bringup audit

Audit findings:

- Existing boot code already had a RuntimeVfs-backed `/spider-rt/sbin/spider-rs` path and MTSS PID1 runnable admission.
- Existing userspace code contained a host-only parser/manager but PID1 printed terminal output directly, which violated the architecture split.
- This update separates bootstrap binaries in `/spider-rt/sbin` from normal userland in `/usr/bin`.
- Added `spider-rsd` as the dispatcher binary and `m1-terminal` as the first normal application.
- Added real unit text for `default.target`, `basic.target`, and `m1-terminal.service`; the parser rejects malformed critical service fields.
- Current blocker: no completed architecture user-mode transition/syscall dispatch, so boot must stop honestly at PID1 runnable and keep spider-rsd/system dispatcher/m1-terminal pending.
