# Mirage Boot Module Policy

This document defines the policy boundary for signed boot modules. Boot modules
are the bridge between the bootloader-provided module set and the supervisor's
service launch plan, but parsing, verification, approval, and launch must remain
separate steps.

## Implemented now

* Mirage uses a signed boot module set concept instead of a traditional
  Linux-style initrd as the architectural boot source for supervisor-managed
  services and early modules.
* Boot module handling is routed through the supervisor policy layer rather than
  treated as arbitrary kernel policy. The kernel provides low-level module
  loading and enforcement primitives; the supervisor decides what is trusted and
  what should start.
* The current architecture distinguishes boot service ordering, service
  lifecycle, signed module validation policy, and launch policy as supervisor
  responsibilities.

## Stubbed now

* Boot manifest parsing is a separate planned stage. Early code and docs may use
  fixed manifests or mock structures, but parsing should not imply approval or
  launch.
* Signature verification is currently mock verification. The verifier may model
  success/failure and module identity, but it is not a real cryptographic trust
  chain yet.
* Policy approval is separate from verification. A valid signature only proves
  identity/integrity under the chosen trust model; the supervisor still decides
  whether this boot profile, service role, version, and capability set are
  allowed.
* Launch is separate from parsing, verification, and approval. A module should
  not be started until its manifest entry is parsed, its signature result is
  known, and supervisor policy has approved its capabilities and ordering.

## Planned next

* Define a boot manifest format with module names, roles, hashes/signatures,
  dependencies, service class, requested capabilities, restart policy, and launch
  order.
* Replace mock signature verification with an explicit verifier interface so
  real cryptographic validation can be added without merging policy into parser
  code.
* Add a policy approval phase that maps verified module identity plus manifest
  intent into granted capabilities, denied capabilities, service dependencies,
  and launch eligibility.
* Add a launch planner that starts approved modules in dependency order and
  registers each service with supervisor monitoring before exposing it to other
  services.
* Keep the boot module pipeline as four separate boundaries:

```text
boot manifest parsing
    -> mock or real signature verification
    -> supervisor policy approval
    -> supervised launch
```
