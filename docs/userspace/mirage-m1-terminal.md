# Mirage M1.1 terminal app

The first minimal userspace terminal payload is the current no_std `spider-rs` PID1 ELF staged at `/spider-rt/sbin/spider-rs`. For this milestone its `_start` path prints exactly:

```text
Mirage M1.1 System
hello world
```

The app emits those lines through the Mirage userspace `write(fd, buf, len)` shim. File descriptors `1` and `2` are the intended early console outputs. After printing, the app calls the Mirage exit syscall and spins if exit is not yet completed by the kernel.

Current limitation: the kernel validates and admits the ELF as a runnable MTSS task, but the architecture dispatcher does not yet complete the ring-3 transition. Boot therefore reports `Userspace [Started: bootstrap console mode]` and `Dispatcher [Pending: user-mode transition not implemented]` instead of full userspace `Online`.
