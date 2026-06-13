# Boot Runtime legacy note

The old Boot Runtime (`/bootrt`) is now an optional temporary bootstrap domain. The permanent trusted Spider source is RuntimeVfs mounted at `/spider-rt` and documented in `docs/runtime-vfs.md` and `docs/spider-runtime.md`.

Compatibility module discovery may still accept legacy `bootrt` module names during transition, but new builds generate and stage `spider-rt.img`.
