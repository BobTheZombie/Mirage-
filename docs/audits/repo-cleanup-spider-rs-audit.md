# Mirage repository cleanup and Spider-rs audit

## Workspace and build findings

- The Cargo workspace includes kernel crate `mirage`, hardware/component crates under `crates/`, host tools under `tools/`, and `userspace/spider-rs` as the active Spider userspace crate.
- `userspace/spider-pid1` was a duplicate no_std PID1 experiment. It overlapped the real `userspace/spider-rs` package and is removed from the workspace so one Spider source tree owns PID 1.
- `make spider-rs` now builds `userspace/spider-rs` for the Mirage no_std target and installs the resulting ELF to `build/userspace/spider-rs`.
- `make spider-rt-tree`, `make spider-rt-image`, and `make runtime-images` create the permanent trusted Spider runtime image.

## Spider-rs location and status

Spider-rs lives in `userspace/spider-rs/`. It exists and is now the canonical no_std userspace PID 1 service manager source tree. It is not linked into the kernel and is not called as a kernel function.

## Runtime image references

- Permanent trusted source: `/spider-rt/sbin/spider-rs` from `build/spider-rt.img`.
- Optional temporary bootstrap domain: `/bootrt` remains recognized for compatibility but is not the canonical Spider source.
- Limine now stages `spider-rt.img` with `mirage.module=spider-rt`.

## Boot path audit

The active path is:

```text
seed-rs -> kernel_main -> Mirage-dispatch-rs kernel component startup
        -> SupervisorCreated -> RuntimeVfs /spider-rt
        -> Userspace Loader -> ELF validation
        -> MTSS PID allocation/task object -> Spider-rs Started/Stub
```

The current kernel still must not mark Spider-rs Online merely because a task object exists. Online is valid only after real ring-3 execution and syscall write confirmation.

## Dead/obsolete candidates

- `userspace/spider-pid1`: duplicate of Spider-rs no_std PID1; removed from workspace and superseded by `userspace/spider-rs`.
- `src/subkernel`: still referenced by capability/security code and cannot be deleted in this pass. Its remaining design value is documented in `docs/legacy/subkernel.md`; future work should split reusable capability types into `mirage-cap`/Supervisor-facing modules and retire the subkernel name.
- Old `/bootrt`-only docs and messages: updated or superseded by `/spider-rt` RuntimeVfs language.

## Code to keep

- Kernel no_std code, architecture backends, storage drivers, MTSS, Supervisor, Mirage-dispatch-rs, userspace ELF validator, syscall ABI notes, and RuntimeVfs image parser.

## Follow-up cleanup

1. Rename `boot_runtime.rs` symbols to RuntimeVfs/SpiderRuntime names after dependent docs/tests are updated.
2. Move remaining subkernel capability structs into a non-obsolete capability module.
3. Add a true ring-3 entry backend so Spider-rs can become Online honestly.
4. Extend RuntimeVfs from single immutable image parsing to explicit `stat`, `readdir`, and execute permission metadata.
