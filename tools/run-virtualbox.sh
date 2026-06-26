#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd "$repo_root"

error() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

command -v VBoxManage >/dev/null 2>&1 || error "missing required command 'VBoxManage'"

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
reuse_image=${MIRAGE_REUSE_IMAGE:-0}
vm_name=${MIRAGE_VBOX_VM_NAME:-Mirage-boot-$USER-$$}
log_dir=${MIRAGE_VBOX_LOG_DIR:-build/logs}
serial_log=${MIRAGE_VBOX_SERIAL_LOG:-$log_dir/virtualbox-boot.log}
vm_log=${MIRAGE_VBOX_VM_LOG:-$log_dir/virtualbox-vm.log}
timeout_seconds=${MIRAGE_VBOX_TIMEOUT:-60}
mem=${MIRAGE_VBOX_MEMORY:-512}

mkdir -p "$log_dir"

case "$reuse_image" in
    1)
        [ -f "$iso_image" ] || error "MIRAGE_REUSE_IMAGE=1 requires existing ISO '$iso_image'"
        MIRAGE_ISO_IMAGE=$iso_image "$script_dir/validate-boot-runtime.sh"
        ;;
    0|'')
        "$script_dir/build-qemu-image.sh"
        [ -f "$iso_image" ] || error "missing ISO '$iso_image' after build"
        ;;
    *) error "MIRAGE_REUSE_IMAGE must be 0 or 1" ;;
esac

cleanup() {
    VBoxManage controlvm "$vm_name" poweroff >/dev/null 2>&1 || true
    VBoxManage unregistervm "$vm_name" --delete >/dev/null 2>&1 || true
}
trap cleanup EXIT INT HUP TERM

VBoxManage createvm --name "$vm_name" --ostype Other_64 --register >"$vm_log" 2>&1
VBoxManage modifyvm "$vm_name" \
    --memory "$mem" \
    --firmware efi \
    --acpi on \
    --ioapic on \
    --boot1 dvd \
    --boot2 none \
    --boot3 none \
    --boot4 none \
    --uart1 0x3f8 4 \
    --uartmode1 file "$serial_log" \
    --audio none \
    --usb off >>"$vm_log" 2>&1
VBoxManage storagectl "$vm_name" --name SATA --add sata --controller IntelAhci >>"$vm_log" 2>&1
VBoxManage storageattach "$vm_name" --storagectl SATA --port 0 --device 0 --type dvddrive --medium "$iso_image" >>"$vm_log" 2>&1
VBoxManage startvm "$vm_name" --type headless >>"$vm_log" 2>&1

elapsed=0
while [ "$elapsed" -lt "$timeout_seconds" ]; do
    if [ -f "$serial_log" ]; then
        if grep -Eq 'BOOT PROGRESS: 100%|CURRENT PHASE: BOOTED|IDLELOOP \[RUNNING\]|Guru Meditation|triple fault|Triple fault' "$serial_log"; then
            break
        fi
    fi
    sleep 1
    elapsed=$((elapsed + 1))
done

VBoxManage showvminfo "$vm_name" --machinereadable >>"$vm_log" 2>&1 || true

if [ -f "$serial_log" ] && grep -Eq 'Guru Meditation|triple fault|Triple fault' "$serial_log"; then
    error "VirtualBox boot failed; see $serial_log and $vm_log"
fi

if [ -f "$serial_log" ] && grep -q 'BOOT PROGRESS: 100%' "$serial_log" && grep -q 'CURRENT PHASE: BOOTED' "$serial_log"; then
    printf 'VirtualBox boot acceptance markers found in %s\n' "$serial_log"
    exit 0
fi

error "VirtualBox boot acceptance markers not observed within ${timeout_seconds}s; see $serial_log and $vm_log"
