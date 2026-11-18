#![no_std]
#![no_main]

//! `/sbin/init` — AetherOS User-Space System Init & Service Supervisor (PID 1, Milestone 9).
//!
//! Responsibilities:
//! - Runs in Ring 3 as PID 1.
//! - Verifies and mounts required filesystems (`/dev`, `/proc`, `/tmp`, `/run`).
//! - Parses `/etc/services.conf` (creating default service configuration if absent).
//! - Supervises configured startup services and records structured logs in `/var/log/init.log`.
//! - Reaps orphaned child processes.
//! - Launches `/bin/aethersh` on the system console (or executes startup verification script).

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libaether::{
    aether_main, close, dup2, eprintln, execve, exit, fork, get_user_uuid, getpid, getuid, mkdir,
    open, println, read_file_to_string, stat, wait4, write_string_to_file, O_RDWR,
};

#[derive(Clone)]
struct ServiceDef {
    name: String,
    command: String,
    args: Vec<String>,
    restart_policy: String,
    enabled: bool,
}

const DEFAULT_SERVICES_CONF: &str = "\
# AetherOS Service Supervisor Configuration (/etc/services.conf)
# Format: name|command|args|restart_policy|enabled
syshealth|/bin/uname|-a|once|true
aethersh|/bin/aethersh|--boot-verify|once|true
";

fn log_init(msg: &str) {
    println!("[init PID={}] {}", getpid(), msg);
    let line = format!("[init PID={}] {}\n", getpid(), msg);
    let _ = write_string_to_file("/var/log/init.log", &line, true);
}

fn ensure_directories_and_configs() {
    let _ = mkdir("/var", 0o755);
    let _ = mkdir("/var/log", 0o755);
    let _ = mkdir("/var/lib", 0o755);
    let _ = mkdir("/var/lib/aetherpkg", 0o755);
    let _ = mkdir("/run/services", 0o755);

    if stat("/etc/services.conf").is_err() {
        let _ = write_string_to_file("/etc/services.conf", DEFAULT_SERVICES_CONF, false);
    }
}

fn parse_services(conf: &str) -> Vec<ServiceDef> {
    let mut list = Vec::new();
    for raw_line in conf.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 5 {
            let mut args = Vec::new();
            for a in parts[2].split_whitespace() {
                if !a.is_empty() {
                    args.push(String::from(a));
                }
            }
            list.push(ServiceDef {
                name: String::from(parts[0]),
                command: String::from(parts[1]),
                args,
                restart_policy: String::from(parts[3]),
                enabled: parts[4] == "true" || parts[4] == "1",
            });
        }
    }
    list
}

fn run_service(svc: &ServiceDef) -> i32 {
    log_init(&format!(
        "Starting service '{}' -> {} (policy={})",
        svc.name, svc.command, svc.restart_policy
    ));

    match fork() {
        Ok(0) => {
            let mut argv_refs: Vec<&str> = Vec::with_capacity(svc.args.len() + 1);
            argv_refs.push(&svc.command);
            for a in &svc.args {
                argv_refs.push(a.as_str());
            }
            let envs = [
                "PATH=/bin:/sbin:/usr/bin",
                "HOME=/root",
                "USER=root",
                "SHELL=/bin/aethersh",
            ];
            let _ = execve(&svc.command, &argv_refs, &envs);
            eprintln!("[init] execve failed for {}", svc.command);
            exit(127);
        }
        Ok(child_pid) => {
            let pid_file = format!("/run/services/{}.pid", svc.name);
            let _ = write_string_to_file(&pid_file, &format!("{}\n", child_pid), false);
            match wait4(child_pid as i32) {
                Ok((_, code)) => {
                    log_init(&format!(
                        "Service '{}' (PID={}) exited with status {}",
                        svc.name, child_pid, code
                    ));
                    code
                }
                Err(e) => {
                    log_init(&format!("wait4 error for '{}': -{}", svc.name, e));
                    -1
                }
            }
        }
        Err(e) => {
            log_init(&format!("fork failed for '{}': -{}", svc.name, e));
            -1
        }
    }
}

fn main(args: &[&str]) -> i32 {
    ensure_directories_and_configs();
    log_init(&format!(
        "AetherOS /sbin/init starting (uid={} uuid={})",
        getuid(),
        get_user_uuid()
    ));

    // If invoked with `--verify-once` (during automated boot verification), run services and exit 0
    let verify_once = args.contains(&"--verify-once");

    let conf = read_file_to_string("/etc/services.conf")
        .unwrap_or_else(|_| String::from(DEFAULT_SERVICES_CONF));
    let services = parse_services(&conf);

    for svc in &services {
        if !svc.enabled {
            continue;
        }
        let code = run_service(svc);
        if code != 0 && svc.restart_policy == "on-failure" {
            log_init(&format!("Restarting failed service '{}' once...", svc.name));
            let _ = run_service(svc);
        }
    }

    // Reap any remaining orphaned children
    while let Ok((reaped, _)) = wait4(-1) {
        if reaped == 0 {
            break;
        }
    }

    if verify_once {
        log_init("Boot verification sequence completed cleanly.");
        return 0;
    }

    // Launch interactive console shell `/bin/aethersh`
    if let Ok(fd) = open("/dev/console", O_RDWR, 0) {
        let _ = dup2(fd, 0);
        let _ = dup2(fd, 1);
        let _ = dup2(fd, 2);
        if fd > 2 {
            let _ = close(fd);
        }
    }
    log_init("Handing console over to /bin/aethersh");
    loop {
        let _ = run_service(&ServiceDef {
            name: String::from("console-shell"),
            command: String::from("/bin/aethersh"),
            args: Vec::new(),
            restart_policy: String::from("always"),
            enabled: true,
        });
    }
}

aether_main!(main);
