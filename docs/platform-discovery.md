# Platform Discovery Notes

Mirage platform discovery records hardware facts before policy or service binding.
The Platform Registry is the single source for discovered CPU, ACPI, PCI, storage,
display, USB, and input candidates.

Current query helpers include:

- `platform_find_pci_by_id(vendor, device)`
- `platform_find_pci_by_class(class, subclass, prog_if)`
- `platform_iter_pci(callback)`
- `platform_find_xhci_controller()`
- `platform_find_amd_xhci_controller()`
- `platform_find_nvme_controller()`
- `platform_find_ahci_controller()`
- `platform_find_renoir_gpu()`
- `platform_has_amd_soc_device()`

PCI records include name, kind, location, vendor/device ID, class/subclass/prog-if,
header type, BAR metadata, and interrupt-line metadata when available.

Drivers must query the registry rather than rescanning PCI.
# Mirage Platform Discovery

Mirage separates hardware discovery from driver/service lifecycle management.
Platform discovery answers only one question: **what hardware exists on this booted machine?** It does not bind a driver, grant a capability, start a service, or mark that service online.

## Boot ownership

The boot sequence preserves three separate responsibilities:

1. **Platform discovery finds hardware.** It records CPU, PCI, ACPI, legacy I/O, USB, storage, display, and input facts in the platform registry and logs them with a `[Platform]` prefix.
2. **Mirage-dispatch-rs registers and dispatches compiled-in drivers/services.** A compiled-in driver may register with dispatch, but its probe path must consult platform facts before it starts hardware work.
3. **Boot Phase Manager reports lifecycle state.** Phase state is about driver/service lifecycle: registered, started, online, skipped, or failed. It must not be used as a substitute for hardware discovery.

The intended log order is:

```text
[Platform] device found: "...", location=..., id=...
[Dispatch] registered: ...
[phase] ... REGISTERED
[Dispatch] dispatching: ...
[phase] ... STARTED
[phase] ... OK/ONLINE/SKIPPED/FAILED
```

## Required platform log format

Every discovered hardware fact is logged before driver/service registration using this exact shape:

```text
[Platform] device found: "<device name>", location=<location>, id=<hardware id>
```

Examples:

```text
[Platform] device found: "AMD Ryzen 5 4500U", location=cpuid, id=AuthenticAMD family=0x17 model=0x60 stepping=0x1
[Platform] device found: "Renoir AMDGPU", location=pci 03:00.0, id=1002:1636
[Platform] device found: "AMD xHCI Controller", location=pci 04:00.3, id=1022:1639
[Platform] device found: "NVMe Controller", location=pci 01:00.0, id=144d:a808
[Platform] device found: "PS/2 Controller", location=i8042, id=0x60/0x64
```

## Platform registry

`mirage-platform` exposes a fixed-capacity, no-heap `PlatformRegistry<CAPACITY>`. It stores `PlatformDevice` records and supports:

- duplicate detection by exact device or location;
- query by `PlatformDeviceKind`;
- query by PCI vendor/device ID;
- query by `PlatformLocation`.

The registry is `no_std` compatible and can be used by early boot code, the supervisor, and dispatch probes without requiring heap allocation.

## Device found vs driver registered vs driver online

These events are intentionally different:

- **Device found**: platform discovery observed hardware and recorded a `PlatformDevice`.
- **Driver registered**: a compiled-in driver/service descriptor was accepted by Mirage-dispatch-rs.
- **Driver started**: dispatch selected the service for startup after feature/dependency/probe checks.
- **Driver online**: the service completed initialization and can serve requests.
- **Skipped**: the driver was compiled in but its probe found no relevant platform hardware, or a dependency/feature gate was unavailable.

The preferred policy is: **compiled-in drivers register with dispatch, probes consult the platform registry, and absent hardware returns `SKIPPED`/`NotPresent` rather than pretending to start.** Platform discovery still records hardware independently of driver availability.

## Location formats

### CPUID

CPU discovery uses:

```text
location=cpuid
```

AMD64 CPU IDs are logged as:

```text
id=AuthenticAMD family=0x<family> model=0x<model> stepping=0x<stepping>
```

### PCI

PCI functions use canonical bus/device/function format:

```text
location=pci bb:dd.f
```

`bb` and `dd` are two-digit lowercase hexadecimal bus and device numbers. `f` is the decimal PCI function number. The PCI hardware ID is:

```text
id=vvvv:dddd
```

where `vvvv` is vendor ID and `dddd` is device ID.

### I/O port and i8042

The legacy PS/2 controller is represented as an i8042 I/O-port location:

```text
location=i8042, id=0x60/0x64
```

## Dispatch usage

Mirage-dispatch-rs should not rediscover hardware. Driver/service probes should receive or otherwise access the platform registry and ask targeted questions, for example:

- AMDGPU Renoir: `find_by_pci_id(0x1002, 0x1636)` or `find_by_pci_id(0x1002, 0x1638)`.
- PS/2 keyboard: `find_by_location(PlatformLocation::IoPort { base: 0x60 })`.
- USB keyboard: require the USB core/xHCI platform device to be present before probing HID devices.
- NVMe/AHCI: consult storage PCI identities or class-derived platform records before registering hardware resources.

A dispatch probe that finds no matching platform record should return `NotPresent`, and Boot Phase Manager should report the service as `SKIPPED` rather than `FAILED`.

## Authoritative registry update (2026-06-12)

The Platform Registry is the authoritative handoff from early discovery to driver/service probing. PCI is enumerated into the registry once in the x86_64 bring-up path, and later AHCI, NVMe, xHCI, Renoir GPU, and AMD SoC decisions query registry helpers instead of rescanning PCI.

Registry helpers now include `find_pci_by_class`, `find_by_pci_id`, `find_ahci`, `find_nvme`, `find_xhci`, `find_amdgpu_renoir`, and `find_amd_soc_device`.
