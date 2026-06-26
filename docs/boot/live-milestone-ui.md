# Live milestone UI

The framebuffer milestone UI is a status renderer, not a scheduler or boot driver. Internal continuation edges such as `KernelConstructed` must remain serial-visible without synchronously gating the next boot phase. Debug shell polling is non-blocking and must not gate rootfs, supervisor, MTSS, or PID1 handoff.
