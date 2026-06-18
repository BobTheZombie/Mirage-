# Framebuffer failure screen

The persistent failure screen is drawn by the boot diagnostics layer after fatal boot failure. It shows:

* `MIRAGE BOOT FAILURE`
* current and last phase
* last substep ID and message
* source file/line for the breadcrumb caller
* panic or fault reason
* fault vector, error code, RIP, RSP, RFLAGS, and CR2 when available
* the last 20 boot log entries
* `Press ESC for debug shell` when keyboard input is online, otherwise `Keyboard unavailable`

The screen does **not** clear first. That preserves any useful framebuffer evidence that existed before the failure. Serial receives a fuller boot-log dump even when keyboard input is unavailable.

The debug shell is expected to own the framebuffer only after explicit activation and should use `clear_for_debug_shell()` so the transition is logged. Required diagnostic commands are `bootlog`, `phases`, `lastfault`, `devices`, `pci`, `ryzen`, and `fb`; current code records the backing data needed by those commands even where the interactive command parser remains a milestone stub.
