//! Job scheduling logic — decides which jobs to start given a trigger.

use crate::config::{Interval, Job};
use crate::executor::{exec_job, ExecArgs};
use crate::notify::{send_notify, NotifyMessage, NotifyType};
use rand::Rng;
use std::collections::HashMap;
use std::time::SystemTime;

/// A trigger that causes start_jobs() to evaluate which jobs should run.
#[derive(Debug)]
pub enum Trigger<'a> {
    Time,
    Watch(String),          // full path of file that changed
    Notify(&'a NotifyMessage),
}

/// Try to start eligible jobs given a trigger.
/// Returns a list of (job_name, pid) for each newly started job.
pub fn start_jobs(
    trigger: &Trigger,
    jobs: &mut Vec<Job>,
    running: &mut HashMap<String, i32>,
    no_new_jobs: bool,
    start_time_unix: u64,
    my_host: &str,
) -> Vec<(String, i32)> {
    if no_new_jobs {
        return Vec::new();
    }

    let now_unix = unix_now();

    // For time triggers: enforce once-per-second minimum
    if let Trigger::Time = trigger {
        // Caller is responsible for not calling more than once per second
    }

    let mut started = Vec::new();
    let mut sort_needed = false;

    // We iterate by index to allow mutation of job.exec_time_unix
    let job_count = jobs.len();
    for i in 0..job_count {
        let job_name = jobs[i].name.clone();

        // --- trigger-type filter ---
        match trigger {
            Trigger::Notify(msg) => {
                if msg.to_job != job_name {
                    continue;
                }
            }
            Trigger::Watch(_) | Trigger::Time => {}
        }

        // --- period check ---
        if let Some(ref period_str) = jobs[i].period.clone() {
            if !crate::period::in_period(period_str) {
                continue;
            }
        }

        // --- conflicts check ---
        if in_conflicts(&jobs[i], running) {
            continue;
        }

        // --- depends check ---
        if no_dependency(&jobs[i], running) {
            continue;
        }

        // --- interval check ---
        let effective_interval = match &jobs[i].interval {
            Interval::Disabled => continue,
            Interval::Range(lo, hi) => {
                let diff = hi.saturating_sub(*lo);
                let r: u64 = rand::thread_rng().gen_range(0..=diff);
                Interval::Seconds(lo + r)
            }
            other => other.clone(),
        };

        // Filter by trigger vs interval type
        match trigger {
            Trigger::Watch(path) => {
                if !matches!(effective_interval, Interval::OnChange) {
                    continue;
                }
                if !in_watch(&jobs[i], path) {
                    log::debug!(
                        "watch path mismatch for [{}]: event={} watches={:?}",
                        job_name, path, jobs[i].watch
                    );
                    continue;
                }
            }
            Trigger::Notify(msg) => {
                let expected = match msg.notify_type {
                    NotifyType::StartMessage => Interval::StartMessage,
                    NotifyType::SuccessMessage => Interval::SuccessMessage,
                    NotifyType::FailureMessage => Interval::FailureMessage,
                    NotifyType::Ping => continue,
                };
                if !interval_matches(&effective_interval, &expected) {
                    continue;
                }
            }
            Trigger::Time => {
                if matches!(effective_interval, Interval::OnChange | Interval::StartMessage | Interval::SuccessMessage | Interval::FailureMessage) {
                    continue;
                }
            }
        }

        // --- time-based interval elapsed check (for numeric intervals only) ---
        if let Trigger::Time = trigger {
            if let Interval::Seconds(secs) = &effective_interval {
                let last_exec = jobs[i].exec_time_unix;
                if last_exec > 0 && now_unix.saturating_sub(last_exec) < *secs {
                    continue;
                }
                // Also: don't run before interval has elapsed since daemon start
                if now_unix.saturating_sub(start_time_unix) < *secs {
                    continue;
                }
            }
        }

        // --- build exec args ---
        let exec_args = match trigger {
            Trigger::Watch(path) => ExecArgs {
                notify: path.clone(),
                from_job: "kubjas".to_string(),
                host: my_host.to_string(),
            },
            Trigger::Notify(msg) => ExecArgs {
                notify: msg.notify_type.as_str().to_string(),
                from_job: msg.from_job.clone(),
                host: msg.from_host.clone(),
            },
            Trigger::Time => ExecArgs::default(),
        };

        // --- signal running job if requested ---
        if let Some(ref sig_name) = jobs[i].signal.clone() {
            if !matches!(trigger, Trigger::Time) {
                if let Some(&pid) = running.get(&job_name) {
                    send_signal(pid, sig_name);
                }
            }
        }

        // --- skip if already running (unless signal-only) ---
        if running.contains_key(&job_name) {
            continue;
        }

        // --- exec ---
        match exec_job(&jobs[i], &exec_args) {
            Some(pid) => {
                log::info!("EXEC [{}] PID {}", job_name, pid);
                running.insert(job_name.clone(), pid);

                // Send notify-start
                let notify_starts: Vec<String> = jobs[i].notify_start.clone();
                for target in &notify_starts {
                    send_notify(target, &job_name, &NotifyType::StartMessage, my_host);
                }

                // Update exec time
                jobs[i].exec_time_unix = now_unix;
                jobs[i].exec_ms = unix_now_ms() % 1_000_000;

                started.push((job_name, pid));
                sort_needed = true;
            }
            None => {
                log::error!("FAILED EXEC {}", job_name);
            }
        }
    }

    // Re-sort jobs by exec_time so least-recently-run jobs go first next cycle
    if sort_needed {
        jobs.sort_by_key(|j| j.exec_time_unix);
    }

    started
}

fn in_conflicts(job: &Job, running: &HashMap<String, i32>) -> bool {
    job.conflicts.iter().any(|name| running.contains_key(name))
}

fn no_dependency(job: &Job, running: &HashMap<String, i32>) -> bool {
    job.depends.iter().any(|name| !running.contains_key(name))
}

fn in_watch(job: &Job, fullpath: &str) -> bool {
    job.watch.iter().any(|w| fullpath.starts_with(w.as_str()))
}

fn interval_matches(a: &Interval, b: &Interval) -> bool {
    matches!(
        (a, b),
        (Interval::OnChange, Interval::OnChange)
            | (Interval::StartMessage, Interval::StartMessage)
            | (Interval::SuccessMessage, Interval::SuccessMessage)
            | (Interval::FailureMessage, Interval::FailureMessage)
            | (Interval::Seconds(_), Interval::Seconds(_))
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Send a Unix signal to a process by name or number.
fn send_signal(pid: i32, sig_name: &str) {
    let signum: i32 = if let Ok(n) = sig_name.parse::<i32>() {
        n
    } else {
        signal_name_to_num(sig_name)
    };
    if signum > 0 {
        unsafe {
            libc::kill(pid, signum);
        }
    }
}

fn signal_name_to_num(name: &str) -> i32 {
    match name.to_uppercase().trim_start_matches("SIG") {
        "HUP" => libc::SIGHUP,
        "INT" => libc::SIGINT,
        "QUIT" => libc::SIGQUIT,
        "KILL" => libc::SIGKILL,
        "TERM" => libc::SIGTERM,
        "USR1" => libc::SIGUSR1,
        "USR2" => libc::SIGUSR2,
        "PIPE" => libc::SIGPIPE,
        "ALRM" | "ALARM" => libc::SIGALRM,
        "CONT" => libc::SIGCONT,
        "STOP" => libc::SIGSTOP,
        "TSTP" => libc::SIGTSTP,
        _ => {
            log::warn!("unknown signal name: {}", name);
            0
        }
    }
}
