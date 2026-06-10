RUSTUP ?= rustup
CARGO ?= $(shell $(RUSTUP) which cargo 2>/dev/null || which cargo)
RUSTC ?= $(shell $(RUSTUP) which rustc 2>/dev/null || which rustc)
RUSTC_BOOTSTRAP ?= 1
LIMINE_VERSION ?= v12.3.2
LIMINE_VERSION_NUMBER := $(patsubst v%,%,$(LIMINE_VERSION))
LIMINE_URL := https://github.com/limine-bootloader/limine/releases/download/$(LIMINE_VERSION)/limine-binary.tar.xz
TARGET_JSON_SOURCE := targets/x86_64-mirage.json
TARGET_JSON = $(BUILD_DIR)/targets/x86_64-mirage.json
CARGO_JSON_TARGET_SPEC_FLAG := $(shell RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) -Z help 2>/dev/null | sed -n "s/.*-Z json-target-spec.*/-Z json-target-spec/p")
UNSTABLE_OPTIONS_FLAG := -Z unstable-options
KERNEL_ELF := target/x86_64-mirage/release/mirage-kernel
KERNEL_FEATURES ?= hw-framebuffer full-boot
BUILD_DIR := build
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO_IMAGE := $(BUILD_DIR)/mirage.iso
LIMINE_DIR := $(BUILD_DIR)/limine
LIMINE_BIN := $(LIMINE_DIR)/limine

.PHONY: all kernel iso run-qemu smoke-x86_64-boot clean limine rust-src check-rust-src target-json FORCE

all: iso

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

kernel: rust-src check-rust-src $(TARGET_JSON)
	RUSTC=$(RUSTC) RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) build --release --no-default-features --features "$(KERNEL_FEATURES)" --bin mirage-kernel \
		--target $(TARGET_JSON) \
		$(CARGO_JSON_TARGET_SPEC_FLAG) \
		$(UNSTABLE_OPTIONS_FLAG) \
		-Z build-std=core,alloc,compiler_builtins \
		-Z build-std-features=compiler-builtins-mem

limine: $(LIMINE_BIN)

$(LIMINE_BIN):
	rm -rf $(LIMINE_DIR) $(BUILD_DIR)/limine-binary.tar.xz
	mkdir -p $(LIMINE_DIR)
	curl -L --fail -o $(BUILD_DIR)/limine-binary.tar.xz $(LIMINE_URL)
	tar -xf $(BUILD_DIR)/limine-binary.tar.xz -C $(LIMINE_DIR) --strip-components=1
	$(MAKE) -C $(LIMINE_DIR)

iso: kernel limine
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

run-qemu: iso
	MIRAGE_SKIP_BUILD=1 MIRAGE_ISO_IMAGE=$(ISO_IMAGE) tools/run-qemu.sh

smoke-x86_64-boot: scripts/x86_64-boot-smoke.sh
	BUILD_KERNEL=1 KERNEL_ELF=$(KERNEL_ELF) ./scripts/x86_64-boot-smoke.sh

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)
