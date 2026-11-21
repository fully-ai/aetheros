# AetherOS Troubleshooting Guide (`TROUBLESHOOTING.md`)

## 1. QEMU Disk Image Write Lock (`Failed to get "write" lock`)
- **Symptom**: `qemu-system-x86_64: Failed to get "write" lock — Is another process using the image [build/aetheros.img]?`
- **Resolution**: Another QEMU instance is already running with `build/aetheros.img`. Close the existing QEMU window (or run `pkill -f qemu-system-x86_64`) before starting a new `./scripts/run.sh` session.

## 2. Releasing Mouse & Keyboard Grab in QEMU SDL Window
- **Symptom**: Clicking inside the QEMU graphical window captures the host mouse cursor.
- **Resolution**: Press **`Ctrl + Alt + G`** to release the mouse/keyboard grab back to the host desktop.

## 3. Executing Scripts on Removable USB Drives (`EACCES`)
- **Symptom**: Running `/mnt/usb/AETHER_USB/HELLO.SH` directly fails with permission denied (`EACCES`).
- **Resolution**: Removable USB drives are automatically mounted with **`MS_NOSUID | MS_NODEV | MS_NOEXEC`** for security. Copy trusted scripts to `/home/aether/` and mark them executable (`chmod 0755`) if execution is intended.
