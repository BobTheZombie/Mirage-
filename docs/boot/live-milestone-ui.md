# Live milestone UI

The framebuffer milestone UI is a status renderer, not a scheduler, boot driver, or policy engine. It renders BootPhase and KSO status; it must not create boot progress.

## Relationship to KSO

KSO reports real startup graph state into BootPhase. The live UI reads those states and displays concise progress. This preserves the core boot rule: UI reflects state, state does not follow UI.

The UI may display:

* pending dependencies;
* waiting nodes;
* required node failures;
* optional driver degradation;
* MTSS core, cooperative, preemptive, and online distinctions;
* PID1 handoff pending, allowed, runnable, or blocked status.

The UI must not:

* mark KSO nodes online;
* unblock KSO dependencies;
* turn optional device matches into hardware success;
* report Spider-rs, Spider-rsd, M1 Terminal, PID1, or IdleLoop as running before their real code paths execute;
* replace the live milestone display with raw BOOTDIAG text unless framebuffer UI initialization fails.

## Debug and fallback behavior

Internal continuation edges such as `KernelConstructed` must remain serial-visible without synchronously gating the next boot phase. Debug shell polling is non-blocking and must not gate rootfs, supervisor, MTSS, KSO retry, or PID1 handoff.

BOOTDIAG raw text is fallback/debug output. When framebuffer is online, the milestone UI remains the default display and concise high-level renderer, while detailed KSO reasons can go to serial diagnostics.
