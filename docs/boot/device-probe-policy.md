# Device Probe Policy

Boot-critical services are rootfs, Supervisor, MTSS, and the Spider runtime. Input devices are optional.

## Optional input

Keyboard failure must report `DEGRADED` or `DISABLED` and continue boot. No keyboard path may prevent PID1 handoff.

## Bounded hardware probing

Hardware probe loops must have timeouts or maximum iteration counts. Missing devices, absent ACKs, and controller timeouts are normal degraded outcomes, not kernel panics.

## Display policy

Serial logs may include detailed probe diagnostics. The milestone framebuffer UI stays concise and must not be spammed with raw scancodes.
