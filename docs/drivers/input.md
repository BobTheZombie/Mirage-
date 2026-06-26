# Mirage Input Layer

The early input layer provides a small bounded keyboard queue for built-in drivers.

## Queue behavior

The queue has fixed capacity and does not allocate in the IRQ path. When full, the oldest event is dropped and an overflow counter is incremented. Overflow is diagnostic, not fatal.

## Event model

Early keyboard events include key state, key code, modifiers, ASCII when known, raw source, and raw code. The model is intentionally smaller than evdev and can be extended later by supervised input services.
