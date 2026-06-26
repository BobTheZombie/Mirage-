# QEMU boot validation

Run clean and reused-image boots with:

```sh
MIRAGE_REUSE_IMAGE=0 MIRAGE_ISO_IMAGE=build/mirage.iso tools/run-qemu.sh
MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=build/mirage.iso tools/run-qemu.sh
```

Acceptance requires serial/log markers for the real completed states. Do not claim full boot if userspace/ring3 is still pending.
