RUSTUP ?= rustup
CARGO ?= $(shell $(RUSTUP) which cargo 2>/dev/null || which cargo)
RUSTC ?= $(shell $(RUSTUP) which rustc 2>/dev/null || which rustc)
RUSTC_BOOTSTRAP ?= 1
LIMINE_VERSION ?= v12.3.2
LIMINE_VERSION_NUMBER := $(patsubst v%,%,$(LIMINE_VERSION))
LIMINE_URL := https://github.com/limine-bootloader/limine/releases/download/$(LIMINE_VERSION)/limine-binary.tar.xz
BUILD_DIR := build
CONFIG_SCHEMA := config/MirageConfig.toml
CONFIG_FILE ?= mirage.conf
CONFIG_OUT_DIR := target/mirage/config
CONFIG_RS := $(CONFIG_OUT_DIR)/generated.rs
CONFIG_CARGO_ENV := $(CONFIG_OUT_DIR)/cargo_features.env
CONFIG_BUILD_ENV := $(CONFIG_OUT_DIR)/build_flags.env
MIRAGECONFIG := $(CARGO) run -q -p mirageconfig --
TARGET_JSON_SOURCE := targets/x86_64-mirage.json
TARGET_JSON = $(BUILD_DIR)/targets/x86_64-mirage.json
CARGO_JSON_TARGET_SPEC_FLAG := $(shell RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) -Z help 2>/dev/null | sed -n "s/.*-Z json-target-spec.*/-Z json-target-spec/p")
UNSTABLE_OPTIONS_FLAG := -Z unstable-options
KERNEL_ELF := target/x86_64-mirage/release/mirage-kernel
SPIDER_RS_ELF := target/x86_64-mirage/release/spider-rs
# Backward-compatible manual feature variables. Leave empty to use mirage.conf.
KERNEL_FEATURES ?=
QEMU_FEATURES ?=
QEMU_MINIMAL_FEATURES ?= hw-framebuffer
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO_IMAGE := $(BUILD_DIR)/mirage.iso
LIMINE_DIR := $(BUILD_DIR)/limine
LIMINE_BIN := $(LIMINE_DIR)/limine

.PHONY: all build kernel spider-rs spider-rs-clean spider-rs-check spider-rt-tree spider-rt-image runtime-images userspace-spider-rs install-spider-rs qemu-spider qemu-kernel seed-rs-kernel qemu-seed-image qemu-seed qemu-seed-debug image iso qemu qemu-headless qemu-debug qemu-emergency milestone-boot-screen qemu-check run-qemu run-qemu-headless run-qemu-debug smoke-x86_64-boot clean distclean limine rust-src check-rust-src target-json FORCE mirageconfig defconfig oldconfig savedefconfig listconfig checkconfig config-generate config-check config-print qemu-ahci-sata qemu-ahci-atapi qemu-ahci-gpt qemu-ahci-mbr qemu-bootrt qemu-spider-bootrt qemu-spider-rt menuconfig nconfig olddefconfig

all: iso

build: kernel

spider-rs: userspace-spider-rs
	mkdir -p $(BUILD_DIR)/userspace
	cp $(SPIDER_RS_ELF) $(BUILD_DIR)/userspace/spider-rs

spider-rs-clean:
	rm -f $(BUILD_DIR)/userspace/spider-rs
	rm -rf $(BUILD_DIR)/spider-rt $(BUILD_DIR)/spider-rt.img

spider-rs-check: rust-src check-rust-src $(TARGET_JSON)
	RUSTC=$(RUSTC) RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) check --release -p spider-rs --bin spider-rs --no-default-features \
		--target $(TARGET_JSON) \
		$(CARGO_JSON_TARGET_SPEC_FLAG) \
		$(UNSTABLE_OPTIONS_FLAG) \
		-Z build-std=core,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem

userspace-spider-rs: rust-src check-rust-src $(TARGET_JSON)
	RUSTC=$(RUSTC) RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) build --release -p spider-rs --bin spider-rs --no-default-features \
		--target $(TARGET_JSON) \
		$(CARGO_JSON_TARGET_SPEC_FLAG) \
		$(UNSTABLE_OPTIONS_FLAG) \
		-Z build-std=core,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem

install-spider-rs: spider-rs
	mkdir -p $(BUILD_DIR)/rootfs/sbin $(BUILD_DIR)/rootfs/etc/spider/system
	cp $(BUILD_DIR)/userspace/spider-rs $(BUILD_DIR)/rootfs/sbin/spider-rs
	cp userspace/spider-rs/units/* $(BUILD_DIR)/rootfs/etc/spider/system/

spider-rt-tree: spider-rs
	rm -rf $(BUILD_DIR)/spider-rt
	mkdir -p $(BUILD_DIR)/spider-rt/sbin $(BUILD_DIR)/spider-rt/etc/spider
	cp $(BUILD_DIR)/userspace/spider-rs $(BUILD_DIR)/spider-rt/sbin/spider-rs
	cp userspace/spider-rs/units/default.target $(BUILD_DIR)/spider-rt/etc/spider/default.target
	cp userspace/spider-rs/units/basic.target $(BUILD_DIR)/spider-rt/etc/spider/basic.target
	cp userspace/spider-rs/units/emergency.target $(BUILD_DIR)/spider-rt/etc/spider/emergency.target
	printf 'name=spider-rt\nentry=/sbin/spider-rs\n' > $(BUILD_DIR)/spider-rt/manifest

spider-rt-image: spider-rt-tree
	$(CARGO) run -q -p mk-runtime-image -- $(BUILD_DIR)/spider-rt $(BUILD_DIR)/spider-rt.img --name spider-rt --entry /sbin/spider-rs

runtime-images: spider-rt-image

qemu-spider: install-spider-rs image
	@echo "Spider-rs PID 1 ELF staged at $(BUILD_DIR)/rootfs/sbin/spider-rs; boot remains honest Stub until QFS/rootfs byte-read and ring-3 entry are wired."
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh

mirageconfig:
	$(MIRAGECONFIG) --menu --config $(CONFIG_FILE) --generate

menuconfig: mirageconfig

nconfig: mirageconfig

olddefconfig: oldconfig

defconfig:
	$(MIRAGECONFIG) --defconfig --config $(CONFIG_FILE) --generate

oldconfig:
	$(MIRAGECONFIG) --oldconfig --config $(CONFIG_FILE) --generate

savedefconfig:
	$(MIRAGECONFIG) --savedefconfig --config $(CONFIG_FILE) --output mirage.defconfig

listconfig:
	$(MIRAGECONFIG) --list

checkconfig: config-check

config-generate: $(CONFIG_RS) $(CONFIG_CARGO_ENV) $(CONFIG_BUILD_ENV)

$(CONFIG_RS) $(CONFIG_CARGO_ENV) $(CONFIG_BUILD_ENV): $(CONFIG_SCHEMA) FORCE
	@set -eu; \
	if [ ! -f "$(CONFIG_FILE)" ]; then \
		echo "$(CONFIG_FILE) is missing; creating default configuration"; \
		$(MAKE) defconfig; \
	else \
		$(MIRAGECONFIG) --oldconfig --config $(CONFIG_FILE) --generate; \
	fi

config-check: config-generate
	$(MIRAGECONFIG) --check --config $(CONFIG_FILE) --generate

config-print: config-generate
	@set -eu; \
	printf 'Config file: %s\n' "$(CONFIG_FILE)"; \
	printf 'Generated artifacts: %s\n' "$(CONFIG_OUT_DIR)"; \
	if [ -n "$${MIRAGE_FEATURES:-}" ]; then \
		echo "Using manual MIRAGE_FEATURES override instead of mirage.conf"; \
		printf 'Cargo features: %s\n' "$$MIRAGE_FEATURES"; \
	else \
		. "$(CONFIG_CARGO_ENV)"; \
		printf 'Cargo features: %s\n' "$$MIRAGE_FEATURES"; \
	fi; \
	. "$(CONFIG_BUILD_ENV)"; \
	printf 'QEMU display args: %s\n' "$${MIRAGE_QEMU_DISPLAY_ARGS:-}"; \
	printf 'QEMU serial args: %s\n' "$${MIRAGE_QEMU_SERIAL_ARGS:-}"; \
	printf 'QEMU debug args: %s\n' "$${MIRAGE_QEMU_DEBUG_ARGS:-}"; \
	printf 'Kernel cmdline: %s\n' "$${MIRAGE_KERNEL_CMDLINE:-}"

rust-src:
	@set -eu; \
	sysroot="$$($(RUSTC) --print sysroot)"; \
	src_lock="$$sysroot/lib/rustlib/src/rust/library/Cargo.lock"; \
	if [ -f "$$src_lock" ]; then \
		echo "rust-src already available for $(RUSTC) at $$src_lock"; \
	elif printf '%s\n' "$$sysroot" | grep -q '/toolchains/'; then \
		toolchain="$${sysroot##*/toolchains/}"; \
		$(RUSTUP) component add rust-src --toolchain "$$toolchain"; \
	else \
		echo "error: $(RUSTC) sysroot $$sysroot is not managed by rustup and lacks $$src_lock" >&2; \
		echo "error: install matching Rust source for that compiler, or use rustup-managed CARGO/RUSTC from the same toolchain" >&2; \
		exit 1; \
	fi

check-rust-src:
	@set -eu; \
	sysroot="$$($(RUSTC) --print sysroot)"; \
	src_lock="$$sysroot/lib/rustlib/src/rust/library/Cargo.lock"; \
	if [ ! -f "$$src_lock" ]; then \
		echo "error: rust-src is missing for $(RUSTC) (expected $$src_lock)" >&2; \
		echo "error: run 'make rust-src' or use matching rustup-managed CARGO/RUSTC tools before building with -Z build-std" >&2; \
		exit 1; \
	fi

target-json: $(TARGET_JSON)

FORCE:

$(TARGET_JSON): $(TARGET_JSON_SOURCE) FORCE
	@set -eu; \
	mkdir -p "$(@D)"; \
	cp "$<" "$@.tmp.json"; \
	if RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(RUSTC) $(UNSTABLE_OPTIONS_FLAG) - --target "$@.tmp.json" --print cfg >/dev/null 2>&1 < /dev/null; then \
		mv "$@.tmp.json" "$@"; \
	else \
		sed 's/"target-pointer-width": "64"/"target-pointer-width": 64/' "$<" > "$@.tmp.json"; \
		RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(RUSTC) $(UNSTABLE_OPTIONS_FLAG) - --target "$@.tmp.json" --print cfg >/dev/null < /dev/null; \
		mv "$@.tmp.json" "$@"; \
	fi

kernel: config-generate rust-src check-rust-src $(TARGET_JSON)
	@set -eu; \
	if [ -n "$${MIRAGE_FEATURES:-}" ]; then \
		echo "Using manual MIRAGE_FEATURES override instead of mirage.conf"; \
		features="$$MIRAGE_FEATURES"; \
	elif [ -n "$(KERNEL_FEATURES)" ]; then \
		echo "Using legacy KERNEL_FEATURES override instead of mirage.conf"; \
		features="$(KERNEL_FEATURES)"; \
	elif [ -f "$(CONFIG_CARGO_ENV)" ]; then \
		. "$(CONFIG_CARGO_ENV)"; \
		features="$$MIRAGE_FEATURES"; \
	else \
		features=""; \
	fi; \
	RUSTC=$(RUSTC) RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) build --release --no-default-features --features "$$features" --bin mirage-kernel \
		--target $(TARGET_JSON) \
		$(CARGO_JSON_TARGET_SPEC_FLAG) \
		$(UNSTABLE_OPTIONS_FLAG) \
		-Z build-std=core,alloc,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem

qemu-kernel: config-generate rust-src check-rust-src $(TARGET_JSON)
	@set -eu; \
	if [ -n "$${MIRAGE_FEATURES:-}" ]; then \
		echo "Using manual MIRAGE_FEATURES override instead of mirage.conf"; \
		features="$$MIRAGE_FEATURES"; \
	elif [ -n "$(QEMU_FEATURES)" ]; then \
		echo "Using legacy QEMU_FEATURES override instead of mirage.conf"; \
		features="$(QEMU_FEATURES)"; \
	elif [ -f "$(CONFIG_CARGO_ENV)" ]; then \
		. "$(CONFIG_CARGO_ENV)"; \
		features="$$MIRAGE_FEATURES"; \
	else \
		features=""; \
	fi; \
	RUSTC=$(RUSTC) RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) build --release --no-default-features --features "$$features" --bin mirage-kernel \
		--target $(TARGET_JSON) \
		$(CARGO_JSON_TARGET_SPEC_FLAG) \
		$(UNSTABLE_OPTIONS_FLAG) \
		-Z build-std=core,alloc,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem

seed-rs-kernel: qemu-kernel

limine: $(LIMINE_BIN)

$(LIMINE_BIN):
	rm -rf $(LIMINE_DIR) $(BUILD_DIR)/limine-binary.tar.xz
	mkdir -p $(LIMINE_DIR)
	curl -L --fail -o $(BUILD_DIR)/limine-binary.tar.xz $(LIMINE_URL)
	tar -xf $(BUILD_DIR)/limine-binary.tar.xz -C $(LIMINE_DIR) --strip-components=1
	$(MAKE) -C $(LIMINE_DIR)

image: config-generate iso

iso: config-generate qemu-kernel runtime-images limine
	rm -rf $(ISO_ROOT)
	mkdir -p $(ISO_ROOT)/boot/limine $(ISO_ROOT)/EFI/BOOT
	cp $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	./tools/verify-seed-rs-elf.sh $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	cp boot/limine/limine.conf $(ISO_ROOT)/boot/limine/limine.conf
	cp $(BUILD_DIR)/spider-rt.img $(ISO_ROOT)/spider-rt.img
	cp $(LIMINE_DIR)/limine-bios.sys $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_ROOT)/EFI/BOOT/BOOTX64.EFI
	cp $(LIMINE_DIR)/BOOTIA32.EFI $(ISO_ROOT)/EFI/BOOT/BOOTIA32.EFI
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		-o $(ISO_IMAGE) $(ISO_ROOT)
	$(LIMINE_BIN) bios-install $(ISO_IMAGE)

qemu-seed-image: seed-rs-kernel limine
	rm -rf $(ISO_ROOT)
	mkdir -p $(ISO_ROOT)/boot/limine $(ISO_ROOT)/EFI/BOOT
	cp $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	cp boot/limine/limine.conf $(ISO_ROOT)/boot/limine/limine.conf
	cp $(LIMINE_DIR)/limine-bios.sys $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_ROOT)/boot/limine/
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_ROOT)/EFI/BOOT/BOOTX64.EFI
	cp $(LIMINE_DIR)/BOOTIA32.EFI $(ISO_ROOT)/EFI/BOOT/BOOTIA32.EFI
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		-o $(ISO_IMAGE) $(ISO_ROOT)
	$(LIMINE_BIN) bios-install $(ISO_IMAGE)

qemu-seed: qemu-seed-image
	./tools/verify-seed-rs-elf.sh $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	qemu-system-x86_64 -machine q35 -m 512M -serial stdio -display none -no-reboot -no-shutdown -d int,cpu_reset -D build/qemu.log -cdrom $(ISO_IMAGE)

qemu-seed-debug: qemu-seed-image
	./tools/verify-seed-rs-elf.sh $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	qemu-system-x86_64 -machine q35 -m 512M -serial stdio -display none -no-reboot -no-shutdown -d int,cpu_reset -D build/qemu.log -S -s -cdrom $(ISO_IMAGE)

qemu: config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh

qemu-headless: config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu-headless.sh

qemu-debug: config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu-debug.sh

qemu-emergency: override MIRAGE_FEATURES :=
qemu-emergency: override QEMU_FEATURES := emergency-boot
qemu-emergency: config-generate image
	QEMU_FEATURES=emergency-boot \
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh

# Mirage Boot Milestone 1.0: framebuffer status screen, serial stdio, QEMU log.
milestone-boot-screen: override QEMU_FEATURES := hw-framebuffer
milestone-boot-screen: config-generate image
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh


qemu-keyboard-ps2: override QEMU_FEATURES := hw-ps2-keyboard
qemu-keyboard-ps2: config-generate image
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh


qemu-usb-none: override QEMU_FEATURES := hw-usb-hid
qemu-usb-none: config-generate image
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh

qemu-usb-kbd: qemu-keyboard-usb

qemu-usb-xhci: override QEMU_FEATURES := hw-usb-hid
qemu-usb-xhci: config-generate image
	MIRAGE_QEMU_EXTRA_ARGS="-device qemu-xhci" \
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh

qemu-usb-keyboard: qemu-keyboard-usb

qemu-keyboard-usb: override QEMU_FEATURES := hw-usb-hid
qemu-keyboard-usb: config-generate image
	MIRAGE_QEMU_EXTRA_ARGS="-device qemu-xhci -device usb-kbd" \
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh

qemu-keyboard-all: override QEMU_FEATURES := hw-ps2-keyboard,hw-usb-hid,hw-laptop-hotkeys
qemu-keyboard-all: config-generate image
	MIRAGE_QEMU_EXTRA_ARGS="-device qemu-xhci -device usb-kbd" \
	MIRAGE_QEMU_SERIAL_ARGS="-serial stdio" \
	MIRAGE_QEMU_DEBUG_ARGS="-d int,cpu_reset -D build/qemu.log" \
	MIRAGE_REUSE_IMAGE=1 \
	MIRAGE_ISO_IMAGE=$(ISO_IMAGE) \
	tools/run-qemu.sh

qemu-check: tools/check-qemu-image.sh
	./tools/check-qemu-image.sh

run-qemu: qemu

run-qemu-headless: qemu-headless

run-qemu-debug: qemu-debug

smoke-x86_64-boot: scripts/x86_64-boot-smoke.sh
	BUILD_KERNEL=1 KERNEL_ELF=$(KERNEL_ELF) ./scripts/x86_64-boot-smoke.sh

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR) $(CONFIG_OUT_DIR)

distclean: clean
	@if [ "$(CONFIG_CLEAN)" = "1" ]; then \
		rm -f $(CONFIG_FILE); \
		echo "Removed $(CONFIG_FILE) because CONFIG_CLEAN=1"; \
	else \
		echo "Preserved $(CONFIG_FILE) (use CONFIG_CLEAN=1 to remove it)"; \
	fi

qfs-image:
	mkdir -p $(BUILD_DIR)
	$(CARGO) run --features qfs-std --bin qfsprogs -- mkfs $(BUILD_DIR)/qfs.img
	$(CARGO) run --features qfs-std --bin qfsprogs -- fsck $(BUILD_DIR)/qfs.img

$(BUILD_DIR)/sata-qfs.img:
	mkdir -p $(BUILD_DIR)
	$(CARGO) run --features qfs-std --bin qfsprogs -- mkfs $@

$(BUILD_DIR)/nvme-qfs.img:
	mkdir -p $(BUILD_DIR)
	$(CARGO) run --features qfs-std --bin qfsprogs -- mkfs $@


$(BUILD_DIR)/sata.img:
	mkdir -p $(BUILD_DIR)
	truncate -s 64M $@

qemu-ahci-disk: override QEMU_FEATURES := hw-ahci
qemu-ahci-disk: $(BUILD_DIR)/sata.img config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(BUILD_DIR)/sata.img,if=none,id=sata0,format=raw -device ich9-ahci,id=ahci -device ide-hd,drive=sata0,bus=ahci.0" tools/run-qemu.sh

qemu-ahci-qfs: $(BUILD_DIR)/sata-qfs.img image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_KERNEL_CMDLINE="root=sata0" MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(BUILD_DIR)/sata-qfs.img,if=none,id=sata0,format=raw -device ich9-ahci,id=ahci -device ide-hd,drive=sata0,bus=ahci.0" tools/run-qemu.sh

qemu-nvme-qfs: $(BUILD_DIR)/nvme-qfs.img image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_KERNEL_CMDLINE="root=nvme0n1" MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(BUILD_DIR)/nvme-qfs.img,if=none,id=nvme0,format=raw -device nvme,drive=nvme0,serial=mirage-nvme0" tools/run-qemu.sh

qemu-m2-qfs: qemu-nvme-qfs

EXT4_ROOT_IMAGE := $(BUILD_DIR)/ext4-root.img
EXT4_ROOT_SIZE ?= 128M

.PHONY: ext4-image qemu-ahci-disk qemu-ahci-ext4 qemu-nvme-ext4

ext4-image: spider-rs
	@set -eu; \
	mkdir -p $(BUILD_DIR)/ext4-staging/sbin $(BUILD_DIR)/ext4-staging/etc/spider/system $(BUILD_DIR)/ext4-staging/bin; \
	cp $(BUILD_DIR)/rootfs/sbin/spider-rs $(BUILD_DIR)/ext4-staging/sbin/spider-rs; \
	cp userspace/spider-rs/units/* $(BUILD_DIR)/ext4-staging/etc/spider/system/; \
	printf '#!/bin/sh\necho hello from GNU/Mirage ext4 rootfs\n' > $(BUILD_DIR)/ext4-staging/bin/hello; \
	chmod +x $(BUILD_DIR)/ext4-staging/bin/hello; \
	truncate -s $(EXT4_ROOT_SIZE) $(EXT4_ROOT_IMAGE); \
	if command -v mkfs.ext4 >/dev/null 2>&1; then mkfs.ext4 -F -O '^has_journal' -d $(BUILD_DIR)/ext4-staging $(EXT4_ROOT_IMAGE); \
	elif command -v mke2fs >/dev/null 2>&1; then mke2fs -t ext4 -F -O '^has_journal' -d $(BUILD_DIR)/ext4-staging $(EXT4_ROOT_IMAGE); \
	else echo 'mkfs.ext4 or mke2fs is required to create $(EXT4_ROOT_IMAGE)' >&2; exit 1; fi; \
	echo 'Created $(EXT4_ROOT_IMAGE) with /sbin/spider-rs, Spider units, and /bin/hello (non-journaled for Mirage-safe rw experiments).'

qemu-ahci-ext4: ext4-image image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_KERNEL_CMDLINE="root=ext4:sata0" MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(EXT4_ROOT_IMAGE),if=none,id=sata0,format=raw -device ich9-ahci,id=ahci -device ide-hd,drive=sata0,bus=ahci.0" tools/run-qemu.sh

qemu-nvme-ext4: ext4-image image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_KERNEL_CMDLINE="root=ext4:nvme0n1" MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(EXT4_ROOT_IMAGE),if=none,id=nvme0,format=raw -device nvme,drive=nvme0,serial=mirage-nvme0" tools/run-qemu.sh


qemu-ahci-sata: qemu-ahci-disk

$(BUILD_DIR)/atapi.iso:
	mkdir -p $(BUILD_DIR)/atapi-root
	printf '%s\n' 'Mirage ATAPI test media' > $(BUILD_DIR)/atapi-root/README.TXT
	if command -v xorriso >/dev/null 2>&1; then xorriso -as mkisofs -o $@ $(BUILD_DIR)/atapi-root; else echo 'xorriso is required for ATAPI media' >&2; exit 1; fi

qemu-ahci-atapi: override QEMU_FEATURES := hw-ahci
qemu-ahci-atapi: $(BUILD_DIR)/atapi.iso config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(BUILD_DIR)/atapi.iso,if=none,id=cd0,format=raw,media=cdrom -device ich9-ahci,id=ahci -device ide-cd,drive=cd0,bus=ahci.0" tools/run-qemu.sh

$(BUILD_DIR)/sata-mbr.img: $(BUILD_DIR)/sata.img
	cp $(BUILD_DIR)/sata.img $@
	printf '\200' | dd of=$@ bs=1 seek=446 conv=notrunc status=none
	printf '\203' | dd of=$@ bs=1 seek=450 conv=notrunc status=none
	printf '\001\000\000\000' | dd of=$@ bs=1 seek=454 conv=notrunc status=none
	printf '\377\007\000\000' | dd of=$@ bs=1 seek=458 conv=notrunc status=none
	printf '\125\252' | dd of=$@ bs=1 seek=510 conv=notrunc status=none

qemu-ahci-mbr: override QEMU_FEATURES := hw-ahci
qemu-ahci-mbr: $(BUILD_DIR)/sata-mbr.img config-generate image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) MIRAGE_KERNEL_CMDLINE="root=sata0p1" MIRAGE_QEMU_EXTRA_ARGS="-drive file=$(BUILD_DIR)/sata-mbr.img,if=none,id=sata0,format=raw -device ich9-ahci,id=ahci -device ide-hd,drive=sata0,bus=ahci.0" tools/run-qemu.sh

qemu-ahci-gpt: qemu-ahci-mbr

qemu-bootrt: image
	@echo "Boot Runtime image format is kernel-supported; stage a bootrt Limine module to exercise this target."
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh

qemu-spider-bootrt: install-spider-rs image
	@echo "Expected: Spider Runtime module is staged, RuntimeVfs mounts /spider-rt, and Spider-rs remains Started/Stub until ring-3 output is confirmed."
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh

qemu-spider-rt: runtime-images image
	MIRAGE_REUSE_IMAGE=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh
