CARGO ?= cargo
RUSTUP ?= rustup
RUSTC_BOOTSTRAP ?= 1
LIMINE_VERSION ?= v12.3.2
LIMINE_VERSION_NUMBER := $(patsubst v%,%,$(LIMINE_VERSION))
LIMINE_URL := https://github.com/limine-bootloader/limine/releases/download/$(LIMINE_VERSION)/limine-binary.tar.xz
TARGET_JSON := targets/x86_64-mirage.json
KERNEL_ELF := target/x86_64-mirage/release/mirage-kernel
BUILD_DIR := build
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO_IMAGE := $(BUILD_DIR)/mirage.iso
LIMINE_DIR := $(BUILD_DIR)/limine
LIMINE_BIN := $(LIMINE_DIR)/limine

.PHONY: all kernel iso run-qemu clean limine rust-src

all: iso

rust-src:
	$(RUSTUP) component add rust-src

kernel: rust-src
	RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) $(CARGO) build --release --bin mirage-kernel \
		--target $(TARGET_JSON) \
		-Z build-std=core,compiler_builtins \
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
	cp $(KERNEL_ELF) $(ISO_ROOT)/boot/mirage-kernel.elf
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
	qemu-system-x86_64 -M q35 -m 256M -cdrom $(ISO_IMAGE) -serial stdio -display none -no-reboot -no-shutdown

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)
