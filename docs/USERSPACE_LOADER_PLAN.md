# Mirage Userspace Loader Plan

This document describes the planned path for launching GNU/POSIX-compatible
userspace on top of Mirage internals. The external surface should look familiar
to C and GNU programs, while the implementation remains capability-secured,
IPC-first, and supervisor-monitored.

## Implemented now

* Mirage defines a POSIX/GNU compatibility surface with libc-shaped symbols,
  ELF loading goals, files, pipes, sockets, errno, pthreads, and spawn/fork-like
  semantics as compatibility targets.
* Mirage libc is planned as a C ABI surface that may be implemented in Rust. C
  programs link against normal libc-style symbols while those symbols translate
  into Mirage runtime, IPC, and capability operations.
* The supervisor is the service/process launch policy owner. Process handles,
  service registration, capability grants, crash detection, and monitoring are
  not supposed to be hardcoded as kernel policy.

## Stubbed now

* The ELF loader is a plan, not a complete production loader. Early work may
  validate headers and model segments, but complete relocation, interpreter,
  TLS, permission, and failure semantics remain future work.
* Address-space creation and initial stack construction are architectural
  scaffolding. The intended launch record includes argv, envp, auxv, stack
  bounds, executable mappings, and initial capabilities.
* Mirage libc process entry is not yet a full crt0/libc startup path. Stubs may
  expose C ABI functions, but complete process startup, errno storage, thread
  setup, and runtime hooks are incomplete.
* RAMFS attachment for early userspace is a planned compatibility bridge. It
  should be granted as a filesystem capability rather than exposed as a global
  kernel filesystem namespace.

## Planned next

* Implement a minimal ELF loader that validates executable headers, maps loadable
  segments into a new address space with correct permissions, and records the
  Mirage runtime entry contract.
* Build the initial user stack with argv, envp, auxv, stack alignment, and a
  small capability bootstrap table for the runtime.
* Enter Mirage libc through a defined startup ABI, then register the POSIX-shaped
  process with the supervisor so monitoring, signals/events, descriptors, and
  crash cleanup have an owner.
* Attach RAMFS or QFS-backed filesystem roots through explicit filesystem
  capabilities and descriptor table initialization, not through ambient Unix
  globals.
* Target the future userspace launch path:

```text
ELF loader
    -> address space
    -> stack / auxv / envp / argv
    -> Mirage libc entry
    -> POSIX process registration
    -> RAMFS attachment
    -> supervisor monitoring
```
