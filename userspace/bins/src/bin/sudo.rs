#![no_std]
#![no_main]

//! `/bin/sudo` — AetherOS Controlled Privilege Elevation Utility (Section 12.1).
//!
//! Requirements enforced:
//! - Uses kernel-controlled `sys_elevate_sudo` (`SYS_ELEVATE_SUDO`), which checks `/etc/sudoers`
//!   group membership (`%wheel`), rejects `no_new_privs`, and writes an immutable kernel audit record.
//! - Prints an explicit warning when passwordless QEMU development mode is active.
//! - Executes the requested target command via `execve`.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libaether::{aether_main, elevate_sudo, eprintln, execve, getuid, println, stat};

fn resolve_bin(cmd: &str) -> Option<String> {
    if cmd.contains('/') {
        return Some(String::from(cmd));
    }
    for dir in ["/bin", "/sbin", "/usr/bin"] {
        let candidate = format!("{}/{}", dir, cmd);
        if stat(&candidate).is_ok() {
            return Some(candidate);
        }
    }
    None
}

fn main(args: &[&str]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: sudo <command> [args...]");
        return 1;
    }

    let caller_uid = getuid();
    let target_cmd = args[1];

    println!(
        "[sudo] WARNING: QEMU development passwordless mode active (caller_uid={}, command='{}')",
        caller_uid, target_cmd
    );

    if let Err(e) = elevate_sudo(target_cmd) {
        eprintln!(
            "sudo: permission denied for UID {} (not in %wheel or blocked by policy, errno={})",
            caller_uid, e
        );
        return 1;
    }

    let Some(bin_path) = resolve_bin(target_cmd) else {
        eprintln!("sudo: {}: command not found", target_cmd);
        return 127;
    };

    let child_args: Vec<&str> = args[1..].to_vec();
    let envs = [
        "PATH=/bin:/sbin:/usr/bin",
        "HOME=/root",
        "USER=root",
        "SUDO_UID=1000",
    ];
    let _ = execve(&bin_path, &child_args, &envs);
    eprintln!("sudo: execve failed for {}", bin_path);
    126
}

aether_main!(main);
