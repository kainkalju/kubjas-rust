mod config;
mod executor;
mod inotify_watch;
mod notify;
mod period;
mod scheduler;

use chrono::Local;
use clap::Parser;
use config::Job;
use inotify_watch::FileWatcher;
use notify::{recv_notify, send_notify, DuplicateFilter, NotifyType};
use scheduler::{start_jobs, Trigger};
use std::collections::HashMap;
use std::fs;
use std::io::Write as IoWrite;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

// ── Signal flags ──────────────────────────────────────────────────────────────

static SIGCHLD_RECEIVED: AtomicBool = AtomicBool::new(false);
static SIGHUP_RECEIVED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_RECEIVED: AtomicBool = AtomicBool::new(false);
static SIGUSR1_RECEIVED: AtomicBool = AtomicBool::new(false);
static SIGUSR2_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigchld(_: libc::c_int) {
    SIGCHLD_RECEIVED.store(true, Ordering::Relaxed);
}
extern "C" fn handle_sighup(_: libc::c_int) {
    SIGHUP_RECEIVED.store(true, Ordering::Relaxed);
}
extern "C" fn handle_shutdown(_: libc::c_int) {
    SHUTDOWN_RECEIVED.store(true, Ordering::Relaxed);
}
extern "C" fn handle_sigusr1(_: libc::c_int) {
    SIGUSR1_RECEIVED.store(true, Ordering::Relaxed);
}
extern "C" fn handle_sigusr2(_: libc::c_int) {
    SIGUSR2_RECEIVED.store(true, Ordering::Relaxed);
}

// Avoid "function pointer cast" warning by wrapping casts
fn sig_fn(f: extern "C" fn(libc::c_int)) -> libc::sighandler_t {
    f as libc::sighandler_t
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "kubjas", about = "Cron-like daemon to execute scheduled commands")]
struct Cli {
    /// Main configuration file
    #[arg(long, default_value = "/etc/kubjas.conf")]
    conf_file: String,

    /// Log file path (appended)
    #[arg(long)]
    log_file: Option<String>,

    /// PID file path
    #[arg(long)]
    pid_file: Option<String>,

    /// Run in background (daemonize)
    #[arg(long)]
    background: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_str() -> String {
    Local::now().format("%a %b %e %T %Y").to_string()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn whoami() -> String {
    if let Ok(h) = fs::read_to_string("/proc/sys/kernel/hostname") {
        return h.trim().to_string();
    }
    if let Ok(output) = std::process::Command::new("/bin/hostname").output() {
        return String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    "localhost".to_string()
}

fn elapsed_str(start_unix: u64, start_us: u64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let now_secs = now.as_secs();
    let now_us = now.subsec_micros() as u64;

    let secs = now_secs.saturating_sub(start_unix);
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let rem = rem % 3600;
    let mins = rem / 60;
    let sec = rem % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, mins, sec)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, mins, sec)
    } else if mins > 0 {
        format!("{}m {}s", mins, sec)
    } else if sec > 0 {
        format!("{}s", sec)
    } else {
        let us = (now_us as i64 - start_us as i64).abs() as f64 / 1_000_000.0;
        format!("{:.3}s", us)
    }
}

// ── Daemonize ─────────────────────────────────────────────────────────────────

unsafe fn daemonize() {
    let fd = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDONLY);
    if fd >= 0 {
        libc::dup2(fd, libc::STDIN_FILENO);
        libc::close(fd);
    }
    let null_out = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_WRONLY);
    if null_out >= 0 {
        libc::dup2(null_out, libc::STDOUT_FILENO);
        libc::dup2(null_out, libc::STDERR_FILENO);
        libc::close(null_out);
    }
    let pid = libc::fork();
    if pid < 0 {
        eprintln!("kubjas: fork failed");
        libc::_exit(1);
    }
    if pid > 0 {
        libc::_exit(0);
    }
    libc::setsid();
    libc::chdir(b"/\0".as_ptr() as *const libc::c_char);
}

fn redirect_output_to_file(log_path: &str) {
    unsafe {
        use std::ffi::CString;
        let path = CString::new(log_path).unwrap();
        let fd = libc::open(
            path.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
            0o644,
        );
        if fd >= 0 {
            libc::dup2(fd, libc::STDOUT_FILENO);
            libc::dup2(fd, libc::STDERR_FILENO);
            libc::close(fd);
        }
    }
}

// ── Signal setup ─────────────────────────────────────────────────────────────

fn setup_signals() {
    unsafe {
        libc::signal(libc::SIGCHLD, sig_fn(handle_sigchld));
        libc::signal(libc::SIGHUP,  sig_fn(handle_sighup));
        libc::signal(libc::SIGTERM, sig_fn(handle_shutdown));
        libc::signal(libc::SIGINT,  sig_fn(handle_shutdown));
        libc::signal(libc::SIGUSR1, sig_fn(handle_sigusr1));
        libc::signal(libc::SIGUSR2, sig_fn(handle_sigusr2));
    }
}

// ── SIGCHLD reaping ───────────────────────────────────────────────────────────

struct ChildExit {
    pid: i32,
    exit_code: i32,
    signal_num: i32,
}

fn reap_children() -> Vec<ChildExit> {
    let mut exited = Vec::new();
    loop {
        let mut status: libc::c_int = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break;
        }
        let exit_code = libc::WEXITSTATUS(status);
        let signal_num = if libc::WIFSIGNALED(status) { libc::WTERMSIG(status) } else { 0 };
        exited.push(ChildExit { pid, exit_code, signal_num });
    }
    exited
}

// ── Config helpers ────────────────────────────────────────────────────────────

fn log_jobs(jobs: &[Job]) {
    for job in jobs {
        println!("{}  Job [{}] interval={}", now_str(), job.name, job.interval);
    }
}

fn save_exec_times(jobs: &[Job]) -> HashMap<String, (u64, u64)> {
    jobs.iter()
        .map(|j| (j.name.clone(), (j.exec_time_unix, j.exec_ms)))
        .collect()
}

fn setup_inotify_watches(
    jobs: &[Job],
    watcher: &mut FileWatcher,
    known_watches: &mut HashMap<String, bool>,
) {
    for job in jobs {
        for path in &job.watch {
            if !known_watches.contains_key(path) {
                match watcher.add_watch(path) {
                    Ok(_) => {
                        known_watches.insert(path.to_string(), true);
                    }
                    Err(e) => {
                        log::error!("watch creation failed for {}: {}", path, e);
                    }
                }
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Redirect output to log file before doing anything else
    if let Some(ref log_path) = cli.log_file {
        redirect_output_to_file(log_path);
    }

    // Init logger — writes to stdout (which may be the log file above)
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| writeln!(buf, "{}  {}", Local::now().format("%a %b %e %T %Y"), record.args()))
        .init();

    // Daemonize
    if cli.background {
        unsafe { daemonize() };
        // Re-open log file in the child process
        if let Some(ref log_path) = cli.log_file {
            redirect_output_to_file(log_path);
        }
    }

    // PID file
    if let Some(ref pid_path) = cli.pid_file {
        match fs::File::create(pid_path) {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", unsafe { libc::getpid() });
            }
            Err(e) => eprintln!("Cannot write PID file {}: {}", pid_path, e),
        }
    }

    let my_host = whoami();
    let start_unix = unix_now();
    let config_dir = "/etc/kubjas.d";

    println!(
        "{}  Starting [kubjas] PID {} at host \"{}\"",
        now_str(),
        unsafe { libc::getpid() },
        my_host
    );

    // Initial config load
    println!("{}  Reading configuration files", now_str());
    let mut jobs: Vec<Job> = config::load_config(&cli.conf_file, config_dir, &HashMap::new());
    log_jobs(&jobs);

    // Signal handlers
    setup_signals();

    // inotify watcher
    let mut watcher = match FileWatcher::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Unable to create inotify watcher: {}", e);
            std::process::exit(1);
        }
    };
    let mut known_watches: HashMap<String, bool> = HashMap::new();
    setup_inotify_watches(&jobs, &mut watcher, &mut known_watches);

    // UDP socket
    let sock = match UdpSocket::bind(("0.0.0.0", notify::NOTIFY_PORT)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Couldn't be a udp server on port {}: {}",
                notify::NOTIFY_PORT, e
            );
            std::process::exit(1);
        }
    };
    sock.set_nonblocking(true).expect("set_nonblocking failed");

    // Runtime state
    let mut running: HashMap<String, i32> = HashMap::new(); // job name → pid
    let mut pid_to_job: HashMap<i32, String> = HashMap::new(); // pid → job name
    let mut dup_filter = DuplicateFilter::new();
    let mut no_new_jobs = false;
    let mut last_tick = unix_now().saturating_sub(1); // trigger immediately on first loop

    // Event loop
    loop {
        // SIGCHLD: reap children
        if SIGCHLD_RECEIVED.swap(false, Ordering::Relaxed) {
            for child in reap_children() {
                if let Some(job_name) = pid_to_job.remove(&child.pid) {
                    let elapsed = if let Some(job) = jobs.iter().find(|j| j.name == job_name) {
                        elapsed_str(job.exec_time_unix, job.exec_ms)
                    } else {
                        "?".to_string()
                    };

                    let (notify_failure, notify_success) = jobs
                        .iter()
                        .find(|j| j.name == job_name)
                        .map(|j| (j.notify_failure.clone(), j.notify_success.clone()))
                        .unwrap_or_default();

                    if child.exit_code != 0 || child.signal_num != 0 {
                        println!(
                            "{}  PID {} exited [{}] running time {}.",
                            now_str(), child.pid, job_name, elapsed
                        );
                        println!(
                            "{}  FAILURE: PID {} exited with status (exit={}, signal={})",
                            now_str(), child.pid, child.exit_code, child.signal_num
                        );
                        for target in &notify_failure {
                            send_notify(target, &job_name, &NotifyType::FailureMessage, &my_host);
                        }
                    } else {
                        println!(
                            "{}  PID {} exited [{}] running time {}.",
                            now_str(), child.pid, job_name, elapsed
                        );
                        for target in &notify_success {
                            send_notify(target, &job_name, &NotifyType::SuccessMessage, &my_host);
                        }
                    }
                    running.remove(&job_name);
                } else if child.exit_code != 0 {
                    println!(
                        "{}  WARN: PID {} exited with status (exit={}, signal={})",
                        now_str(), child.pid, child.exit_code, child.signal_num
                    );
                }
            }
        }

        // SIGHUP: reload config
        if SIGHUP_RECEIVED.swap(false, Ordering::Relaxed) {
            if let Some(ref log_path) = cli.log_file {
                redirect_output_to_file(log_path);
            }
            println!("{}  Reading configuration files", now_str());
            let prev = save_exec_times(&jobs);
            jobs = config::load_config(&cli.conf_file, config_dir, &prev);
            log_jobs(&jobs);
            setup_inotify_watches(&jobs, &mut watcher, &mut known_watches);
        }

        // SIGTERM/SIGINT: shutdown
        if SHUTDOWN_RECEIVED.load(Ordering::Relaxed) {
            println!("{}  Shutdown", now_str());
            if let Some(ref pid_path) = cli.pid_file {
                let _ = fs::remove_file(pid_path);
            }
            std::process::exit(0);
        }

        // SIGUSR1: print running jobs
        if SIGUSR1_RECEIVED.swap(false, Ordering::Relaxed) {
            let names: Vec<&str> = running.keys().map(|s| s.as_str()).collect();
            println!("{}  running ({})", now_str(), names.join(" "));
        }

        // SIGUSR2: toggle scheduling
        if SIGUSR2_RECEIVED.swap(false, Ordering::Relaxed) {
            no_new_jobs = !no_new_jobs;
            if no_new_jobs {
                println!("{}  Switching job scheduling OFF", now_str());
            } else {
                println!("{}  Switching job scheduling ON", now_str());
            }
        }

        // inotify events
        for path in watcher.read_events() {
            let trigger = Trigger::Watch(path);
            let new_pids = start_jobs(
                &trigger,
                &mut jobs,
                &mut running,
                no_new_jobs,
                start_unix,
                &my_host,
            );
            for (name, pid) in new_pids {
                pid_to_job.insert(pid, name);
            }
        }

        // UDP notify events
        let mut buf = [0u8; 4096];
        loop {
            match sock.recv_from(&mut buf) {
                Ok((n, remote)) => {
                    if let Some(msg) = recv_notify(&buf[..n], remote, &mut dup_filter) {
                        let _ = sock.send_to(b"OK\n", remote);
                        let trigger = Trigger::Notify(&msg);
                        let new_pids = start_jobs(
                            &trigger,
                            &mut jobs,
                            &mut running,
                            no_new_jobs,
                            start_unix,
                            &my_host,
                        );
                        for (name, pid) in new_pids {
                            pid_to_job.insert(pid, name);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::warn!("UDP recv error: {}", e);
                    break;
                }
            }
        }

        // Timer tick — run once per second
        let now = unix_now();
        if now > last_tick {
            last_tick = now;
            let trigger = Trigger::Time;
            let new_pids = start_jobs(
                &trigger,
                &mut jobs,
                &mut running,
                no_new_jobs,
                start_unix,
                &my_host,
            );
            for (name, pid) in new_pids {
                pid_to_job.insert(pid, name);
            }
        }

        std::thread::sleep(Duration::from_millis(200));
    }
}
