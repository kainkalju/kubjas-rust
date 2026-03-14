use configparser::ini::Ini;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone)]
pub enum Interval {
    Disabled,
    Seconds(u64),
    Range(u64, u64),
    OnChange,
    StartMessage,
    SuccessMessage,
    FailureMessage,
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Interval::Disabled => write!(f, "0"),
            Interval::Seconds(n) => write!(f, "{}", n),
            Interval::Range(lo, hi) => write!(f, "{}-{}", lo, hi),
            Interval::OnChange => write!(f, "onchange"),
            Interval::StartMessage => write!(f, "start-message"),
            Interval::SuccessMessage => write!(f, "success-message"),
            Interval::FailureMessage => write!(f, "failure-message"),
        }
    }
}

impl Interval {
    pub fn parse(s: &str) -> Interval {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "0" | "" => Interval::Disabled,
            "onchange" => Interval::OnChange,
            "start-message" => Interval::StartMessage,
            "success-message" => Interval::SuccessMessage,
            "failure-message" => Interval::FailureMessage,
            _ => {
                if let Some(pos) = s.find('-') {
                    let (a, b) = s.split_at(pos);
                    let b = &b[1..];
                    if let (Ok(lo), Ok(hi)) = (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
                        return Interval::Range(lo, hi);
                    }
                }
                if let Ok(n) = s.parse::<u64>() {
                    Interval::Seconds(n)
                } else {
                    Interval::Disabled
                }
            }
        }
    }


}

#[derive(Debug, Clone)]
pub enum Output {
    Passthrough,
    None,
    File(String),
}

impl Output {
    fn parse(s: &str) -> Output {
        match s.trim().to_lowercase().as_str() {
            "passthrough" => Output::Passthrough,
            "none" => Output::None,
            _ => Output::File(s.trim().to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    pub cmdline: Option<String>,
    pub user: String,
    pub group: String,
    pub interval: Interval,
    pub period: Option<String>,
    pub conflicts: Vec<String>,
    pub depends: Vec<String>,
    pub watch: Vec<String>,
    pub notify_start: Vec<String>,
    pub notify_success: Vec<String>,
    pub notify_failure: Vec<String>,
    pub signal: Option<String>,
    pub output: Output,
    pub ionice: bool,
    pub nice: bool,
    // Runtime tracking (not from config)
    pub exec_time_unix: u64,  // unix seconds for interval comparison
    pub exec_ms: u64,
}

impl Job {
    fn default(name: &str) -> Self {
        Job {
            name: name.to_string(),
            cmdline: None,
            user: "root".to_string(),
            group: "root".to_string(),
            interval: Interval::Disabled,
            period: None,
            conflicts: Vec::new(),
            depends: Vec::new(),
            watch: Vec::new(),
            notify_start: Vec::new(),
            notify_success: Vec::new(),
            notify_failure: Vec::new(),
            signal: None,
            output: Output::Passthrough,
            ionice: false,
            nice: false,
            exec_time_unix: 0,
            exec_ms: 0,
        }
    }
}

/// Splits a multi-value INI field (newline or repeat keys) into a Vec<String>.
fn split_multivalue(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Returns true if a command exists on PATH or is an absolute executable.
pub fn is_executable(cmdline: &str) -> bool {
    if cmdline.is_empty() {
        return false;
    }
    let cmd = cmdline.split_whitespace().next().unwrap_or("");
    if std::path::Path::new(cmd).is_absolute() {
        return std::path::Path::new(cmd).is_file()
            && std::fs::metadata(cmd)
                .map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o111 != 0
                })
                .unwrap_or(false);
    }
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            let full = format!("{}/{}", dir, cmd);
            if std::path::Path::new(&full).is_file() {
                if let Ok(m) = std::fs::metadata(&full) {
                    use std::os::unix::fs::PermissionsExt;
                    if m.permissions().mode() & 0o111 != 0 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn apply_ini_section(
    job: &mut Job,
    cfg: &Ini,
    section: &str,
    all_sections: &str,
) {
    macro_rules! get {
        ($key:expr) => {
            cfg.get(section, $key)
        };
    }

    if let Some(v) = get!("cmdline") {
        if !v.is_empty() {
            job.cmdline = Some(v);
        }
    }
    if let Some(v) = get!("user") {
        if !v.is_empty() {
            job.user = v;
        }
    }
    if let Some(v) = get!("group") {
        if !v.is_empty() {
            job.group = v;
        }
    }
    if let Some(v) = get!("interval") {
        if !v.is_empty() {
            job.interval = Interval::parse(&v);
        }
    }
    if let Some(v) = get!("period") {
        if !v.is_empty() {
            job.period = Some(v);
        }
    }
    if let Some(v) = get!("conflicts") {
        let targets: Vec<String> = if v.trim() == "*" {
            all_sections
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s != &job.name)
                .collect()
        } else {
            split_multivalue(&v)
        };
        if !targets.is_empty() {
            job.conflicts = targets;
        }
    }
    if let Some(v) = get!("depends") {
        let targets: Vec<String> = if v.trim() == "*" {
            all_sections
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s != &job.name)
                .collect()
        } else {
            split_multivalue(&v)
        };
        if !targets.is_empty() {
            job.depends = targets;
        }
    }
    if let Some(v) = get!("watch") {
        let vals = split_multivalue(&v);
        if !vals.is_empty() {
            job.watch = vals;
        }
    }
    if let Some(v) = get!("notify-start") {
        let vals = split_multivalue(&v);
        if !vals.is_empty() {
            job.notify_start = vals;
        }
    }
    if let Some(v) = get!("notify-success") {
        let vals = split_multivalue(&v);
        if !vals.is_empty() {
            job.notify_success = vals;
        }
    }
    if let Some(v) = get!("notify-failure") {
        let vals = split_multivalue(&v);
        if !vals.is_empty() {
            job.notify_failure = vals;
        }
    }
    if let Some(v) = get!("signal") {
        if !v.is_empty() {
            job.signal = Some(v);
        }
    }
    if let Some(v) = get!("output") {
        if !v.is_empty() {
            job.output = Output::parse(&v);
        }
    }
    if let Some(v) = get!("ionice") {
        job.ionice = v.trim() == "1";
    }
    if let Some(v) = get!("nice") {
        job.nice = v.trim() == "1";
    }
}

/// Load all config files, returning a list of valid jobs.
/// `prev_exec` carries over execution timestamps from the previous load.
pub fn load_config(
    default_cfg: &str,
    config_dir: &str,
    prev_exec: &HashMap<String, (u64, u64)>,
) -> Vec<Job> {
    let mut cfg_files = vec![default_cfg.to_string()];

    if let Ok(entries) = fs::read_dir(config_dir) {
        let mut extras: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                let path = e.path();
                let fname = path.file_name()?.to_string_lossy().to_string();
                if fname.ends_with("dpkg-old") || fname.ends_with('~') || fname.starts_with('.') {
                    return None;
                }
                if fs::metadata(&path).is_err() {
                    return None;
                }
                Some(path.to_string_lossy().to_string())
            })
            .collect();
        extras.sort();
        cfg_files.extend(extras);
    }

    let mut jobs: Vec<Job> = Vec::new();
    let mut seen: HashMap<String, bool> = HashMap::new();

    for cfg_file in &cfg_files {
        let mut cfg = Ini::new_cs(); // case-sensitive keys
        cfg.set_comment_symbols(&[';', '#']);
        if cfg.load(cfg_file).is_err() {
            log::warn!("Cannot read config file: {}", cfg_file);
            continue;
        }

        // Collect default params from [*] section
        let mut defaults: HashMap<String, String> = HashMap::new();
        if let Some(map) = cfg.get_map().and_then(|m| m.get("*").cloned()) {
            for (k, v) in map {
                if let Some(val) = v {
                    if !val.is_empty() {
                        defaults.insert(k, val);
                    }
                }
            }
        }

        // Collect all non-default section names for wildcard expansion
        let all_sections: String = cfg
            .sections()
            .into_iter()
            .filter(|s| s != "*")
            .collect::<Vec<_>>()
            .join("\n");

        for section in cfg.sections() {
            if section == "*" {
                continue;
            }
            if seen.contains_key(&section) {
                log::warn!("Duplicate job [{}] in {}", section, cfg_file);
                continue;
            }
            seen.insert(section.clone(), true);

            let mut job = Job::default(&section);

            // Apply defaults first
            for (k, v) in &defaults {
                apply_field_from_str(&mut job, k, v, &all_sections);
            }

            // Apply job-specific settings (override defaults)
            apply_ini_section(&mut job, &cfg, &section, &all_sections);

            // Restore exec time from previous config load
            if let Some(&(et, em)) = prev_exec.get(&section) {
                job.exec_time_unix = et;
                job.exec_ms = em;
            }

            // Validate period
            if let Some(ref period_str) = job.period.clone() {
                if !crate::period::is_valid_period(period_str) {
                    log::warn!(
                        "incorrect period [{}] {}",
                        job.name,
                        period_str
                    );
                    continue;
                }
            }

            // Validate cmdline
            match &job.cmdline {
                None => {
                    log::warn!("no cmdline for job [{}]", job.name);
                    continue;
                }
                Some(cmd) => {
                    if !is_executable(cmd) {
                        log::warn!("cannot execute [{}] {}", job.name, cmd);
                        continue;
                    }
                }
            }

            jobs.push(job);
        }
    }

    jobs
}

/// Apply a single key=value from defaults to a Job.
fn apply_field_from_str(job: &mut Job, key: &str, val: &str, all_sections: &str) {
    match key {
        "cmdline" => job.cmdline = Some(val.to_string()),
        "user" => job.user = val.to_string(),
        "group" => job.group = val.to_string(),
        "interval" => job.interval = Interval::parse(val),
        "period" => job.period = Some(val.to_string()),
        "conflicts" => {
            job.conflicts = if val.trim() == "*" {
                all_sections
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != &job.name)
                    .collect()
            } else {
                split_multivalue(val)
            };
        }
        "depends" => {
            job.depends = if val.trim() == "*" {
                all_sections
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != &job.name)
                    .collect()
            } else {
                split_multivalue(val)
            };
        }
        "watch" => job.watch = split_multivalue(val),
        "notify-start" => job.notify_start = split_multivalue(val),
        "notify-success" => job.notify_success = split_multivalue(val),
        "notify-failure" => job.notify_failure = split_multivalue(val),
        "signal" => job.signal = Some(val.to_string()),
        "output" => job.output = Output::parse(val),
        "ionice" => job.ionice = val.trim() == "1",
        "nice" => job.nice = val.trim() == "1",
        _ => {}
    }
}
