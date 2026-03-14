# kubjas — daemon to execute scheduled commands

## NAME

kubjas — (cron-like) daemon to execute scheduled commands

## SYNOPSIS

```
kubjas [--background] [--conf-file /etc/kubjas.conf] [--log-file /path/kubjas.log] [--pid-file /path/kubjas.pid]
```

## DESCRIPTION

Kubjas is a periodic job scheduler that operates with minimum 1-second intervals.

Kubjas is not another cron daemon. Kubjas does not start programs at a certain time but at specified intervals. Kubjas also includes a **period** filter. You can configure **interval** and **period** combinations that act like crontab.

Kubjas measures executed job running times and logs them when a job exits. Measurements are in millisecond resolution.

Kubjas configuration is standard INI file format. You can have multiple configuration files at the same time. The main configuration is `/etc/kubjas.conf` and `/etc/kubjas.d/` is a directory for additional configurations. Each job can have its own config file. You can force a configuration reload with the **HUP** signal.

## INSTALLATION

### From source

```sh
cargo build --release
sudo cp target/release/kubjas /usr/local/bin/kubjas
```

### Requirements

- Linux, macOS, or Windows
- Rust 1.85+ (to build from source)

## CONFIGURATION

### example.conf

```ini
[*]
notify-failure = 127.0.0.1:send_failure_notify

[date-job]
cmdline = date +"%H:%M" > /var/tmp/date.txt
interval = 60
user = nobody
group = nogroup
notify-success = 192.168.1.27:catch-signals

[catch-signals]
cmdline = /usr/local/bin/catch-signals.sh
interval = success-message
signal = USR2

[readfile]
cmdline = /usr/local/bin/readfile.sh
interval = onchange
watch = /var/tmp/date.txt
output = /tmp/date.log
user = nobody
group = nogroup

[very-shy-job]
cmdline = /usr/local/bin/shy.sh
interval = 10-20
period = wday {1 3 5 7} min {0-29}, wday {2 4 6} min {30-59}
depends = catch-signals
conflicts = date-job
nice = 1
ionice = 1

[send_failure_notify]
cmdline = send_failure_notify.sh %host% %job% %notify%
interval = failure-message
output = none
```

### job-name

`[job-name]` is the INI file section. Job names must be unique.

The special section name `[*]` sets default params that will be used with all jobs defined in the same configuration file. Named job sections overwrite default params.

### cmdline

Parameter **cmdline** defines the executable program with parameters:

```
cmdline = /usr/local/bin/myscript.sh
cmdline = myscript.sh arg1 arg2
```

The second form works if the **PATH** environment variable includes the script's directory. Using full path names is recommended.

In combination with **watch** and **notify** you can use template parameters that are filled at execution time:

```
cmdline = send_alert.sh %host% %job% %notify%
```

- `%host%` — replaced with the hostname where the notify originates
- `%job%` — replaced with the job name that sent the notify
- `%notify%` — replaced with the notify type (`start-message`, `success-message`, `failure-message`) or the filename that triggered an `onchange` watch event

### output

Default is **passthrough** — all job STDOUT and STDERR are passed through to kubjas STDOUT or to the log file (if defined with `--log-file`).

Value **none** disables all output (equivalent to `2>&1 >/dev/null`):

```ini
output = none
```

If the value is a filename, kubjas opens it in append mode and forwards job STDOUT and STDERR there:

```ini
output = /var/log/job-name.log
```

### interval

Specifies the time in seconds between job starts. It is the minimum delay between runs — the actual delay may be longer if other conditions prevent running. `0` means the job is disabled.

Interval can also be defined as a randomized range. The following example starts the job every 20 to 30 seconds:

```ini
interval = 20-30
```

There are four special (non-numeric) intervals activated only by external events:

| Value | Trigger |
|---|---|
| `onchange` | File or directory watch event (see **watch**) |
| `start-message` | Notify received when another job starts |
| `success-message` | Notify received when another job exits successfully |
| `failure-message` | Notify received when another job exits with an error |

### period

Determines if a given time falls within a given period. Kubjas executes the job only if the current time matches the period. Period is optional.

Combined with **interval** you can emulate crontab behaviour. Example — run only once a day at midnight:

```ini
interval = 60
period = hr {12am} min {0}
```

A sub-period has the form:

```
scale {range [range ...]} [scale {range [range ...]}]
```

Comma-separated sub-periods are OR'd together. Within one sub-period all clauses must match (AND).

Scale must be one of:

| Scale | Code | Valid Range Values |
|-------|------|-------------------|
| year | yr | integer 0–99 or ≥1970 |
| month | mo | 1–12 or jan, feb, mar, apr, may, jun, jul, aug, sep, oct, nov, dec |
| week | wk | 1–6 |
| yday | yd | 1–365 |
| mday | md | 1–31 |
| wday | wd | 1–7 or su, mo, tu, we, th, fr, sa |
| hour | hr | 0–23 or 12am, 1am–11am, 12noon, 12pm, 1pm–11pm |
| minute | min | 0–59 |
| second | sec | 0–59 |

**crontab comparison [1]** — run every 5 minutes as nobody:

```
*/5 * * * *  nobody  cmdline
```
```ini
interval = 300
user = nobody
```

**crontab comparison [2]** — run at midnight on Sundays:

```
0 0 * * 7  cmdline
```
```ini
interval = 60
period = wd {su} hr {12am} min {0}
```

**crontab comparison [3]** — run at 2:15 pm on the first of every month:

```
15 14 1 * *  cmdline
```
```ini
period = md {1} hr {14} min {15} sec {0}
```

**crontab comparison [4]** — run at 10 pm on weekdays:

```
0 22 * * 1-5  cmdline
```
```ini
period = wd {mon-fri} hr {22} min {0} sec {0}
```

### user

Run the job as the given user. Kubjas resolves the user UID.

### group

Run the job as the given group. Kubjas resolves the group GID. Defaults to the same as `user`.

### watch

Kubjas monitors filesystem events if you specify files or directories to **watch**. One job can have multiple watch entries. Kubjas monitors write-close events (equivalent to inotify `IN_CLOSE_WRITE` on Linux).

```ini
watch = /var/tmp/date.txt
watch = /etc/myapp
```

Works together with `interval = onchange`.

### notify-start, notify-success, notify-failure

Kubjas notifies other local or remote jobs when the current job starts and ends. The target job's `interval` specifies when it should react.

```ini
[job-one]
cmdline = /usr/local/bin/step1.sh
interval = 60
notify-success = 127.0.0.1:job-two

[job-two]
cmdline = /usr/local/bin/step2.sh
interval = success-message
```

Target format: `hostname-or-ip:job-name`. Use `127.0.0.1` for local jobs.

Notifications are sent over UDP port **2380**.

When a job exits with a non-zero return code you can route failure notifications to a handler job:

```ini
[failure-handler]
cmdline = /usr/local/bin/send_email_to_admin.sh
interval = failure-message
```

### conflicts

This job will only run if none of the specified jobs are currently running. Useful for CPU-intensive jobs that must not overlap:

```ini
[hard-work]
conflicts = cpu-work1
conflicts = hard-work2
```

The special wildcard value `*` conflicts with every other job in the same config file:

```ini
[exclusive-job]
conflicts = *
```

### depends

This job will only run if all specified jobs are currently running:

```ini
[monitor]
depends = main-worker
```

The special wildcard value `*` requires every other job in the same config file to be running:

```ini
[ping]
depends = *
```

### nice, ionice

Decrease the executed job's CPU and I/O scheduler priority:

```ini
nice = 1
ionice = 1
```

`nice = 1` sets the process priority to +10 (equivalent to `renice +10`).
`ionice = 1` sets the I/O scheduler class to idle (equivalent to `ionice -c 3`). Linux only.

### signal

Combined with event-driven intervals, send a Unix signal to a running job when a notify or watch event occurs instead of (or before) starting a new instance:

```ini
[reload-on-change]
cmdline = /usr/local/bin/myserver
interval = onchange
watch = /etc/myapp/config.yaml
signal = HUP
```

Accepts a signal name (`HUP`, `USR1`, `USR2`, `TERM`, etc.) or a number.

## SIGNALS

| Signal | Effect |
|--------|--------|
| `HUP` | Reload all configuration files. Does not affect running jobs. |
| `USR1` | Print currently running jobs to the log. |
| `USR2` | Toggle job scheduling on/off. Useful before maintenance — signal USR2, wait for all jobs to finish, then restart safely. |
| `TERM` / `INT` | Graceful shutdown. Removes the PID file and exits. |

```sh
kill -HUP  $(cat /var/run/kubjas.pid)
kill -USR1 $(cat /var/run/kubjas.pid)
kill -USR2 $(cat /var/run/kubjas.pid)
```

## FILES

```
/etc/kubjas.conf
/etc/kubjas.d/
```

## SEE ALSO

`inotify(7)`, `crontab(5)`

## AUTHOR

Kain Kalju

Co-Authored-By: Claude Sonnet 4.6

## LICENSE

MIT License — Copyright (c) 2026 Kain Kalju.
See [LICENSE](LICENSE) for full text.
