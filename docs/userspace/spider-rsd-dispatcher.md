# spider-rsd dispatcher

`/spider-rt/sbin/spider-rsd` is the child system dispatcher daemon launched by spider-rs. It is not modeled as its own service unit. Its responsibility is to load Spider units, resolve `default.target`, start target units, and spawn service units through the userspace spawn ABI.

The host build exercises the real parser, dependency resolver, and process-spawner abstraction. The no_std image is packaged for Mirage and will run after the architecture user-mode transition and syscall table are wired.
