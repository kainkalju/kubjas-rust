//! UDP notification system on port 2380.
//!
//! Message format (space-separated):
//!   <from_host> <to_job> <from_job> <notify_type> <timestamp>
//!
//! Valid notify types: start-message, success-message, failure-message, ping

use crc32fast::Hasher;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};

pub const NOTIFY_PORT: u16 = 2380;

#[derive(Debug, Clone, PartialEq)]
pub enum NotifyType {
    StartMessage,
    SuccessMessage,
    FailureMessage,
    Ping,
}

impl NotifyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NotifyType::StartMessage => "start-message",
            NotifyType::SuccessMessage => "success-message",
            NotifyType::FailureMessage => "failure-message",
            NotifyType::Ping => "ping",
        }
    }
    fn from_str(s: &str) -> Option<NotifyType> {
        match s {
            "start-message" => Some(NotifyType::StartMessage),
            "success-message" => Some(NotifyType::SuccessMessage),
            "failure-message" => Some(NotifyType::FailureMessage),
            "ping" => Some(NotifyType::Ping),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotifyMessage {
    pub to_job: String,
    pub notify_type: NotifyType,
    pub from_job: String,
    pub from_host: String,
}

/// Duplicate message filter using CRC32, keeps last 20 fingerprints.
pub struct DuplicateFilter {
    known: HashMap<u32, String>,
    lifo: VecDeque<u32>,
}

impl DuplicateFilter {
    pub fn new() -> Self {
        DuplicateFilter {
            known: HashMap::new(),
            lifo: VecDeque::new(),
        }
    }

    /// Returns true if the message is a duplicate (should be ignored).
    pub fn is_duplicate(&mut self, msg: &str) -> bool {
        let fingerprint = crc32(msg.as_bytes());
        if let Some(existing) = self.known.get(&fingerprint) {
            if existing == msg {
                return true;
            }
        }
        self.known.insert(fingerprint, msg.to_string());
        self.lifo.push_back(fingerprint);
        if self.lifo.len() > 20 {
            if let Some(old_fp) = self.lifo.pop_front() {
                self.known.remove(&old_fp);
            }
        }
        false
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Parse and validate an incoming UDP packet.
/// Returns None if duplicate, invalid format, or unknown notify type.
pub fn recv_notify(
    buf: &[u8],
    remote: SocketAddr,
    filter: &mut DuplicateFilter,
) -> Option<NotifyMessage> {
    let packet = std::str::from_utf8(buf).ok()?.trim_end_matches('\n').trim();
    if filter.is_duplicate(packet) {
        return None;
    }
    let remote_addr = match remote {
        SocketAddr::V4(a) => a.ip().to_string(),
        SocketAddr::V6(a) => a.ip().to_string(),
    };
    let port = remote.port();
    log::info!("notify from {}:{} {{{}}}", remote_addr, port, packet);

    let parts: Vec<&str> = packet.splitn(5, ' ').collect();
    if parts.len() < 4 {
        return None;
    }
    let _from_host_in_msg = parts[0];
    let to_job = parts[1];
    let from_job = parts[2];
    let notify_str = parts[3];

    let notify_type = NotifyType::from_str(notify_str)?;
    Some(NotifyMessage {
        to_job: to_job.to_string(),
        notify_type,
        from_job: from_job.to_string(),
        from_host: remote_addr,
    })
}

/// Send a notify message to a remote target in a forked child process.
/// `target` is in the form "hostname:job-name" or "ip:job-name".
/// `my_job` is the name of the job sending the notify.
/// `my_host` is our own hostname.
pub fn send_notify(target: &str, my_job: &str, notify: &NotifyType, my_host: &str) {
    let target = target.to_string();
    let my_job = my_job.to_string();
    let my_host = my_host.to_string();
    let notify_str = notify.as_str().to_string();

    // Fork a child so sending doesn't block the scheduler
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            log::error!("send_notify: fork failed");
            return;
        }
        if pid > 0 {
            // parent
            return;
        }
        // child
        send_notify_child(&target, &my_job, &notify_str, &my_host);
        libc::_exit(0);
    }
}

fn send_notify_child(target: &str, my_job: &str, notify_str: &str, my_host: &str) {
    let (remote_host, remote_job) = match target.rsplit_once(':') {
        Some(p) => p,
        None => {
            log::warn!("send_notify: invalid target '{}'", target);
            return;
        }
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let message = format!(
        "{} {} {} {} {}",
        my_host, remote_job, my_job, notify_str, timestamp
    );

    let remote_addr = format!("{}:{}", remote_host, NOTIFY_PORT);
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            log::warn!("send_notify: cannot bind socket: {}", e);
            return;
        }
    };
    if let Err(e) = sock.connect(&remote_addr) {
        log::warn!("send_notify: cannot connect to {}: {}", remote_addr, e);
        return;
    }

    let timeout = std::time::Duration::from_secs(1);
    let _ = sock.set_read_timeout(Some(timeout));

    let mut attempts = 0;
    loop {
        if let Err(e) = sock.send(message.as_bytes()) {
            log::warn!("send_notify: send failed: {}", e);
            break;
        }
        let mut buf = [0u8; 1024];
        match sock.recv(&mut buf) {
            Ok(n) => {
                let resp = std::str::from_utf8(&buf[..n]).unwrap_or("");
                if !resp.starts_with("OK") {
                    log::warn!("send_notify: {} did not respond OK", remote_host);
                }
                break;
            }
            Err(_) => {
                attempts += 1;
                if attempts >= 3 {
                    log::warn!("send_notify: {} did not respond to notify!", remote_host);
                    break;
                }
            }
        }
    }
}
