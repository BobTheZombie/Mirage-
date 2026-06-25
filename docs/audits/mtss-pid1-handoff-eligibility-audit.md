# MTSS PID1 Handoff Eligibility Audit

## Scope

This audit qualifies the PID1 boot gate wording around MTSS readiness. It replaces stale shorthand that implied PID1 must always wait for full `MTSS ONLINE` with explicit cooperative versus preemptive eligibility semantics.

## Findings

1. `MTSS ONLINE` must mean full preemptive scheduling only. It requires MTSS core readiness, scheduler readiness, timer readiness, preemption readiness, and architecture context restore proof.
2. Degraded/cooperative MTSS is a valid intermediate state. It has core/scheduler/idle/API readiness but lacks full preemption proof.
3. Degraded/cooperative MTSS may create task/thread records and mark PID1 runnable when the userspace loader, Supervisor approval, ELF/stack/address-space preflight, idle fallback, and runnable admission APIs are valid.
4. The policy switch `require_preemption_for_userspace` controls whether degraded/cooperative MTSS may admit PID1. The documented default is `false`.
5. When `require_preemption_for_userspace = true`, PID1 handoff must remain pending until preemption readiness is proven.
6. Boot coordination must retry PID1 handoff after MTSS state changes so an earlier pending result does not survive after core, scheduler, degraded, timer, preemption, or online readiness changes.

## Required status semantics

| Status | Exact meaning |
| --- | --- |
| `PID1 HANDOFF [ALLOWED: cooperative MTSS]` | Core/scheduler/idle/API readiness exists, policy permits cooperative userspace (`require_preemption_for_userspace = false`), and PID1 may be created and admitted runnable without claiming `MTSS ONLINE`. |
| `PID1 HANDOFF [ALLOWED: preemptive MTSS]` | Full preemptive MTSS readiness exists; `MTSS ONLINE` may be truthful and PID1 may launch under the preemptive scheduling contract. |
| `PID1 HANDOFF [PENDING: policy requires preemption before userspace]` | Cooperative readiness may exist, but `require_preemption_for_userspace = true` and preemption readiness is not yet proven. |

## Documentation updates

* `docs/kernel/mtss.md` now defines layered readiness and qualifies `MTSS ONLINE` as preemptive-only.
* `docs/kernel/mtss-readiness.md` provides the authoritative readiness vocabulary, policy default, retry behavior, and PID1 handoff status definitions.
* `docs/kernel/mtss-preemption.md` ties preemption proof to `MTSS ONLINE` and the policy-pending handoff status.
* `docs/boot/pid1-handoff.md` replaces the unconditional `MTSS online` gate with an MTSS readiness gate and documents cooperative/preemptive PID1 eligibility.

## Remaining limitations

This is a documentation audit only. It does not prove target ring-3 execution, spider-rsd spawning, M1 Terminal launch, or full hardware preemption. Those stages must remain pending unless their real code paths execute.
