# AetherOS Init Process & Service Supervision (`/sbin/init`)

## 1. `/sbin/init` Responsibilities (`userspace/bins/src/bin/init.rs`)

`/sbin/init` is a compiled Ring 3 ELF64 binary spawned as the first user-space supervisor process (`PID 1` in normal boot; `PID 11` in self-test verification). It performs four core functions:

1. **Filesystem & Runtime Verification**:
   - Ensures `/run`, `/var/log`, and `/tmp` exist.
   - Initializes `/etc/os-release` with release metadata (`NAME="AetherOS"`, `ID=aetheros`, `VERSION="0.1.0-alpha.1"`).
   - Opens `/var/log/init.log` for structured service event logging (`log_init`).
2. **Service Configuration Parsing (`/etc/services.conf`)**:
   - Creates `/etc/services.conf` if absent and parses colon-delimited service records:
     ```text
     # AetherOS /etc/services.conf (format: name:command:restart_policy:enabled)
     syshealth:/bin/uname -a:once:1
     aethersh:/bin/aethersh --boot-verify:once:1
     ```
3. **Service Supervision & Restart Policy**:
   - Spawns each enabled service via `fork()` + `execve()`, waits for completion (`wait4`), logs the child PID and exit status to both `/dev/console` and `/var/log/init.log`, and honors `restart_policy`:
     - `once`: Run once during boot initialization.
     - `on-failure`: Automatically restart up to 3 times if the service exits with a non-zero exit code.
     - `always`: Respawn continuously (used for the interactive console `/bin/aethersh`).
4. **Orphan & Zombie Process Reaping**:
   - Continuously reaps terminated child processes via `wait4(-1, ...)` so no zombie tasks accumulate in the kernel task table.
