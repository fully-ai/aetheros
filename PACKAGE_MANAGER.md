# AetherOS Native Package Manager (`aetherpkg`)

## 1. Architecture & Repository Layout (`userspace/bins/src/bin/aetherpkg.rs`)

`/bin/aetherpkg` is AetherOS's native Ring 3 ELF64 package manager featuring recursive dependency resolution, Castagnoli CRC32C payload integrity verification, and an atomic installed-package manifest database under `/var/lib/aetherpkg/installed.db`.

### Built-In System Package Catalog
| Package Name | Version | Dependencies | Installed Artifact | CRC32C Checksum |
| :--- | :--- | :--- | :--- | :--- |
| `aether-base` | `0.1.0` | *(none)* | `/etc/aether-base.manifest` | `0xf4529967` |
| `aether-devtools` | `0.1.0` | `aether-base` | `/etc/aether-devtools.conf` | `0x235af06a` |
| `aether-netutils` | `0.1.0` | `aether-base` | `/etc/hosts` | *(computed & verified on install)* |

---

## 2. Subcommands & Verification Workflow

- `aetherpkg list`: Displays all catalog packages, version numbers, installation status (`installed` / `available`), and descriptions.
- `aetherpkg info <pkg>`: Displays package version, dependency list, target path, expected CRC32C checksum, and current install state.
- `aetherpkg install <pkg>`:
  1. Recursively resolves and installs any missing dependencies first (e.g., `aether-devtools` requires `aether-base`).
  2. Computes the Castagnoli CRC32C checksum of the package payload and verifies it before writing the target file.
  3. Re-reads the written file from VFS to confirm on-disk CRC32C integrity.
  4. Records `name:version:target_path:crc32c_hex` in `/var/lib/aetherpkg/installed.db`.
- `aetherpkg verify`:
  Iterates through `/var/lib/aetherpkg/installed.db`, reads every installed package's target file from disk, recomputes its CRC32C checksum, and reports `[OK]` or `[CORRUPTED]`.
- `aetherpkg remove <pkg>`:
  Unlinks the package artifact and removes its entry from `/var/lib/aetherpkg/installed.db` (refusing to remove `aether-base`).
