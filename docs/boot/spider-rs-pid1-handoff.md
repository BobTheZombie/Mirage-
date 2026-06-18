# Spider-rs PID1 Handoff

Spider-rs is the first Mirage userspace daemon. It is PID1, the system dispatcher, the service root, and the daemon that will later launch the first terminal/session application. It is not the hello-world app.

The intended chain is:

```text
kernel -> supervisor -> MTSS -> userspace loader -> spider-rs PID1 -> mirage-m1-terminal
```

## Dependency gate

The loader may attempt PID1 only when all authoritative boot dependencies are true:

- root FS is online
- supervisor is online
- MTSS is online
- RuntimeVfs is mounted and exposes `/spider-rt/sbin/spider-rs`

Once MTSS reports online, the old message “MTSS handoff not reached yet” is stale. The remaining missing stage is PID1 creation, MTSS task admission, dispatcher entry, or user-mode transition.

## Implemented milestones

The current implementation performs these real steps:

1. starts the userspace loader
2. reads `/spider-rt/sbin/spider-rs` from RuntimeVfs
3. validates ELF64 magic, class, endianness, type, x86_64 machine, entry point, program headers, and loadable segments
4. asks the supervisor to authorize Spider-rs as PID1
5. creates a kernel process record
6. admits the PID1 userspace task through MTSS and marks it runnable only after MTSS accepts it
7. records the dispatcher as pending because ring-3 transition is not implemented yet

## Pending

The ring-3/user-mode transition is still pending, so the boot status must not claim `SPIDER-RS [ONLINE]`, `USERSpace [ONLINE]`, or `SYSTEM DISPATCHER [ONLINE]`. The honest terminal state is:

```text
PID1              [ RUNNABLE ]
SYSTEM DISPATCHER [ PENDING: user-mode transition not implemented ]
M1 TERMINAL       [ PENDING: dispatcher child launch not implemented ]
```
