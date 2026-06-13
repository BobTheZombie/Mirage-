# Keyboard Driver Notes

Mirage has separate PS/2 and USB HID input paths that publish common keyboard
events. PS/2 remains independent and must not regress while USB matures.

USB path:

```text
Platform Registry xHCI candidate
  -> xhci-host0
  -> usb-core0
  -> usb-hid0
  -> usb-kbd0
  -> MirageInputEvent / poll_debug_escape()
```

`usb-kbd0` is Online only after a descriptor-backed HID boot keyboard is bound and
endpoint polling is scheduled. It must not wait for a keypress during init.
# Mirage Hardware Keyboard Drivers

Mirage now has an early, no-userspace input path for laptop and QEMU keyboards.
The implementation keeps policy out of the kernel: built-in drivers only perform
low-level hardware initialization, event decoding, bounded queueing, and debug
shell notification. Supervisor policy can later consume the same events through
normal device/input interfaces.

## Common input layer

`kernel::input` defines the shared event model:

- `KeyState::{Pressed, Released}`
- `KeyCode` for escape, enter, tab, arrows, F1-F12, ASCII characters, and laptop
  hotkey events
- `KeyModifiers` for left/right shift, ctrl, alt, and caps lock
- `KeyboardEvent` with the decoded key, optional ASCII byte, raw source, and raw
  hardware code
- `InputRawSource::{Ps2, UsbHid, AcpiEc, Unknown}`

All hardware drivers publish into a fixed-size queue. Overflow drops the oldest
event so early boot never allocates or spins indefinitely. `KeyboardEvent` values
convert to the stable `MirageInputEvent` ABI for device reads.

`kernel::input::poll_debug_escape()` is the common ESC path used by the idle
loop. The architecture wrapper polls hardware sources first, then checks the
shared queue. If ESC is pressed, Mirage prints `debug shell requested` and enters
the existing early debug shell stub.

## PS/2 i8042 keyboard

Files:

- `src/arch/x86_64/i8042.rs`
- `src/arch/x86_64/ps2_keyboard.rs`

The i8042 driver performs real programmed-I/O initialization on ports `0x60` and
`0x64`:

1. disable first and second PS/2 ports
2. flush the output buffer
3. read and update the controller configuration byte
4. run controller self-test
5. test the first port and probe the second port
6. optionally disable translation to prefer scan set 2
7. enable the first port
8. reset the keyboard, wait for ACK/BAT, identify, set scan code set 2 when safe,
   and enable scanning

All controller waits have bounded timeouts. ACK, RESEND, BAT, and device-error
bytes are handled without panicking. The event path supports polling today and
sets up the configuration byte so IRQ1 can be enabled when Mirage grows an IRQ1
IDT stub.

The PS/2 decoder supports translated set 1 and native set 2 streams, extended
scancodes, press/release events, modifiers, caps lock, printable US ASCII,
enter/backspace/tab/escape, arrows, and F1-F12.

## USB HID boot keyboard through modular xHCI stack

File:

- `src/arch/x86_64/xhci_keyboard.rs`

USB input is no longer a single inline boot-time HID initializer.  It is split
into four early kernel-registered driver modules that mirror the future
supervised service model:

```text
xhci-host0 -> usb-core0 -> usb-hid0 -> usb-kbd0
```

Each module has a static descriptor, dependency list, `init()`, `start()`,
`stop()`, `poll()`, fixed-capacity state, and status.  Dependency skips
propagate downward so optional USB failure cannot freeze boot.

`xhci-host0` performs real hardware discovery and controller bring-up:

1. scans PCI config space for class `0x0c`, subclass `0x03`, prog-if `0x30`
2. enables memory space and bus mastering
3. maps BAR0 through the boot HHDM when available
4. reads xHCI capability registers, `HCSPARAMS1`, `HCCPARAMS1`, `DBOFF`, and `RTSOFF`
5. halts the controller
6. resets the controller with bounded waits
7. prepares static DCBAA, command-ring, event-ring, and ERST backing
8. programs a conservative max-slot count
9. starts the controller

`usb-core0` scans and resets root ports through bounded waits and records
fixed-capacity device records. `usb-hid0` owns HID class records. `usb-kbd0`
binds boot keyboard records, configures the interrupt-IN endpoint record, marks
`InputRawSource::UsbHid` online, and uses the common `KeyboardEvent` queue.

USB HID boot report decoding is implemented independently from enumeration:
8-byte reports are diffed against the previous report and converted to the same
`KeyboardEvent` queue as PS/2. This covers modifiers, printable US ASCII, ESC,
arrows, F keys, enter, backspace, tab, ctrl, alt, and shift.

The initialization path is fail-closed. It uses bounded waits for controller
halt/reset/run and root-port reset/enable, reports `Skipped` when dependencies
or devices are absent, and reports `Failed` with the stage message on timeout.
It never waits for a keypress before declaring the keyboard online.

Known limitation: Mirage still needs a production xHCI command/event/transfer
ring owner before descriptor-driven enumeration and runtime interrupt endpoint
polling are complete. Do not reintroduce one-shot inline HID initialization to
work around that missing service boundary.

## ACPI EC hotkey events

File:

- `src/arch/x86_64/acpi_ec.rs`

The ACPI EC driver uses BootInfo RSDP presence as the firmware discovery gate and
then probes the standard EC command/status path (`0x66`) and data path (`0x62`)
without guessing a laptop vendor profile. If ACPI is absent or the EC is not
responsive, the driver skips cleanly and boot continues.

The implemented EC operations have bounded waits and support query command
`0x84`. Query codes are translated through a small table for generic hotkeys:
brightness up/down, volume up/down, mute, sleep, display switch, and an ESC
mapping for debug-shell experiments. Unknown query codes are logged and exposed
as raw vendor events.

Known limitation: no AML namespace parser exists yet, so non-standard EC I/O
resources and vendor WMI hotkey methods are not claimed. Future profiles can add
ThinkPad, Dell, HP, ASUS, and Framework-specific mapping tables without changing
the common input ABI.

## Boot phases

The Boot Phase Manager now tracks:

- `I8042`
- `PS/2 Kbd`
- `xHCI`
- `USB Core`
- `USB HID`
- `USB Kbd`
- `ACPI EC`
- `EC Hotkeys`
- `Input`

The framebuffer boot screen shows:

```text
Input        [ OK/SKIPPED/FAILED ]
PS/2 Kbd     [ OK/SKIPPED/FAILED ]
USB Core     [ ONLINE/SKIPPED/FAILED ]
USB HID      [ ONLINE/SKIPPED/FAILED ]
USB Kbd      [ ONLINE/SKIPPED/FAILED ]
EC Hotkeys   [ OK/SKIPPED/FAILED ]
```

`Input` is `OK` when at least one keyboard/event source is online.

## Feature gates

Cargo features:

- `hw-keyboard`
- `hw-i8042`
- `hw-ps2-keyboard` (enables `hw-i8042` and `hw-keyboard`)
- `hw-xhci`
- `hw-usb-hid` (enables `hw-xhci` and `hw-keyboard`)
- `hw-acpi-ec`
- `hw-laptop-hotkeys` (enables `hw-acpi-ec` and `hw-keyboard`)

Mirageconfig symbols should mirror these names:

- `CONFIG_MIRAGE_HW_KEYBOARD`
- `CONFIG_MIRAGE_HW_I8042`
- `CONFIG_MIRAGE_HW_PS2_KEYBOARD`
- `CONFIG_MIRAGE_HW_XHCI`
- `CONFIG_MIRAGE_HW_USB_HID_KEYBOARD`
- `CONFIG_MIRAGE_HW_ACPI_EC`
- `CONFIG_MIRAGE_HW_LAPTOP_HOTKEYS`

## QEMU testing

PS/2 keyboard:

```sh
make qemu-keyboard-ps2
```

USB keyboard with QEMU xHCI:

```sh
make qemu-usb-kbd
```

All keyboard paths:

```sh
make qemu-usb-none
make qemu-keyboard-all
```

All targets use serial stdio, no reboot/shutdown, and QEMU interrupt logging to
`build/qemu.log`. Pressing ESC from an online source should print
`debug shell requested`.

## Real laptop expectations

- PS/2/AT keyboards exposed by firmware should work first.
- Internal USB keyboards require complete xHCI DMA ring enumeration before they
  can be marked fully online.
- Fn/media keys can appear via PS/2, USB HID usages, ACPI EC queries, WMI, or a
  vendor AML method. Mirage only claims the generic EC query path today and logs
  unknown vendor events for future profile work.
