# Spider-rs userspace init for GNU/Mirage

Spider-rs is the planned PID 1 userspace init and service manager for GNU/Mirage. It is inspired by systemd's unit graph model, but it is Mirage-native: POSIX/GNU programs see a familiar userspace service environment while Mirage keeps its non-Unix internal architecture.

## What Spider-rs is

Spider-rs manages userspace service policy after the kernel has already reached the userspace handoff point:

```text
Limine
  -> seed-rs
  -> kernel_main
  -> Mirage-dispatch-rs
  -> Kernel Supervisor
  -> userspace loader
  -> /sbin/spider-rs
  -> Spider-rs service graph
  -> userspace services
```

Spider-rs owns userspace-facing init duties:

- loading service and target unit files;
- resolving dependency order;
- tracking service state;
- starting services through a process-spawner ABI;
- providing the future home for sockets, timers, logging hooks, restart policy, and shutdown ordering.

The preferred installed binary path is:

```text
/sbin/spider-rs
```

## What Spider-rs is not

Spider-rs is not part of seed-rs, not part of the kernel early boot path, and not the kernel-internal startup dispatcher.

Spider-rs does **not** manage:

- kernel memory;
- kernel drivers;
- BootInfo;
- the Boot Phase Manager;
- Mirage-dispatch-rs internals;
- kernel Supervisor internals;
- MTSS kernel scheduling internals.

## Boundary with Mirage-dispatch-rs

Mirage-dispatch-rs is kernel-internal. It starts kernel components and subsystems while the kernel is still bringing up privileged mechanisms.

Spider-rs is userspace. It starts only after root filesystem, userspace loader, stdio, and process creation are available. A failure in Spider-rs should be handled as a userspace init/service failure, not as a reason to move userspace policy into the kernel dispatcher.

## Boundary with the kernel Supervisor

The Mirage Supervisor remains the kernel-side policy, security, capability, service-registration, and recovery authority. Spider-rs may request service launches and observe userspace process states, but it must not bypass capability enforcement or replace Supervisor policy.

In the current scaffold, `StubSpawner` deliberately logs intended `ExecStart` commands without executing processes. This avoids falsely claiming that Spider-rs runs as PID 1 before the Mirage process ABI is ready.

## Unit locations

Spider-rs searches these system unit directories:

```text
/etc/spider/system/
/usr/lib/spider/system/
/run/spider/system/
```

When those directories are unavailable in the current milestone, Spider-rs falls back to compiled-in test units:

- `basic.target`
- `multi-user.target`
- `default.target`
- `shell.service`
- `getty.service`

## Unit format

Spider-rs unit files use a Mirage-native INI-like format. Unit names use systemd-like suffixes such as `.service` and `.target`; the project also reserves `.socket`, `.timer`, `.mount`, `.device`, and `.path` for later milestones.

Example service:

```ini
[Unit]
Description=Mirage system logger
After=basic.target
Requires=basic.target

[Service]
ExecStart=/bin/mirage-logd
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Initial supported fields:

- `[Unit] Description=`
- `[Unit] After=` ordering-only dependencies
- `[Unit] Before=` ordering-only dependencies
- `[Unit] Requires=` required dependencies
- `[Unit] Wants=` non-fatal dependencies
- `[Service] ExecStart=` command line to launch later through Mirage process syscalls
- `[Service] Restart=` with `no`, `on-failure`, or `always`
- `[Install] WantedBy=` reverse target membership used by the scaffold resolver

## Boot target flow

The default compiled-in target chain is:

```text
default.target
  -> multi-user.target
  -> basic.target
```

The deterministic startup order starts dependencies before dependents, so the fallback graph reaches `basic.target` before `multi-user.target`, then reaches `default.target` after requested services have been stub-started.

## Dependency semantics

- `Requires=` pulls a unit into the graph and failure propagates to the dependent unit.
- `Wants=` pulls a unit into the graph but failure does not fail the dependent unit.
- `After=` and `Before=` only affect ordering when both units are already in the graph.
- Cycles are detected and reported as graph errors.
- Ordering is deterministic by using sorted unit names during graph resolution.

## Required Mirage kernel ABI for real PID 1 mode

Spider-rs can become real PID 1 only after these userspace/kernel ABI pieces are available and tested.

Process syscalls:

- `spawn` or `execve`
- `waitpid`
- `exit`
- `kill`/signal delivery later
- `getpid`

Filesystem syscalls:

- `open`
- `read`
- `close`
- `getdents`/`readdir`
- `stat`
- `access`

I/O syscalls:

- `write` for stdout/stderr logging
- optional `read` for stdin/console interaction

Time syscalls:

- monotonic clock
- sleep/timer support for later restart policies and timers

Service supervision ABI:

- process exit notification
- restart timers later
- Supervisor-mediated capability grants for services

## Current limitations

The current implementation is a buildable host-side/userspace-target scaffold. It parses units, resolves the dependency graph, prints a startup plan, and uses `StubSpawner` to mark service startup as stubbed. It does not claim to run inside Mirage yet.

The kernel boot phase table includes a separate `Spider-rs` phase so boot screens can show `Spider-rs [ Stub ]` or `Spider-rs [ Pending ]` independently from general userspace. The kernel does not depend on Spider-rs during boot; it merely prefers `/sbin/spider-rs` as the first userspace init candidate once real userspace execution is available.
