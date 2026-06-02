# Mirage Linux/POSIX Compatibility Matrix

This document is the versioned compatibility contract for Mirage kernel-facing
APIs, libc/stdlib shims, VFS behavior, and migration away from Mirage-only
interfaces. Mirage targets a **bounded, staged Linux/POSIX subset** instead of
an unbounded claim of complete Linux compatibility.

## Versioned target levels

| Level | Status | Target | Contract |
| --- | --- | --- | --- |
| **M0: Mirage native ABI** | Implemented baseline | Stable Mirage syscall numbers, IPC, device, and memory services | Existing Mirage-only services remain available for kernel and platform code, but new portable user code should not depend on them directly. |
| **M1: POSIX filesystem/runtime subset** | Current target | POSIX.1/SUS file-descriptor semantics plus Linux/BSD filesystem entry points already represented in `SyscallNumber` | Required compatibility level for libc/stdlib filesystem wrappers, allocator wrappers, string/memory primitives, and VFS errno mapping. |
| **M2: Process and path expansion** | Planned | Relative-path resolution, descriptor-rooted path operations, and richer process state | Adds missing POSIX process/path behavior without changing M1 syscall numbers. |
| **M3: Broader Unix facilities** | Non-current | Signals, `fork`/`execve`, `fcntl`, polling/event APIs, sockets, TTYs, namespaces, and advisory locks | Requires a new compatibility revision before implementation. |

A compatibility level may add new APIs, but stable numeric assignments in
`src/kernel/syscall.rs::SyscallNumber` are append-only and must not be reused
for a different operation.

## Standards and ABI sources

Mirage M1 targets the following exact standards and ABI sources:

| Source | Exact target | Mirage M1 interpretation |
| --- | --- | --- |
| **POSIX.1** | IEEE Std 1003.1-2017 / The Open Group Base Specifications Issue 7, 2018 edition | Normative baseline for file descriptors, open-file descriptions, file offsets, `read`, `write`, `close`, `lseek`, `fsync`, `ftruncate`, `mkdir`, `unlink`, `rename`, `stat`-family behavior, errno names, string/memory functions, and allocation APIs that are present in Mirage. |
| **SUS** | Single UNIX Specification Version 4, 2018 edition, including the XSI option only when explicitly named here | SUS/XSI is a reference for Unix naming and descriptor semantics, not a promise of UNIX certification, shell utilities, curses, TTYs, sockets, or full process-control coverage. |
| **Linux syscall ABI subset** | Linux x86-64 syscall names and argument shapes for `openat`, `close`, `read`, `write`, `pread64`, `pwrite64`, `lseek`, `statx`, `newfstatat`, `getdents64`, `mkdirat`, `unlinkat`, `renameat2`, `ftruncate`, `fsync`, and root-only `mount` | Mirage exposes stable Mirage syscall numbers, not Linux numeric syscall IDs. libc wrappers map Linux-shaped calls to `SyscallNumber` variants. Unsupported flags or semantics return Linux-compatible errno values when possible. |
| **BSD extensions** | Historical BSD/libc interfaces: `bzero`, `bcopy`, `bcmp`, `strnlen`, `reallocarray`, `memalign`, and directory-entry type constants (`DT_*`) | Available as compatibility conveniences. They do not imply BSD kernel ABI compatibility, BSD process semantics, sockets, kqueue, or vnode behavior. |
| **Mirage-specific deviations** | `SyscallNumber`, `SyscallErrorCode`, Mirage IPC/device APIs, bounded heap-free VFS path grammar, and fixed descriptor capacity | These are explicit deviations from POSIX/Linux/SUS. They are stable for Mirage-native code but should be wrapped or isolated in portable user components. |
| **Future POSIX/SUS editions** | POSIX.1-2024 / The Open Group Base Specifications Issue 8 and later | Not an M1 conformance target. New Issue 8 APIs or semantic changes require a new compatibility revision before being treated as required. |

## Required libc/stdlib functions to kernel services

The M1 libc and stdlib surface is intentionally small. The table below lists
functions that are required for the staged target, where each operation gets its
kernel service, and whether the implementation is syscall-backed or local.

| Function or family | Source level | Kernel service | Notes |
| --- | --- | --- | --- |
| `openat`, `open` | POSIX/Linux | `SyscallNumber::OpenAt` | `open` is `openat(AT_FDCWD, path, flags, mode)`. Relative paths are tracked as a future M2 requirement; current VFS resolution requires absolute path strings. |
| `close` | POSIX/Linux | `SyscallNumber::Close` | Closes the descriptor table entry and releases the referenced open-file description. |
| `read` | POSIX/Linux | `SyscallNumber::Read` | Uses and updates the shared open-file-description offset. |
| `write` | POSIX/Linux | `SyscallNumber::Write` | Uses and updates the shared open-file-description offset; write permission and filesystem capacity errors map to errno. |
| `pread64` | POSIX/Linux | `SyscallNumber::Pread64` | Required syscall ABI slot; libc wrapper is not yet exported and should be added before declaring M1 complete for offset-preserving reads. |
| `pwrite64` | POSIX/Linux | `SyscallNumber::Pwrite64` | Required syscall ABI slot; libc wrapper is not yet exported and should be added before declaring M1 complete for offset-preserving writes. |
| `lseek` | POSIX/Linux | `SyscallNumber::Lseek` | Updates the open-file-description offset. |
| `stat`, `fstat` | POSIX/SUS | `SyscallNumber::NewFstatAt` | libc maps these convenience functions through `newfstatat`-style service calls. |
| `statx` | Linux | `SyscallNumber::Statx` | Linux extension retained because it is already represented in the syscall ABI. |
| `mkdir`, `mkdirat` | POSIX/Linux | `SyscallNumber::MkdirAt` | `mkdir` is `mkdirat(AT_FDCWD, path, mode)`. |
| `unlink`, `unlinkat` | POSIX/Linux | `SyscallNumber::UnlinkAt` | `unlink` is `unlinkat(AT_FDCWD, path, 0)`. |
| `rename`, `renameat`, `renameat2` | POSIX/Linux | `SyscallNumber::RenameAt2` | POSIX forms pass zero Linux `renameat2` flags. Non-zero flag support is limited by kernel implementation. |
| `fsync` | POSIX/Linux | `SyscallNumber::Fsync` | Flushes the file associated with the descriptor where supported by the filesystem. |
| `ftruncate` | POSIX/Linux | `SyscallNumber::Ftruncate` | Changes file length through the descriptor. |
| `getdents64` | Linux/BSD-shaped directory iteration | `SyscallNumber::Getdents64` | Linux syscall-shaped directory enumeration using Mirage `CDirEntry`/`DT_*` payloads. |
| `mmap`, `munmap` | POSIX/Linux-shaped memory | `SyscallNumber::Mmap`, `SyscallNumber::Munmap` | `librust` supports anonymous Mirage memory mappings only; file-backed mappings, MAP flags, and fixed-address semantics are not part of M1. |
| `malloc`, `calloc`, `realloc`, `reallocarray`, `free` | C/POSIX/BSD libc runtime | `SyscallNumber::Malloc`, `SyscallNumber::Realloc`, `SyscallNumber::Free` | `calloc`/`reallocarray` are local wrappers around allocator syscalls with overflow checks. |
| `aligned_alloc`, `posix_memalign`, `memalign` | C/POSIX/BSD allocator compatibility | `SyscallNumber::MallocAligned` | `memalign` is a BSD/GNU-style compatibility export; prefer `posix_memalign` for portable code. |
| `memcpy`, `memmove`, `memset`, `memcmp`, `memchr` | C/POSIX runtime | Local implementation in `src/librust.rs` | No kernel service is required. Callers remain responsible for valid pointers and object sizes. |
| `strlen`, `strnlen`, `strcmp`, `strncmp`, `strcpy`, `strncpy`, `strcat`, `strncat`, `strchr` | C/POSIX/BSD runtime | Local implementation in `src/librust.rs` | `strnlen` is treated as a BSD/POSIX compatibility helper; string functions are byte-oriented and do not add locale support. |
| `bzero`, `bcopy`, `bcmp` | BSD libc compatibility | Local wrappers over memory primitives | Migration target is `memset`, `memmove`, and `memcmp` respectively. |
| `stdlib::fs::*` constants and payloads | POSIX/Linux/BSD-shaped ABI data | `crate::kernel::fs` constants and C-compatible structs | Re-exported so user code can import a stdlib-shaped namespace without creating duplicate C symbols. |

## Required Linux/BSD syscalls to `SyscallNumber`

Mirage syscall numbers are not Linux syscall numbers. The stable contract is the
`SyscallNumber` variant and its raw Mirage number.

| Linux/BSD syscall or interface | Required level | `src/kernel/syscall.rs::SyscallNumber` | Raw Mirage number | M1 status and notes |
| --- | --- | --- | --- | --- |
| `getpid` | Mirage native / POSIX-shaped | `GetPid` | `0` | Mirage process ID service; libc-level POSIX `getpid()` export is not part of the current filesystem-focused M1 table. |
| `openat` | Linux/POSIX | `OpenAt` | `16` | Primary path-opening syscall. |
| `open` | POSIX convenience | `OpenAt` | `16` | libc compatibility wrapper over `openat(AT_FDCWD, ...)`. |
| `close` | Linux/POSIX | `Close` | `17` | Descriptor close. |
| `read` | Linux/POSIX | `Read` | `18` | Descriptor read. |
| `write` | Linux/POSIX | `Write` | `19` | Descriptor write. |
| `pread64` / BSD `pread` shape | Linux/POSIX | `Pread64` | `20` | ABI slot exists; libc export and full service coverage must be verified before claiming complete M1 support. |
| `pwrite64` / BSD `pwrite` shape | Linux/POSIX | `Pwrite64` | `21` | ABI slot exists; libc export and full service coverage must be verified before claiming complete M1 support. |
| `lseek` | Linux/POSIX | `Lseek` | `22` | Descriptor offset control. |
| `statx` | Linux extension | `Statx` | `23` | Linux-specific status syscall. |
| `newfstatat` / `fstatat` | Linux/POSIX | `NewFstatAt` | `24` | Used by `stat`, `fstat`, and fstatat-style compatibility. |
| `getdents64` / BSD-style directory iteration | Linux/BSD-shaped | `Getdents64` | `25` | Directory enumeration with Mirage C directory-entry layout. |
| `mkdirat` / `mkdir` | Linux/POSIX | `MkdirAt` | `26` | `mkdir` wraps `mkdirat(AT_FDCWD, ...)`. |
| `unlinkat` / `unlink` | Linux/POSIX | `UnlinkAt` | `27` | `unlink` wraps `unlinkat(AT_FDCWD, ..., 0)`. |
| `renameat2` / `renameat` / `rename` | Linux/POSIX | `RenameAt2` | `28` | POSIX forms pass zero flags. |
| `ftruncate` | Linux/POSIX | `Ftruncate` | `29` | Descriptor truncation. |
| `fsync` | Linux/POSIX | `Fsync` | `30` | Descriptor synchronization. |
| `mount` | Linux-shaped Mirage placeholder | `Mount` | `31` | Root-only compatibility placeholder; not Linux namespace/mount propagation compatibility. |
| `mmap` | POSIX/Linux-shaped memory | `Mmap` | `8` | Anonymous Mirage memory mapping only. |
| `munmap` | POSIX/Linux-shaped memory | `Munmap` | `9` | Mirage memory unmap service. |
| `malloc`/`free` allocator backing | libc runtime | `Malloc`, `Free` | `10`, `11` | Mirage allocator services; not Linux syscalls. |
| `realloc` allocator backing | libc runtime | `Realloc` | `13` | Mirage allocator service; not a Linux syscall. |
| `aligned_alloc`/`posix_memalign` backing | libc runtime | `MallocAligned` | `14` | Mirage allocator service; not a Linux syscall. |
| BSD `bzero`, `bcopy`, `bcmp` | BSD libc extension | None | N/A | Local user-space wrappers; no kernel ABI. |
| BSD/GNU `reallocarray`, `memalign`, `strnlen` | BSD/GNU libc extension | Allocator syscalls or local code | N/A | Compatibility helpers, not standalone kernel syscalls. |

## Supported process and descriptor model

Mirage implements the following POSIX-style process file state for M1:

- A **per-process file descriptor table** with fixed capacity (`MAX_OPEN_FILES`).
- A descriptor table entry references a kernel open-file description.
- Open-file descriptions keep the shared file offset, open status flags, and a
  reference count.
- Descriptor-local flags are separate from open status flags. `O_CLOEXEC` is
  represented as descriptor `FD_CLOEXEC` state.
- Child processes inherit the parent descriptor table at spawn time. Inherited
  descriptors share open-file descriptions and increment reference counts.
- Process teardown closes all descriptors and decrements the referenced
  open-file-description counts.
- Each process tracks `cwd`, `root`, and `umask`. The current VFS only resolves
  absolute paths; relative-path composition from `cwd` and directory-backed
  descriptor roots is tracked as process state but not yet exposed by the path
  resolver.

## Errno compatibility

Mirage preserves structured kernel/VFS errors internally. At syscall/libc
boundaries those errors are translated to Linux-compatible errno numbers where a
clear mapping exists: `ENOENT`, `EBADF`, `ENOTDIR`, `EISDIR`, `EEXIST`,
`EACCES`, `EROFS`, `ENOSPC`, `EBUSY`, `EXDEV`, `EMLINK`, `ENAMETOOLONG`,
`EINVAL`, and `ENOTSUP` are part of the supported mapping.

Path syntax failures that Linux would not normally observe because Linux accepts
a broader path grammar are documented deviations. Mirage maps invalid bytes and
absolute-path violations to `EINVAL`, while empty paths map to `ENOENT` and path
length failures map to `ENAMETOOLONG`.

## Mirage-specific deviations

The following deviations are intentional in M1:

- Paths are heap-free and bounded by `MAX_PATH_BYTES`; components are bounded by
  `MAX_COMPONENT_BYTES`.
- Path bytes are restricted to ASCII alphanumerics plus `.`, `-`, `_`, and `/`.
- Relative path resolution is currently not implemented by the VFS resolver.
  `cwd` and `root` are maintained for the process ABI boundary, but syscalls
  currently require absolute path strings.
- No dynamic descriptor-table growth is provided; exhaustion returns an
  out-of-memory style syscall error/`ENOMEM` at the libc layer.
- `mount` is a capability placeholder for the root filesystem and does not model
  Linux mount namespaces, propagation, bind mounts, or filesystem-type-specific
  options; filesystem type handling is limited to the default `qfs` root and
  the explicitly recognized `ext4`/`ssd_usb` compatibility names.
- Mirage syscall numbers are stable Mirage ABI numbers and deliberately do not
  match Linux architecture syscall-number tables.
- Mirage-native IPC and device APIs remain available but are not POSIX, SUS,
  Linux, or BSD compatibility guarantees.

## Explicit non-goals for M1

Because “complete Linux/POSIX compatibility” is reduced to a staged target, the
following are explicit non-goals until a later compatibility revision says
otherwise:

- UNIX certification, POSIX certification, or conformance claims for the full
  POSIX.1/SUS specifications.
- Shells, utilities, curses, locale catalogs, user/group databases, or full XSI
  option coverage.
- `fork`, `vfork`, `execve`, wait semantics, process groups, sessions, job
  control, credentials, or POSIX spawn semantics beyond Mirage-native `spawn`.
- Signals, timers, interval timers, `sigaction`, signal masks, or signal-driven
  I/O.
- Linux `fcntl` operations, descriptor duplication APIs, advisory locks,
  mandatory locks, leases, `O_PATH`, `O_TMPFILE`, and full `O_NOFOLLOW` edge
  semantics unless explicitly implemented and documented in a later level.
- Sockets, pipes, FIFOs beyond type constants, TTYs, pseudoterminals, network
  stacks, `select`, `poll`, `epoll`, `kqueue`, inotify, fanotify, or eventfd.
- Linux namespaces, chroot/pivot-root, mount namespaces, bind mounts, overlay
  mounts, propagation, and filesystem-specific mount options.
- File-backed `mmap`, shared mappings, fixed-address mappings, executable
  mapping policy, copy-on-write mapping semantics, or memory protection behavior
  beyond Mirage `MemoryProtection` bits.
- ABI compatibility with Linux ELF loaders, vDSO, dynamic linkers, glibc, musl,
  or BSD libc binaries.
- Full Linux path grammar, unbounded path lengths, dynamic descriptor-table
  growth, or Linux-compatible behavior for every invalid-path edge case.

## Migration guidance for Mirage-only APIs

Portable user code should migrate toward POSIX-shaped wrappers while keeping
Mirage-native APIs isolated behind platform modules.

### `src/libc/`

- Prefer the unprefixed filesystem wrappers exported from `src/libc/mod.rs` over
  direct calls to `mirage_*` filesystem symbols: open wrappers in
  `src/libc/fcntl.rs` (`openat` and `open`), descriptor wrappers in
  `src/libc/unistd.rs` (`close`, `read`, `write`, and `lseek`), stat and
  filesystem mutation wrappers in `src/libc/sys_stat.rs` (`stat`, `fstat`,
  `statx`, `mkdir`, `mkdirat`, `unlink`, `unlinkat`, `rename`, `renameat`,
  `renameat2`, `fsync`, and `ftruncate`), and directory entry wrappers in
  `src/libc/dirent.rs` (`getdents64`).
- Treat `mirage_openat`, `mirage_read`, and similar prefixed symbols in
  `src/libc/fcntl.rs`, `src/libc/unistd.rs`, `src/libc/sys_stat.rs`, and
  `src/libc/dirent.rs` as transitional C ABI aliases for existing Mirage-only
  callers. New code should use POSIX/Linux-shaped names and reserve `mirage_*`
  for functionality that has no POSIX-shaped equivalent.
- Keep process, IPC, and device services exported through `src/libc/mod.rs`
  (`getpid`, `spawn`, `send_ipc`, `receive_ipc`, `receive_ipc_or_block`,
  `block_for_ipc`, `enumerate_devices`, `device_info`, `device_read`,
  `device_write`, and the `mirage_device_*` C symbols) in Mirage-specific
  adapter modules. Do not present them as POSIX or Linux APIs until a
  compatibility revision defines their portable contract.
- When adding missing M1 exports such as `pread64` and `pwrite64`, route them
  through `SyscallNumber::Pread64` and `SyscallNumber::Pwrite64` rather than
  introducing new Mirage-only names first.
- Use `src/libc/string.rs` for string/memory compatibility exports.

### `src/stdlib.rs`

- Use `src/stdlib.rs` as the stable import surface for no-alloc user code that
  wants a stdlib-shaped namespace, because it re-exports the libc filesystem
  wrappers without creating duplicate exported C symbols.
- Add future portable wrappers to `stdlib` only after they exist in `libc` or in
  a clearly local runtime implementation; do not add Mirage IPC/device shortcuts
  to `stdlib` unless they are named as Mirage-specific.
- Keep ABI structs and constants under `stdlib::fs` when they describe C-facing
  filesystem payloads (`CStat`, `CDirEntry`, `O_*`, `S_IF*`, `DT_*`, and access
  mode constants). Avoid duplicating numeric constants in application code.

### `src/librust.rs`

- Continue using `librust` for local C/POSIX/BSD runtime primitives and allocator
  entry points that are needed before a full hosted libc is available.
- Prefer standard names (`memcpy`, `memmove`, `memset`, `memcmp`, `malloc`,
  `calloc`, `realloc`, `free`, `aligned_alloc`, `posix_memalign`, `mmap`, and
  `munmap`) in new code. Use BSD compatibility names (`bzero`, `bcopy`, `bcmp`,
  `memalign`, `reallocarray`) only when porting code that already expects them.
- Treat `mmap`/`munmap` in `librust` as Mirage anonymous-memory services, not as
  file-backed Linux memory mappings. Code that depends on file descriptors,
  offsets, shared mappings, or detailed MAP flags must stay behind a platform
  abstraction until M3 or a dedicated memory-mapping revision exists.
- Do not wrap imports in `try`/`catch` or add dynamic runtime fallbacks around
  these exports; the compatibility boundary should be explicit and testable.

Future work should update this matrix before adding new Linux/POSIX/BSD behavior,
and code should target the named compatibility level instead of asserting
complete Linux compatibility.
