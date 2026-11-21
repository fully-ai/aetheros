# AetherOS User Guide (`USER_GUIDE.md`)

## 1. Launching AetherOS Desktop
```bash
cd /home/ubeyt/Belgeler/AetherOS
./scripts/run.sh
```
This opens the QEMU SDL graphical desktop window (`1280x800`) and attaches the interactive serial console.

## 2. Using the Graphical Desktop (`Aether Desktop`)
- **Mouse Navigation**: Click once inside the QEMU window to capture the PS/2 mouse.
  - **Move Windows**: Drag any window by its top 24 px title bar.
  - **Resize Windows**: Drag the bottom-right corner grip of any window.
  - **Minimize / Maximize / Close**: Click `[-]` (yellow), `[+]` (green), or `[X]` (red) on the top-right of any window.
  - **Launch Applications**: Click any of the 8 **Desktop Shortcut Icons** on the left rail, or click **`[AETHER]`** (bottom-left Start Button) to open the Application Launcher.
  - **Switch Workspaces**: Click `[WS1]`, `[WS2]`, `[WS3]`, or `[WS4]` on the top bar.
  - **Quick Settings & Notifications**: Click `[QUICK]` or `[BELL(N)]` on the bottom-right system tray, or right-click anywhere on the wallpaper.

## 3. Working with USB Drives & Files (`Aether Files` & `Aether Disks`)
- On boot, `/dev/sdc1` (`AETHER_USB`, FAT32 over USB Mass Storage BOT/SCSI) is automatically mounted at `/mnt/usb/AETHER_USB` with `nosuid,nodev,noexec`.
- In **Aether Files**, click `USB Drive` on the left sidebar to browse `README.TXT`, `SYSINFO.TXT`, and `HELLO.SH`, click `[+File]` or `[+Folder]` to create new files synced to FAT32 clusters, or click `[Eject USB]` to flush caches and safely unmount the drive.

## 4. Browsing the Web (`Aether Browser`)
- Click `[aetheros.org]`, `[Docs]`, `[Store]`, or `[Example]` in **Aether Browser** to fetch pages over DNS + TCP + HTTP/HTTPS (`[TLS 1.3 OK]`).
- Click `[BadSSL!]` to verify how Aether Browser blocks expired X.509 certificates (`[TLS BLOCKED]`).
- Click `[Download]` to save the current page to `/home/aether/Downloads/page.txt`.
