# Mirage userspace syscalls

Spider-rs v0 uses a minimal syscall ABI:

| Number | Name | Purpose |
| --- | --- | --- |
| 1 | exit(code) | terminate current userspace task |
| 2 | write(fd, buf, len) | write bytes to a kernel-mediated file descriptor |
| 3 | yield() | cooperative MTSS yield |
| 4 | getpid() | return current PID |

Unsupported calls must return `ENOSYS` rather than faking success.
