# Mirage Userspace Syscall ABI

Status: planned/minimal ABI for Spider-rs PID 1. The ABI is documented before full ring-3 enablement so the kernel and no_std Spider-rs shim stay aligned.

## x86_64 convention

- `rax`: syscall number and return value.
- `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`: arguments 0 through 5.
- Return values are non-negative on success and negative errno values on failure.

## Initial syscall numbers

| Number | Name | Arguments |
| --- | --- | --- |
| 1 | `exit` | `status` |
| 2 | `write` | `fd`, `buf`, `len` |
| 3 | `yield` | none |
| 4 | `getpid` | none |

`write(1|2, buf, len)` writes to serial/framebuffer after validating that `buf..buf+len` is a canonical userspace range.
