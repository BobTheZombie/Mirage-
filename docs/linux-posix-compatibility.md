# Mirage Linux/POSIX Compatibility Boundary

Mirage intentionally targets a **bounded Linux/POSIX subset** rather than an
unbounded claim of 100% Linux compatibility. This document is the compatibility
contract for kernel-facing APIs, libc shims, and VFS behavior.

## Supported process and descriptor model

Mirage implements the following POSIX-style process file state:

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

## Supported filesystem syscall subset

The supported Linux/POSIX-like syscall surface is limited to the operations
already represented in `SyscallNumber`:

- `openat`, `close`, `read`, `write`
- `pread64`, `pwrite64`, `lseek`
- `statx`/`newfstatat`, `getdents64`
- `mkdirat`, `unlinkat`, `renameat2`
- `ftruncate`, `fsync`
- `mount` only accepts the existing root target and otherwise reports an
  unsupported operation

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

## Documented deviations from Linux

The following deviations are intentional in this target:

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
  options.
- Signals, `fork`, `execve`, Linux `fcntl` operations, advisory locks,
  `O_PATH`, `O_TMPFILE`, leases, epoll/inotify, and namespace semantics are not
  part of the current compatibility boundary.

Future work should expand this document before adding new Linux/POSIX behavior,
and code should target this bounded surface instead of asserting complete Linux
compatibility.
