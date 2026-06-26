# VirtualBox boot validation

VirtualBox validation must use an ephemeral VM or a clearly isolated Mirage VM with serial logs captured under `build/logs/`. It is acceptable to document an environment blocker such as missing `VBoxManage`, unavailable kernel modules, permission denial, or missing serial capture. It is not acceptable to skip MTSS, rootfs, PID1, or userspace to pass VirtualBox.
