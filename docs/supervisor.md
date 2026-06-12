# Mirage Supervisor

The Supervisor is the privileged authority layer above the mechanism kernel. It owns service lifecycle policy, capability grants/revocation, manifest validation, crash recovery, and launch authorization.

For Spider-rs PID 1, the Supervisor must authorize a launch request and prepare capability policy, but it must not call Spider-rs as a kernel function. The userspace loader and MTSS must create and schedule the user task.
