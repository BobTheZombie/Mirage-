# m1-terminal

`m1-terminal` is the first normal userspace app. It is packaged as `/usr/bin/m1-terminal`, not under `/spider-rt`. Its program output is:

```text
Mirage M1.1 System
hello world
```

The text is emitted by the userspace binary through the Mirage write syscall shim when ring-3 execution is available.
