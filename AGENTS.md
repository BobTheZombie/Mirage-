# AGENTS.md — Mirage Kernel Architecture

## Project Identity

Mirage is a Rust-based experimental operating system architecture designed as a modern GNU-compatible service kernel.

Mirage is **not** Linux, Unix, or a Unix clone internally.

Mirage provides a POSIX/Unix-compatible surface for existing GNU and C software, but its internal design is based on:

* service supervision
* capability-based authority
* message-passing IPC
* signed modules
* restartable driver services
* a privileged supervisor layer
* QFS object-oriented filesystem indexing

The target identity is:

```text
GNU/Mirage
```

Meaning:

```text
GNU/POSIX userspace
running on top of
the Mirage kernel and supervisor architecture
```

---

## Core Rule

Mirage separates **mechanism** from **policy**.

```text
Mirage Kernel      = CPU entry/exit, privilege boundaries, low-level primitives
MTSS               = portable multitasking mechanics
Mirage Supervisor  = policy, authority, recovery
Driver Services    = isolated execution units
Applications       = POSIX/GNU-compatible programs
```

The kernel must stay small, strict, and low-level.

The supervisor owns policy decisions.

---

## Layer Model

```text
Applications
    │
GNU / POSIX Compatibility Layer
    │
Mirage libc / Mirage Runtime
    │
IPC + Capability Layer
    │
Mirage Supervisor
    │
MTSS — portable multitasking mechanics
    │
Mirage Kernel
    │
Hardware
```

---

## Mirage Kernel Responsibilities

The kernel is responsible only for core machine control.

The kernel owns:

* CPU entry/exit
* trap and syscall entry/exit
* interrupt handling
* low-level timer delivery
* architecture-specific context frame restore/save mechanics
* low-level scheduling primitives required to enter, leave, or preempt CPU execution
* virtual memory
* address spaces
* page tables
* IPC transport
* capability enforcement
* low-level module loading
* hardware privilege boundaries

The kernel does not own portable scheduler, task, or thread policy. Portable runnable-state management, task/thread lifecycle mechanics, run queues, timeslice accounting, and scheduler-visible state transitions belong in MTSS.

The kernel must not own high-level system policy.

The kernel must not become a Linux-style monolith.

The kernel should expose minimal, stable primitives that MTSS, the supervisor, and services build upon.

---

## MTSS Responsibilities

MTSS is the Mirage Multitasking Subsystem layer for portable multitasking mechanics.

Portable task/thread/scheduler logic belongs in MTSS, not directly in the kernel or supervisor.

MTSS owns:

* portable task and micro-thread lifecycle state
* runnable, blocked, sleeping, contained, terminated, and reaped state transitions
* scheduler-visible identities and accounting records
* run queues and queue transitions
* portable priority and timeslice mechanics
* scheduler lifecycle events for supervisor observation
* backend-neutral contracts for future CPU-specific scheduling backends

MTSS does not own CPU privilege transitions, page-table switching, interrupt/trap entry, or raw hardware timer programming. Those remain kernel and architecture-backend mechanisms.

MTSS does not own service policy, recovery policy, capability granting, or launch authorization. Those remain supervisor responsibilities.

---

## Mirage Supervisor Responsibilities

The Mirage Supervisor is a privileged system authority layer above the kernel.

The supervisor owns:

* service lifecycle
* driver service management
* crash detection
* crash recovery
* capability granting
* capability revocation
* service registration
* signed module validation policy
* boot service ordering
* session management
* process/service launch policy

The supervisor is not a normal userspace init.

It is closer to:

```text
PID 1 + service manager + driver manager + recovery manager + security broker
```

The supervisor is where Mirage becomes more than a kernel.


---

## Mirage MTSS Contract

1. MTSS owns portable task, process, micro-thread, scheduler-visible lifecycle, run-queue, priority hint, timeslice, and accounting mechanics.
2. The kernel owns CPU entry/exit, interrupt/trap entry, raw timer delivery, page-table switching, address-space mechanisms, and architecture context save/restore.
3. The Supervisor owns launch authorization, service lifecycle policy, crash recovery, capability grant/revoke policy, boot ordering, and service status policy.
4. MTSS must not grant capabilities, authorize PID1/services, validate signed modules as policy, directly mutate hardware privilege state, or fake service progress.
5. Scheduler state transitions must be explicit and truthful: Created/New before admission, Runnable/Ready only after preflight and queue insertion, Running only after actual dispatch, Exited/Zombie only after real exit, and Reaped only after real cleanup.
6. Preemption must flow from a bounded hardware timer/interrupt source through kernel delivery into MTSS scheduling hooks; MTSS must not program raw hardware timers directly.
7. Context switching must preserve the architecture ABI: saved CPU context, canonical user RIP/RSP, valid selectors, valid kernel stack/TSS state, and valid CR3/address-space proof before user entry.
8. Userspace task creation must validate every PT_LOAD segment, mapped executable entry, mapped writable aligned stack, address-space handle, CR3, and kernel stack before making a task runnable.
9. PID1 handoff must load `/spider-rt/sbin/spider-rs`, obtain Supervisor approval, create real kernel/MTSS process and thread records, and report Created/Runnable/Running only when each real stage has executed.
10. MTSS changes must document limitations and test evidence, including state-transition/unit tests where possible and fresh QEMU/image-validation results with `MIRAGE_REUSE_IMAGE=0` before claiming boot/runtime acceptance.

---

## Mirage MTSS Readiness and PID1 Handoff Contract

1. MTSS ONLINE means full preemptive scheduling is available.
2. MTSS DEGRADED means cooperative scheduling may be available while timer/preemption are pending.
3. Do not mark MTSS ONLINE if timer/preemption are not working.
4. PID1 handoff may proceed in DEGRADED cooperative mode if MTSS core and scheduler are ready.
5. PID1 handoff must not wait forever for full MTSS ONLINE unless an explicit policy requires preemption before userspace.
6. Every MTSS state transition must retry or re-evaluate PID1 handoff eligibility.
7. Stale PID1 pending state must not block later handoff.
8. PID1 may be marked RUNNABLE only after MTSS accepts a real task/thread into a runnable queue.
9. Status messages must distinguish scheduler-ready, degraded, full online, and failed states.
10. No fake ONLINE/RUNNABLE/RUNNING statuses.

---

## Driver Model

Mirage supports three driver execution models.

### 1. Built-In Kernel Drivers

Used only for unavoidable early boot or core machine support.

Examples:

* interrupt controller
* boot console
* minimal timer
* early CPU/architecture support

### 2. Loadable Kernel Modules

Used for performance-critical or kernel-adjacent components.

Examples:

* low-level storage
* filesystem module
* GPU kernel interface
* architecture-specific hardware module

All kernel modules must be signed and verified before loading.

### 3. Supervised Driver Services

Preferred model for most drivers.

Examples:

* `netd`
* `storaged`
* `inputd`
* `audiod`
* `displayd`
* `usbd`
* `bluetoothd`

Driver services run under supervisor control and communicate through IPC.

If a driver service crashes, the supervisor must be able to:

```text
detect crash
revoke capabilities
reclaim resources
restart service
restore IPC endpoints
continue system operation
```

A driver crash should not automatically become a kernel panic.

---

## Capability Model

Mirage must not grant raw unrestricted hardware or kernel access to services.

All authority is represented through capabilities.

A capability may represent access to:

* IPC endpoint
* IRQ line
* DMA region
* PCI device
* filesystem object
* service control operation
* module loading permission
* process handle
* memory object

Services may only perform actions for which they hold valid capabilities.

Capabilities must support:

* grant
* revoke
* check
* transfer
* inheritance rules
* crash cleanup

The supervisor issues policy decisions.

The kernel enforces capability validity.

---

## IPC Model

IPC is the main system communication primitive.

IPC must support:

* message passing
* request/reply calls
* endpoint registration
* capability passing
* shared-memory transport for large data
* service discovery through supervisor-controlled registries

IPC is used for:

* filesystem calls
* networking
* driver communication
* service control
* process management
* POSIX compatibility translation

Applications may appear to call POSIX APIs, but internally those calls may become IPC transactions.

Example:

```text
write(fd, buf, len)
    -> Mirage libc
    -> Mirage runtime
    -> IPC to console/filesystem service
    -> kernel-enforced capability check
```

---

## POSIX/GNU Compatibility

Mirage is not Unix internally, but it must provide a Unix-like compatibility surface.

C and GNU programs should see:

* libc headers
* POSIX-like ABI
* ELF loading
* files
* pipes
* sockets
* signals
* errno
* pthreads
* fork/exec or compatible spawn semantics

Internally, Mirage may implement these through:

* services
* IPC
* capabilities
* supervisor mediation
* QFS objects

The rule is:

```text
Unix outside.
Mirage inside.
```

C programs must not need to know that Mirage internals are not Unix.

---

## Mirage libc

Mirage libc may be implemented in Rust while exporting a C ABI.

Required pattern:

```rust
#[no_mangle]
pub extern "C" fn write(fd: i32, buf: *const u8, len: usize) -> isize {
    // translate POSIX write into Mirage runtime operation
}
```

C programs link against normal libc-style symbols.

The implementation language is irrelevant to C as long as the ABI, headers, and behavior are correct.

Mirage libc should expose:

* standard POSIX symbols
* Mirage-native runtime hooks
* errno handling
* syscall/IPC wrappers
* process startup support

---

## Header Strategy

Mirage must provide a sysroot.

Example:

```text
/sysroot
├── usr/include
│   ├── stdio.h
│   ├── stdlib.h
│   ├── unistd.h
│   ├── fcntl.h
│   ├── errno.h
│   └── sys/
├── usr/include/mirage
│   ├── ipc.h
│   ├── capability.h
│   ├── service.h
│   └── supervisor.h
└── usr/lib
    ├── crt0.o
    ├── libc.a
    ├── libmirage.a
    └── ld-mirage.so
```

GNU/POSIX software should include normal headers.

Mirage-native software may include Mirage-specific headers.

---

## QFS

QFS is the native Mirage filesystem concept.

QFS uses this conceptual hierarchy:

```text
Library / Book / Chapter / Page / Sector
```

Mapping:

```text
Library  = volume / namespace / collection
Book     = package / service / domain
Chapter  = directory / object group
Page     = file / object
Sector   = block / extent
```

QFS must be:

* indexed
* journaled
* crash recoverable
* object-aware
* service-aware
* capable of storing signatures and metadata
* suitable for boot module discovery

QFS is not merely a renamed Unix directory tree.

The important part is the indexed object model.

Each object should have:

* object ID
* path identity
* metadata
* extent map
* permissions
* optional signature
* optional capability metadata
* journal transaction state

---

## Boot Model

Mirage should avoid a traditional Linux-style initrd.

Instead, Mirage uses a signed boot module set.

Boot flow:

```text
bootloader
    -> Mirage kernel
    -> Mirage supervisor
    -> verify signed boot modules
    -> load storage/filesystem services
    -> mount QFS root
    -> start core services
    -> launch POSIX/GNU environment
```

The boot module set may contain:

* kernel image
* supervisor image
* module verifier
* storage service/module
* QFS service/module
* device manager
* service manifest
* signatures

---

## Graphics Policy

Mirage is Wayland-only as a native graphics target.

X11 must not be part of the base architecture.

Future graphics stack:

```text
GPU module/service
    -> displayd
    -> Wayland compositor
    -> Wayland clients
```

XWayland may exist later as an optional compatibility layer.

---

## Development Rule

Do not fake completeness.

Prefer:

* clean traits
* explicit boundaries
* mock implementations
* unit tests
* clear TODOs

over pretending hardware support exists.

Initial goal:

```text
bootable architecture skeleton
not production kernel
```

First proof target:

```text
kernel skeleton
scheduler mock
IPC mock
capability table
supervisor
service crash/restart demo
QFS object lookup
Rust libc C ABI stubs
hello.c compatibility demo
```

---

## Non-Goals For Early Versions

Do not implement these first:

* full Linux compatibility
* full glibc
* full desktop stack
* real GPU driver
* full SMP scheduler
* production filesystem
* complete POSIX signal semantics
* package manager integration
* real cryptographic module verification

These come after the architecture skeleton is proven.

---

## Design Standard

Every contribution must preserve the Mirage architecture:

```text
small kernel
privileged supervisor
capability enforcement
IPC-first services
restartable drivers
signed modules
POSIX-compatible surface
non-Unix internals
```

If a change makes Mirage more monolithic without a clear reason, reject it.

If a change bypasses capability enforcement, reject it.

If a change hardcodes Linux assumptions into the kernel, reject it.

If a change improves POSIX compatibility without violating Mirage internals, prefer it.

---

## One-Sentence Definition

Mirage is a Rust-based hybrid service kernel for GNU/POSIX software, using a small mechanism-focused kernel, a privileged supervisor, capability-secured IPC, restartable driver services, signed modules, and QFS as its native indexed journaling filesystem.

---

## Mirage Non-Negotiable Boot Contract

1. Never produce or test a Mirage ISO/runtime image that is missing /spider-rt/sbin/spider-rs.

2. /spider-rt/sbin/spider-rs is mandatory. It is the PID1/init bootstrap binary launched by the kernel/userspace loader through MTSS.

3. /spider-rt/sbin/spider-rsd is mandatory. It is the system dispatcher daemon spawned by spider-rs.

4. /usr/bin/m1-terminal is a normal userspace app and must not be placed under /spider-rt.

5. Normal apps belong in rootfs paths such as /usr/bin. Bootstrap runtime binaries belong in /spider-rt/sbin.

6. The build/package pipeline must validate required runtime files before QEMU/VirtualBox testing.

7. A boot image missing spider-rs is a build failure, not an acceptable runtime state.

8. The live Mirage milestone boot UI must remain the default display when framebuffer is online.

9. BOOTDIAG raw text is fallback/debug only and must not replace the live milestone UI unless the milestone UI cannot initialize.

10. Do not mark Spider-rs, Spider-rsd, PID1, System Dispatcher, Userspace Loader, M1 Terminal, or IdleLoop as OK/ONLINE/RUNNING unless the real code path has executed.

11. Do not fake userspace output from the kernel.

12. Do not open a PR unless a clean image build proves the required runtime files are present and QEMU reaches the stated acceptance markers.

13. Always test with MIRAGE_REUSE_IMAGE=0 before using cached/reused images.

14. If a stage cannot be implemented, report the exact blocker and leave the status FAILED/PENDING with a reason. Do not stub success.

Required image layout:

```text
Required bootstrap runtime:
/spider-rt/sbin/spider-rs
/spider-rt/sbin/spider-rsd

Required rootfs userland:
/usr/bin/m1-terminal
/etc/spider/units/default.target
/etc/spider/units/basic.target
/etc/spider/units/m1-terminal.service
```

---

## Mirage Menuconfig Input Contract

1. Menuconfig must support Up, Down, Left, and Right arrow keys.
2. Arrow keys must not be interpreted as standalone Escape.
3. ANSI sequences ESC [ A/B/C/D and ESC O A/B/C/D must decode correctly unless a terminal library handles them directly.
4. Left and Right must have explicit menu semantics.
5. Key decoding must be tested separately from menu state transitions.
6. Menu navigation must be tested with a pure state-machine test when possible.
7. Do not ship menuconfig changes without manually testing all four arrow keys.
8. Do not leave terminal raw mode enabled after exit or panic.

---

## Mirage Boot Runtime Validation Contract

1. The boot runtime validator must never be disabled to pass a build.
2. False-positive validation failures must be fixed in the validator.
3. False-negative validation passes are release blockers.
4. The validator must check staging and generated ISO contents.
5. The validator must use Rock Ridge/POSIX paths when inspecting ISO files.
6. Required bootstrap runtime:
   /spider-rt/sbin/spider-rs
   /spider-rt/sbin/spider-rsd
7. Required rootfs userland:
   /usr/bin/m1-terminal
   /etc/spider/units/default.target
   /etc/spider/units/basic.target
   /etc/spider/units/m1-terminal.service
8. /usr/bin/m1-terminal must not be placed under /spider-rt.
9. A missing spider-rs or spider-rsd is a build failure.
10. A validator change must include both positive and negative proof.

## Mirage Userspace Launch Contract

1. Do not attempt ring3 entry unless ELF entry and initial stack are proven mapped in the target process address space.
2. The userspace loader must validate every PT_LOAD segment before creating a runnable task.
3. The initial stack builder must return only mapped, writable, canonical, aligned user stack pointers.
4. PID1 may be marked CREATED only after a real process record exists.
5. PID1 may be marked RUNNABLE only after MTSS accepts a real user task.
6. PID1 may be marked RUNNING only after execution actually begins.
7. If launch preflight fails, report an exact failure and do not execute iretq/sysret.
8. A VirtualBox Guru Meditation or QEMU triple fault during PID1 launch is a release blocker.
9. Bootdiag serial logs must not corrupt the live milestone framebuffer UI.
10. The live milestone UI must remain the default boot display after framebuffer init.

---

## Mirage Syscall ABI Contract

1. Target builds must route user-visible syscall ABI entry through real Mirage target syscall handlers that run at the intended privilege boundary, validate process/thread identity, and enforce kernel-owned mechanisms without bypassing MTSS or Supervisor policy.
2. Host-test-only syscall stubs are permitted only for host unit tests and tooling; they must be clearly gated to host/test configurations, must not be linked into target images, and must not be used as evidence that target syscalls work.
3. Every syscall that receives a userspace pointer, buffer, string, iovec, or structure must validate canonical address ranges, mapped pages, access permissions, length overflow, and copy-in/copy-out boundaries before dereferencing or exposing kernel memory.
4. Spawn, exec, and POSIX-compatible process creation syscalls must route launch authorization through the Supervisor, executable loading through the userspace loader, runnable-state admission through MTSS, and capability checks through the kernel capability layer.
5. Wait, exit, reap, and status-reporting syscalls must reflect real lifecycle transitions: exit records must come from an actual terminating task, wait must observe a real child/service state, and reaping must occur only after MTSS and resource cleanup have completed.
6. `m1-terminal` output must originate from the real `/usr/bin/m1-terminal` userspace process using the Mirage libc/runtime `SYS_WRITE` path, then flow through the syscall/IPC/capability stack to the console or terminal service; the kernel must not fake terminal output.
7. `spider-rs` must spawn `/spider-rt/sbin/spider-rsd` through the real spawn/exec syscall path after PID1 is running, with Supervisor approval and truthful MTSS state transitions for the dispatcher process.
8. `spider-rsd` must launch `/usr/bin/m1-terminal` through the configured unit dependency chain, including `/etc/spider/units/default.target`, `/etc/spider/units/basic.target`, and `/etc/spider/units/m1-terminal.service`; direct kernel-side app launch shortcuts are not acceptable.
9. Syscall return values, errno values, process IDs, service IDs, bytes-written counts, and child statuses must be derived from real execution results and validated state, not hardcoded success paths or milestone UI assumptions.
10. If any syscall, spawn, exec, wait, exit, `SYS_WRITE`, `spider-rs`, `spider-rsd`, unit launch, or userspace-output stage is incomplete, blocked, or running under a host-only stub, status must remain FAILED/PENDING/SKIPPED with the exact reason; do not report OK/ONLINE/RUNNING until the target code path has executed.

---

## Mirage Hardware Driver Contract

1. Hardware drivers must use bounded waits only. No infinite polling loops.
2. Driver failure must not block boot unless explicitly configured fatal.
3. Drivers must not mark hardware ONLINE unless real initialization succeeded.
4. Linux kernel source may be studied as a reference, but GPL code must not be copied into Mirage.
5. Hardware implementation must be original Rust code based on public specs, observed hardware behavior, and documented quirks.
6. xHCI/USB bring-up must validate MMIO, rings, command/event flow, and device descriptors before reporting ONLINE.
7. USB input must not block boot waiting for the first key event.
8. Verbose hardware diagnostics go to serial by default; framebuffer milestone UI stays concise and live.
9. Real hardware quirks must be documented with source/provenance.
10. Do not open PRs for driver bring-up unless QEMU/VirtualBox boot still works and failures are explicit.

---

## External OS Reference Contract

1. External OS repositories may be studied for architecture and behavior.
2. Do not copy code without license review and explicit documentation.
3. Mirage architecture must be preserved: lower kernel, MTSS, Supervisor, spider-rs, spider-rsd.
4. External references should produce focused Mirage-native patches, not broad rewrites.
5. Any external-reference audit must document inspected files, learned concepts, licensing, and Mirage changes.
6. Do not chase POSIX/desktop compatibility before MTSS and PID1 handoff work in QEMU.

---

## Mirage Input Driver Contract

1. Input drivers must never hang kernel boot.
2. All hardware polling loops must be bounded by timeout or max iteration count.
3. Keyboard/input device failure must degrade or disable the device, not stop PID1 handoff.
4. Polling mode means non-blocking poll-once or bounded drain, never infinite wait for input.
5. Interrupt handlers must never block or perform heavy parsing.
6. Unknown scancodes must not panic.
7. Input queues must be bounded and overflow-safe.
8. Do not mark input devices OK unless real probe/start succeeded.
9. External OS input code may be studied, but code must not be copied without license/provenance review.
10. Mirage input drivers must preserve Mirage architecture and boot policy.
