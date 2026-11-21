#![no_std]
#![no_main]

//! `/bin/aethersh` — Native AetherOS Command Shell (Milestone 11).
//!
//! Features:
//! - Command parsing with single/double quoting, backslash escaping, and `$VAR` expansion.
//! - Environment variable table (`env`, `export KEY=VALUE`) and command history (`history`).
//! - `PATH` executable resolution (`/bin`, `/sbin`, `/usr/bin`).
//! - Multi-stage pipelines (`cmd1 | cmd2`, e.g., `ps | grep init`) using real kernel pipes.
//! - Input/Output file redirection (`>` truncate, `>>` append, `<` input).
//! - Background execution (`cmd &`) and exit status tracking (`$?`).
//! - Built-in commands (`help`, `clear`, `echo`, `pwd`, `cd`, `env`, `export`, `history`, `exit`)
//!   and real ELF64 external process execution via `fork()` + `execve()`.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libaether::{
    aether_main, chdir, close, dup2, eprintln, execve, exit, fork, getcwd, getpid, getuid, open,
    pipe, print, println, read, stat, wait4, write_string_to_file, O_APPEND, O_CREAT, O_RDONLY,
    O_TRUNC, O_WRONLY,
};

struct ShellState {
    env: BTreeMap<String, String>,
    history: Vec<String>,
    last_status: i32,
}

impl ShellState {
    fn new() -> Self {
        let mut env = BTreeMap::new();
        env.insert(String::from("PATH"), String::from("/bin:/sbin:/usr/bin"));
        env.insert(String::from("HOME"), String::from("/root"));
        env.insert(String::from("USER"), String::from("root"));
        env.insert(String::from("SHELL"), String::from("/bin/aethersh"));
        env.insert(String::from("HOSTNAME"), String::from("aether"));
        Self {
            env,
            history: Vec::new(),
            last_status: 0,
        }
    }

    fn env_strings(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.env.len());
        for (k, v) in &self.env {
            out.push(format!("{}={}", k, v));
        }
        out
    }

    fn resolve_command_path(&self, cmd: &str) -> Option<String> {
        if cmd.contains('/') {
            if stat(cmd).is_ok() {
                return Some(String::from(cmd));
            }
            return None;
        }
        let path_var = self
            .env
            .get("PATH")
            .map(|s| s.as_str())
            .unwrap_or("/bin:/sbin:/usr/bin");
        for dir in path_var.split(':') {
            let candidate = format!("{}/{}", dir.trim_end_matches('/'), cmd);
            if stat(&candidate).is_ok() {
                return Some(candidate);
            }
        }
        None
    }
}

/// Tokenize a command segment respecting single quotes, double quotes, backslash escapes,
/// and `$VAR` / `$?` expansion.
fn tokenize_segment(input: &str, state: &ShellState) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            if let Some(next_c) = chars.next() {
                current.push(next_c);
            }
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if c == '$' && !in_single {
            if chars.peek() == Some(&'?') {
                let _ = chars.next();
                current.push_str(&format!("{}", state.last_status));
                continue;
            }
            let mut var_name = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_ascii_alphanumeric() || nc == '_' {
                    var_name.push(nc);
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            if !var_name.is_empty() {
                if let Some(val) = state.env.get(&var_name) {
                    current.push_str(val);
                }
            } else {
                current.push('$');
            }
            continue;
        }
        if c.is_whitespace() && !in_single && !in_double {
            if !current.is_empty() {
                tokens.push(core::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

struct ParsedCommand {
    argv: Vec<String>,
    stdin_file: Option<String>,
    stdout_file: Option<(String, bool)>, // (path, append)
}

fn parse_redirections(tokens: Vec<String>) -> ParsedCommand {
    let mut argv = Vec::new();
    let mut stdin_file = None;
    let mut stdout_file = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            ">" if i + 1 < tokens.len() => {
                stdout_file = Some((tokens[i + 1].clone(), false));
                i += 2;
                continue;
            }
            ">>" if i + 1 < tokens.len() => {
                stdout_file = Some((tokens[i + 1].clone(), true));
                i += 2;
                continue;
            }
            "<" if i + 1 < tokens.len() => {
                stdin_file = Some(tokens[i + 1].clone());
                i += 2;
                continue;
            }
            _ => {}
        }
        argv.push(tokens[i].clone());
        i += 1;
    }
    ParsedCommand {
        argv,
        stdin_file,
        stdout_file,
    }
}

fn apply_redirections(cmd: &ParsedCommand) -> Result<(), i64> {
    if let Some(ref in_path) = cmd.stdin_file {
        let fd = open(in_path, O_RDONLY, 0)?;
        let _ = dup2(fd, 0)?;
        if fd > 2 {
            let _ = close(fd);
        }
    }
    if let Some((ref out_path, append)) = cmd.stdout_file {
        let flags = if append {
            O_CREAT | O_WRONLY | O_APPEND
        } else {
            O_CREAT | O_WRONLY | O_TRUNC
        };
        let fd = open(out_path, flags, 0o644)?;
        let _ = dup2(fd, 1)?;
        if fd > 2 {
            let _ = close(fd);
        }
    }
    Ok(())
}

/// Try to run a shell built-in command in the parent process if no pipeline is active.
/// Returns `Some(exit_code)` if handled as a built-in, or `None` to execute as an external ELF64 binary.
fn try_run_builtin(cmd: &ParsedCommand, state: &mut ShellState) -> Option<i32> {
    if cmd.argv.is_empty() {
        return Some(0);
    }
    match cmd.argv[0].as_str() {
        "help" | "man" => {
            println!("AetherSH v0.2.0 (AetherOS Native Shell) — Command Ecosystem:");
            println!("  Built-ins : help man clear echo pwd cd env export umask history exit");
            println!("  Filesystem: ls cat head tail less more mkdir rmdir touch rm cp mv stat");
            println!("              chmod chown chgrp getfacl setfacl df du mount umount findmnt");
            println!("              basename dirname readlink realpath sync fsck mkfs");
            println!("  USB & HW  : lsusb lspci lsblk blkid eject lscpu lsmem vmstat dmesg");
            println!("  Networking: ip ifconfig ping route arp dig nslookup host dhclient");
            println!("              curl wget netstat ss hostname");
            println!("  System/Pkg: ps kill uname uptime free systemctl journalctl sudo useradd");
            println!("              whoami id groups aetherpkg grep wc sort uniq sha256sum md5sum");
            Some(0)
        }
        "umask" => {
            println!("0022");
            Some(0)
        }
        "clear" => {
            print!("\x1b[2J\x1b[H");
            Some(0)
        }
        "pwd" => {
            let cwd = getcwd().unwrap_or_else(|_| String::from("/"));
            if let Some((ref out_path, append)) = cmd.stdout_file {
                let _ = write_string_to_file(out_path, &format!("{}\n", cwd), append);
            } else {
                println!("{}", cwd);
            }
            Some(0)
        }
        "cd" => {
            let target = cmd.argv.get(1).map(|s| s.as_str()).unwrap_or("/root");
            match chdir(target) {
                Ok(()) => Some(0),
                Err(e) => {
                    eprintln!("cd: {}: errno {}", target, e);
                    Some(1)
                }
            }
        }
        "echo" => {
            let mut out = String::new();
            for (idx, arg) in cmd.argv.iter().skip(1).enumerate() {
                if idx > 0 {
                    out.push(' ');
                }
                out.push_str(arg);
            }
            out.push('\n');
            if let Some((ref out_path, append)) = cmd.stdout_file {
                match write_string_to_file(out_path, &out, append) {
                    Ok(()) => Some(0),
                    Err(e) => {
                        eprintln!("echo: {}: errno {}", out_path, e);
                        Some(1)
                    }
                }
            } else {
                print!("{}", out);
                Some(0)
            }
        }
        "env" => {
            for (k, v) in &state.env {
                println!("{}={}", k, v);
            }
            Some(0)
        }
        "export" => {
            for arg in cmd.argv.iter().skip(1) {
                if let Some((k, v)) = arg.split_once('=') {
                    state.env.insert(String::from(k), String::from(v));
                }
            }
            Some(0)
        }
        "history" => {
            for (i, h) in state.history.iter().enumerate() {
                println!("{:4}  {}", i + 1, h);
            }
            Some(0)
        }
        "exit" => {
            let code = cmd
                .argv
                .get(1)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(state.last_status);
            exit(code);
        }
        _ => None,
    }
}

fn execute_external_in_child(cmd: &ParsedCommand, state: &ShellState) -> ! {
    if let Err(e) = apply_redirections(cmd) {
        eprintln!("aethersh: redirection error: {}", e);
        exit(1);
    }

    if cmd.argv[0] == "echo" {
        for (idx, arg) in cmd.argv.iter().skip(1).enumerate() {
            if idx > 0 {
                print!(" ");
            }
            print!("{}", arg);
        }
        println!();
        exit(0);
    }

    let Some(bin_path) = state.resolve_command_path(&cmd.argv[0]) else {
        eprintln!("aethersh: command not found: {}", cmd.argv[0]);
        exit(127);
    };

    let argv_refs: Vec<&str> = cmd.argv.iter().map(|s| s.as_str()).collect();
    let env_owned = state.env_strings();
    let env_refs: Vec<&str> = env_owned.iter().map(|s| s.as_str()).collect();

    let _ = execve(&bin_path, &argv_refs, &env_refs);
    eprintln!("aethersh: failed to execute {}", bin_path);
    exit(126);
}

fn execute_line(raw_line: &str, state: &mut ShellState) -> i32 {
    let mut line = raw_line.trim();
    if line.is_empty() || line.starts_with('#') {
        return 0;
    }
    state.history.push(String::from(line));

    // Support command chaining via `;` and `&&`
    if line.contains(';') {
        let mut last = 0;
        for part in line.split(';') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                last = execute_line(trimmed, state);
            }
        }
        return last;
    }
    if line.contains("&&") {
        let mut last = 0;
        for part in line.split("&&") {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                last = execute_line(trimmed, state);
                if last != 0 {
                    return last;
                }
            }
        }
        return last;
    }

    let background = if let Some(stripped) = line.strip_suffix('&') {
        line = stripped.trim();
        true
    } else {
        false
    };

    let stages_raw: Vec<&str> = line.split('|').collect();
    if stages_raw.len() == 1 {
        let tokens = tokenize_segment(stages_raw[0], state);
        let cmd = parse_redirections(tokens);
        if cmd.argv.is_empty() {
            return 0;
        }
        if !background {
            if let Some(code) = try_run_builtin(&cmd, state) {
                state.last_status = code;
                return code;
            }
        }
        match fork() {
            Ok(0) => execute_external_in_child(&cmd, state),
            Ok(pid) => {
                if background {
                    println!("[bg] started PID {}", pid);
                    state.last_status = 0;
                    return 0;
                }
                let code = wait4(pid as i32).map(|(_, c)| c).unwrap_or(1);
                state.last_status = code;
                return code;
            }
            Err(e) => {
                eprintln!("aethersh: fork failed: {}", e);
                return 1;
            }
        }
    }

    // Multi-stage pipeline: `stage_0 | stage_1 | ... | stage_{n-1}`
    let mut parsed_stages = Vec::with_capacity(stages_raw.len());
    for s in stages_raw {
        let tokens = tokenize_segment(s, state);
        let cmd = parse_redirections(tokens);
        if cmd.argv.is_empty() {
            eprintln!("aethersh: syntax error: empty pipeline stage");
            return 2;
        }
        parsed_stages.push(cmd);
    }

    let mut prev_read_fd: Option<u32> = None;
    let mut child_pids: Vec<u32> = Vec::with_capacity(parsed_stages.len());

    for (idx, stage_cmd) in parsed_stages.iter().enumerate() {
        let is_last = idx + 1 == parsed_stages.len();
        let next_pipe = if !is_last {
            match pipe() {
                Ok(pair) => Some(pair),
                Err(e) => {
                    eprintln!("aethersh: pipe failed: {}", e);
                    return 1;
                }
            }
        } else {
            None
        };

        match fork() {
            Ok(0) => {
                if let Some(rfd) = prev_read_fd {
                    let _ = dup2(rfd, 0);
                    let _ = close(rfd);
                }
                if let Some((next_r, next_w)) = next_pipe {
                    let _ = close(next_r);
                    let _ = dup2(next_w, 1);
                    let _ = close(next_w);
                }
                execute_external_in_child(stage_cmd, state);
            }
            Ok(pid) => {
                child_pids.push(pid);
                if let Some(rfd) = prev_read_fd {
                    let _ = close(rfd);
                }
                if let Some((next_r, next_w)) = next_pipe {
                    let _ = close(next_w);
                    prev_read_fd = Some(next_r);
                } else {
                    prev_read_fd = None;
                }
            }
            Err(e) => {
                eprintln!("aethersh: fork failed in pipeline: {}", e);
                return 1;
            }
        }
    }

    let mut last_code = 0;
    if !background {
        for pid in child_pids {
            if let Ok((_, code)) = wait4(pid as i32) {
                last_code = code;
            }
        }
    }
    state.last_status = last_code;
    last_code
}

fn run_boot_verification(state: &mut ShellState) -> i32 {
    println!("[aethersh] Executing boot verification pipeline & redirection suite...");
    let commands = [
        "echo hello > /tmp/sh_test.txt",
        "echo world >> /tmp/sh_test.txt",
        "cat /tmp/sh_test.txt",
        "ps | grep init",
        "id",
        "ls -l /etc",
        "aetherpkg list",
    ];
    for cmd in commands {
        let rc = execute_line(cmd, state);
        if rc != 0 {
            eprintln!("[aethersh] boot-verify command failed (rc={}): {}", rc, cmd);
            return rc;
        }
    }
    println!("[aethersh] Boot verification suite succeeded.");
    0
}

fn main(args: &[&str]) -> i32 {
    let mut state = ShellState::new();

    if args.len() >= 2 && args[1] == "--boot-verify" {
        return run_boot_verification(&mut state);
    }

    if args.len() >= 3 && args[1] == "-c" {
        let mut last = 0;
        for part in args[2].split(';') {
            last = execute_line(part, &mut state);
        }
        return last;
    }

    println!(
        "AetherSH v0.1.0-alpha.1 (PID={}, UID={}) — Type 'help' for commands.",
        getpid(),
        getuid()
    );
    let mut line_buf = String::new();
    let mut byte = [0u8; 1];

    loop {
        let cwd = getcwd().unwrap_or_else(|_| String::from("/"));
        let prompt_char = if getuid() == 0 { '#' } else { '$' };
        print!("aether:{}{} ", cwd, prompt_char);

        line_buf.clear();
        loop {
            match read(0, &mut byte) {
                Ok(1) => {
                    let c = byte[0];
                    if c == b'\r' || c == b'\n' {
                        println!();
                        break;
                    } else if c == 0x7F || c == 0x08 {
                        if !line_buf.is_empty() {
                            line_buf.pop();
                            print!("\x08 \x08");
                        }
                    } else if (0x20..=0x7E).contains(&c) {
                        line_buf.push(c as char);
                        print!("{}", c as char);
                    }
                }
                _ => {
                    libaether::sleep_ms(10);
                }
            }
        }

        let cmd_str = line_buf.clone();
        execute_line(&cmd_str, &mut state);
    }
}

aether_main!(main);
