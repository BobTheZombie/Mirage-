# Mirage boot flow

The required boot continuation pipeline is:

1. early architecture initialization
2. memory and framebuffer initialization
3. interrupt and platform probe
4. optional storage and input probe
5. kernel construction
6. boot-info application
7. supervisor construction
8. boot runtime validation and RuntimeVfs mount
9. root filesystem mount
10. supervisor service initialization
11. MTSS/PID0 initialization
12. PID1 handoff eligibility
13. userspace loader start
14. spider-rs ELF load and preflight
15. PID1 process/thread creation and MTSS runnable admission
16. scheduler/idleloop entry

The boot UI reflects this state only. It must not drive boot progress or block continuation.
