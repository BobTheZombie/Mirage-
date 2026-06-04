# Mirage Interrupt Ownership

This document defines who owns interrupt delivery in Mirage and how interrupt
authority should flow to supervised driver services without making the kernel a
monolithic driver host.

## Implemented now

* The kernel owns interrupt delivery. Interrupt descriptor setup, CPU exception
  entry, low-level interrupt acknowledgement, and routing into kernel mechanisms
  are kernel responsibilities.
* Early boot and architecture support may use built-in kernel interrupt handling
  for unavoidable machine control such as timers, exception handling, and boot
  console support.
* The Mirage capability model includes IRQ-line authority as a distinct kind of
  resource. A driver must not assume it can receive or bind an interrupt unless
  it holds the relevant IRQ, MSI, or MSI-X capability.
* Existing hardware status documentation treats real IRQ/MSI/MSI-X routing as a
  missing hardware milestone, not as silently complete functionality.

## Stubbed now

* Supervisor interrupt subscription is not yet a complete runtime interface. The
  planned design allows the supervisor to subscribe a service to an interrupt
  stream later, but the kernel remains the delivery and enforcement owner.
* Driver interrupt endpoints are architectural placeholders. A supervised driver
  may be modeled as receiving events through IPC, but production interrupt-to-IPC
  routing, masking, unmasking, and backpressure behavior are not complete.
* IRQ capability revocation is specified as a required recovery operation, but
  full teardown of routed vectors, pending events, and device-side interrupt
  state is still planned work.

## Planned next

* Add a kernel interrupt-routing table that binds physical IRQs or MSI/MSI-X
  vectors to kernel-owned delivery records and optional supervisor-approved
  service subscriptions.
* Define an IRQ capability schema with vector identity, trigger/mask metadata,
  target endpoint, permitted acknowledgement operations, and revocation state.
* Add supervisor policy calls for granting, revoking, and reassigning driver IRQ
  capabilities during service start, crash recovery, and hotplug.
* Deliver driver interrupts as capability-checked IPC notifications where
  practical, with shared-memory completion queues reserved for high-throughput
  devices.
* Preserve the boundary:

```text
hardware interrupt
    -> kernel interrupt delivery
    -> kernel capability check
    -> supervisor-approved subscription
    -> driver service notification
```
