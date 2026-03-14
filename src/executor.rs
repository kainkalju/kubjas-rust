//! Job execution via fork + exec with user/group switching, output redirection,
//! nice/ionice priority adjustment.

use crate::config::{Job, Output};
use libc::{gid_t, uid_t};
use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::unix::io::IntoRawFd;

/// Arguments passed from a trigger event for %host%/%job%/%notify% substitution.
#[derive(Debug, Clone, Default)]
pub struct ExecArgs {
    pub notify: String,   // e.g. "start-message", "success-message", or a file path
    pub from_job: String,
    pub host: String,
}

/// Fork and exec the job. Returns the child PID on success.
pub fn exec_job(job: &Job, args: &ExecArgs) -> Option<i32> {
    let cmdline = job.cmdline.as_deref()?;
    let cmdline = substitute_templates(cmdline, args);

    let user = job.user.clone();
    let group = if job.group.is_empty() { user.clone() } else { job.group.clone() };

    let uid = resolve_uid(&user)?;
    let gid = resolve_gid(&group)?;
    let output = job.output.clone();
    let ionice = job.ionice;
    let nice = job.nice;
    let job_name = job.name.clone();

    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            log::error!("fork failed for job [{}]", job_name);
            return None;
        }
        if pid > 0 {
            return Some(pid);
        }
        // --- child ---
        child_exec(&cmdline, uid, gid, &output, ionice, nice);
    }
}

fn substitute_templates(cmdline: &str, args: &ExecArgs) -> String {
    cmdline
        .replace("%host%", &args.host)
        .replace("%job%", &args.from_job)
        .replace("%notify%", &args.notify)
}

unsafe fn child_exec(
    cmdline: &str,
    uid: uid_t,
    gid: gid_t,
    output: &Output,
    ionice: bool,
    nice: bool,
) -> ! {
    // Apply ionice idle class via system() — this modifies the current process
    if ionice {
        let cmd = format!("/usr/bin/ionice -c 3 -p {}\0", libc::getpid());
        libc::system(cmd.as_ptr() as *const libc::c_char);
    }

    // Lower CPU priority via setpriority (equivalent to renice +10)
    if nice {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }

    // chdir to /
    libc::chdir(b"/\0".as_ptr() as *const libc::c_char);

    // Redirect stdin from /dev/null
    let devnull = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDONLY);
    if devnull >= 0 {
        libc::dup2(devnull, libc::STDIN_FILENO);
        libc::close(devnull);
    }

    // Redirect stdout/stderr
    match output {
        Output::None => {
            let fd = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_WRONLY);
            if fd >= 0 {
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::dup2(fd, libc::STDERR_FILENO);
                libc::close(fd);
            }
        }
        Output::File(path) => {
            if let Ok(file) = OpenOptions::new().append(true).create(true).open(path) {
                let fd = file.into_raw_fd();
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::dup2(fd, libc::STDERR_FILENO);
                libc::close(fd);
            }
        }
        Output::Passthrough => {
            // inherit from parent
        }
    }

    // Start new session
    libc::setsid();

    // Drop privileges
    libc::setgid(gid);
    libc::setuid(uid);

    // Build argv and exec
    let argv = build_argv(cmdline);
    if argv.is_empty() {
        libc::_exit(127);
    }

    let prog = &argv[0];
    let c_argv: Vec<*const libc::c_char> = argv
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    libc::execvp(prog.as_ptr(), c_argv.as_ptr());
    // exec failed
    libc::_exit(127);
}

fn build_argv(cmdline: &str) -> Vec<CString> {
    shell_split(cmdline)
        .into_iter()
        .filter_map(|s| CString::new(s).ok())
        .collect()
}

/// Simple shell-like word splitting (handles single/double quotes).
fn shell_split(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
            }
            '"' => {
                for c2 in chars.by_ref() {
                    if c2 == '"' { break; }
                    current.push(c2);
                }
            }
            '\'' => {
                for c2 in chars.by_ref() {
                    if c2 == '\'' { break; }
                    current.push(c2);
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn resolve_uid(user: &str) -> Option<uid_t> {
    if user == "root" {
        return Some(0);
    }
    let cname = CString::new(user).ok()?;
    unsafe {
        let pw = libc::getpwnam(cname.as_ptr());
        if pw.is_null() {
            log::error!("exec_job: cannot find user '{}'", user);
            return None;
        }
        Some((*pw).pw_uid)
    }
}

fn resolve_gid(group: &str) -> Option<gid_t> {
    if group == "root" {
        return Some(0);
    }
    let cname = CString::new(group).ok()?;
    unsafe {
        let gr = libc::getgrnam(cname.as_ptr());
        if !gr.is_null() {
            return Some((*gr).gr_gid);
        }
        // Fallback: resolve via passwd entry
        let pw = libc::getpwnam(cname.as_ptr());
        if !pw.is_null() {
            return Some((*pw).pw_gid);
        }
        log::error!("exec_job: cannot find group '{}'", group);
        None
    }
}
