#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

error() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

command -v qemu-system-x86_64 >/dev/null 2>&1 || error "missing required command 'qemu-system-x86_64'"

cd "$repo_root"

iso_image=${MIRAGE_ISO_IMAGE:-build/mirage.iso}
reuse_image=${MIRAGE_REUSE_IMAGE:-0}

if [ "${MIRAGE_SKIP_BUILD:-0}" = "1" ]; then
    printf 'warning: MIRAGE_SKIP_BUILD=1 is deprecated; use MIRAGE_REUSE_IMAGE=1 to launch an existing image without rebuilding\n' >&2
    reuse_image=1
fi

case "$reuse_image" in
    1)
        [ -f "$iso_image" ] || error "MIRAGE_REUSE_IMAGE=1 requires existing QEMU image '$iso_image' (run tools/build-qemu-image.sh or unset MIRAGE_REUSE_IMAGE)"
        ;;
    0|'')
        "$script_dir/build-qemu-image.sh"
        [ -f "$iso_image" ] || error "missing QEMU image '$iso_image' after running tools/build-qemu-image.sh"
        ;;
    *)
        error "MIRAGE_REUSE_IMAGE must be '1' to reuse an existing image or unset to rebuild"
        ;;
esac

mem=${MIRAGE_QEMU_MEMORY:-512M}

qemu_accepts_cpu() {
    cpu=$1
    qemu-system-x86_64 -cpu help 2>/dev/null | awk -v cpu="$cpu" '{ for (i = 1; i <= NF; i++) if ($i == cpu) found = 1 } END { exit found ? 0 : 1 }'
}

accel_args=''
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    accel_args='-enable-kvm -cpu host'
elif qemu_accepts_cpu max; then
    accel_args='-cpu max'
else
    accel_args='-cpu qemu64'
fi

find_ovmf_code() {
    if [ -n "${MIRAGE_OVMF_CODE:-}" ]; then
        [ -f "$MIRAGE_OVMF_CODE" ] || error "MIRAGE_OVMF_CODE points to missing file '$MIRAGE_OVMF_CODE'"
        printf '%s\n' "$MIRAGE_OVMF_CODE"
        return 0
    fi

    for candidate in \
        /usr/share/OVMF/OVMF_CODE.fd \
        /usr/share/edk2-ovmf/x64/OVMF_CODE.fd \
        /usr/share/qemu/OVMF.fd
    do
        if [ -f "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done

    return 1
}

image_has_limine_bios_support() {
    image=$1

    case "$image" in
        *.iso|*.ISO)
            if command -v xorriso >/dev/null 2>&1; then
                xorriso -indev "$image" -find /boot/limine/limine-bios.sys -print 2>/dev/null | grep -Fx '/boot/limine/limine-bios.sys' >/dev/null 2>&1 && return 0
            fi
            if command -v isoinfo >/dev/null 2>&1; then
                isoinfo -i "$image" -f 2>/dev/null | grep -Ei '^/BOOT/LIMINE/LIMINE-BIOS\.SYS(;[0-9]+)?$' >/dev/null 2>&1 && return 0
            fi
            # The default Mirage ISO target is built with Limine BIOS support; if
            # ISO inspection tools are unavailable, preserve that BIOS default.
            case "$image" in
                build/mirage.iso|*/build/mirage.iso) return 0 ;;
            esac
            ;;
        *)
            if command -v strings >/dev/null 2>&1; then
                strings "$image" 2>/dev/null | grep -F 'LIMINE' | grep -F 'BIOS' >/dev/null 2>&1 && return 0
            fi
            ;;
    esac

    return 1
}

image_requires_uefi() {
    image=$1

    case "${MIRAGE_QEMU_FIRMWARE:-auto}" in
        bios) return 1 ;;
        uefi) return 0 ;;
        auto) ;;
        *) error "MIRAGE_QEMU_FIRMWARE must be 'auto', 'bios', or 'uefi'" ;;
    esac

    case "$image" in
        *.efi|*.EFI|*uefi*|*UEFI*|*ovmf*|*OVMF*) return 0 ;;
    esac

    if image_has_limine_bios_support "$image"; then
        return 1
    fi

    return 0
}

firmware_args=''
if image_requires_uefi "$iso_image"; then
    ovmf_code=$(find_ovmf_code) || error "selected image '$iso_image' requires UEFI, but no OVMF firmware was found"
    firmware_args="-drive if=pflash,format=raw,readonly=on,file=$ovmf_code"
fi

# shellcheck disable=SC2086
exec qemu-system-x86_64 \
    -M q35 \
    -m "$mem" \
    $accel_args \
    $firmware_args \
    -cdrom "$iso_image" \
    -serial stdio \
    -no-reboot \
    -no-shutdown \
    "$@"
