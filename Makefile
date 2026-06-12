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
# Backward-compatible manual feature variables. Leave empty to use mirage.conf.
KERNEL_FEATURES ?=
QEMU_FEATURES ?=
QEMU_MINIMAL_FEATURES ?= hw-framebuffer
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO_IMAGE := $(BUILD_DIR)/mirage.iso
LIMINE_DIR := $(BUILD_DIR)/limine
LIMINE_BIN := $(LIMINE_DIR)/limine

.PHONY: all build kernel spider-rs spider-rs-check spider-rs-host-test qemu-kernel seed-rs-kernel qemu-seed-image qemu-seed qemu-seed-debug image iso qemu qemu-headless qemu-debug qemu-emergency milestone-boot-screen qemu-check run-qemu run-qemu-headless run-qemu-debug smoke-x86_64-boot clean distclean limine rust-src check-rust-src target-json FORCE mirageconfig defconfig oldconfig savedefconfig listconfig checkconfig config-generate config-check config-print

all: iso

build: kernel

spider-rs:
	$(CARGO) build -p spider-rs
	mkdir -p $(BUILD_DIR)/rootfs/sbin $(BUILD_DIR)/rootfs/etc/spider/system
	cp target/debug/spider-rs $(BUILD_DIR)/rootfs/sbin/spider-rs
	cp userspace/spider-rs/units/* $(BUILD_DIR)/rootfs/etc/spider/system/

spider-rs-check:
	$(CARGO) check -p spider-rs

spider-rs-host-test:
	$(CARGO) test -p spider-rs

mirageconfig:
	$(MIRAGECONFIG) --menu --config $(CONFIG_FILE) --generate

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

iso: config-generate qemu-kernel limine
	rm -rf $(ISO_ROOT)
	mkdir -p $(ISO_ROOT)/boot/limine $(ISO_ROOT)/EFI/BOOT
	cp $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
	./tools/verify-seed-rs-elf.sh $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel
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
