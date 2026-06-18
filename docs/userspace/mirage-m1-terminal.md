# Mirage M1 Terminal

`mirage-m1-terminal` is the first planned userspace child app. It is not PID1 and must not replace Spider-rs.

Expected output once Spider-rs can launch children and the console ABI exists:

```text
Mirage M1.1 System
hello world
```

The current boot path records this app as a dispatcher manifest child and reports:

```text
M1 TERMINAL [ PENDING: dispatcher child launch not implemented ]
```

The kernel does not print the two output lines directly, because that would fake userspace success.
