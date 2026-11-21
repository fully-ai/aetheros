# AetherSH — Native Command Shell Specification (`AETHERSH.md`)

## 1. Shell Features (`userspace/bins/src/bin/aethersh.rs`)

`/bin/aethersh` is AetherOS's native Ring 3 command shell compiled as a freestanding `x86_64` ELF64 binary:
- **Multi-Stage Pipelines (`cmd1 | cmd2 | cmd3`)**: Allocates kernel anonymous pipes (`SYS_PIPE`), forks each stage, binds `stdin`/`stdout` via `SYS_DUP2`, and reaps all stages in order (`SYS_WAIT4`).
- **Command Chaining (`;` and `&&`)**: Executes sequential commands (`cmd1 ; cmd2`) and short-circuit conditional chains (`cmd1 && cmd2` stops on non-zero exit status).
- **Redirection (`>`, `>>`, `<`)**: Redirects file input and overwrite/append output.
- **Background Execution (`&`)**: Runs child processes asynchronously (`[bg] started PID <N>`).
- **Quoting & Environment Variables (`$VAR`, `$?`)**: Supports single quotes (`'...'`), double quotes (`"..."`), and `$KEY` environment expansion (`export KEY=VALUE`, `env`).
- **Built-in Commands**: `help`, `man`, `clear`, `echo`, `pwd`, `cd`, `env`, `export`, `umask`, `history`, `exit`.
