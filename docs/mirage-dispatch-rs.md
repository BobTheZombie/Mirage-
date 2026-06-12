# Mirage-dispatch-rs

Mirage-dispatch-rs is the kernel-internal component dispatcher. It is not userspace init and does not replace the supervisor.

## Responsibilities

- Register compiled-in kernel services/components.
- Check feature gates and dependencies.
- Probe platform presence from the Platform Registry.
- Start components in deterministic order.
- Emit Boot Phase Manager transitions.

## Lifecycle

```text
Registered -> Skipped
Registered -> Detected -> Started -> Ok/Online/Enabled/Stub/Failed
Online -> Running (for persistent scheduler/idle/service loops)
```

A disabled feature or absent device must be Skipped, not Ok. A component with detected hardware but missing implementation should be Stub with an exact reason.
