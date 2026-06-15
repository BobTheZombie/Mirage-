# Supervisor -> MTSS -> Spider-rs PID 1 bring-up

This patch wires the first userspace init path through the architecture Mirage is moving toward:

```text
RuntimeVfs /spider-rt/sbin/spider-rs
    -> Supervisor policy authorization
    -> kernel userspace ELF validation
    -> MTSS-backed PID 1 admission
    -> scheduler/architecture user-entry path
```

## What changed

- `src/supervisor/pid1.rs` adds the Supervisor-owned Spider-rs launch policy.
- `src/kernel/spider_pid1.rs` adds an explicit MTSS-facing Spider-rs PID 1 admission shim.
- `src/main.rs` is patched so boot code no longer calls `kernel.bootstrap_spider_rs_pid1_from_image(...)` directly.
- `userspace/spider-rs/src/syscall.rs` is patched to stop using stale syscall numbers.

## Important honesty boundary

This patch proves and logs the correct authority path:

```text
Supervisor authorized Spider-rs -> kernel/ELF loader accepted it -> MTSS PID 1 admission path returned a PID
```

It still does **not** claim a fully healthy userland until the architecture backend proves ring-3 execution and Spider-rs successfully traps back through a syscall.  The boot phase message intentionally says that userspace confirmation is pending.

## Expected boot log direction

Look for lines like:

```text
MTSS initialized
Userspace Loader Online: read /spider-rt/sbin/spider-rs (... bytes)
Supervisor authorized Spider-rs PID 1 via MTSS: pid=... entry=... bytes=...
```

## Next milestone

The next clean patch after this one should add a positive confirmation path:

```text
Spider-rs write("Spider-rs PID 1 online") syscall
    -> kernel validates the caller is PID 1
    -> boot_phase_online(SpiderRs)
    -> boot_phase_online(Userspace)
```

That final step is the proper no-fake-Online proof.
