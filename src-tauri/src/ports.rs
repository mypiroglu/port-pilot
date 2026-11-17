//! Port and process discovery / control.
//!
//! Sockets come from `netstat2` (cross-platform, no shelling out to `lsof`/`netstat`)
//! and process details come from `sysinfo`.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::Path;

use netstat2::{
    iterate_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState,
};
use serde::Serialize;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, Signal, System, Uid, UpdateKind};

/// One row in the UI: a port bound by a process.
///
/// The same port bound on several addresses (typically `0.0.0.0` and `::`)
/// by the same process is merged into a single row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortEntry {
    /// "TCP" or "UDP"
    pub protocol: &'static str,
    /// All local addresses this port is bound on, IPv4 first.
    pub local_addrs: Vec<String>,
    pub port: u16,
    /// TCP state (LISTEN, ESTABLISHED, ...). UDP sockets are reported as "BOUND".
    pub state: String,
    pub remote_addr: Option<String>,
    pub remote_port: Option<u16>,
    /// `None` when the OS did not expose the owning process (usually a permission issue).
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub exe_path: Option<String>,
    pub command: Option<String>,
    /// Resident memory in bytes.
    pub memory: Option<u64>,
    pub cpu_usage: Option<f32>,
    /// OS service or a process owned by another user. Killing these is blocked.
    pub is_system: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillResult {
    pub pid: u32,
    pub forced: bool,
}

/// Executables under these prefixes belong to the operating system.
const SYSTEM_PATH_PREFIXES: &[&str] = &[
    "/System/",
    "/usr/libexec/",
    "/usr/sbin/",
    "/sbin/",
    "/Library/Apple/",
    "/lib/systemd/",
    "/usr/lib/systemd/",
];

fn fmt_addr(addr: IpAddr) -> String {
    match addr {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
        IpAddr::V4(v4) => v4.to_string(),
    }
}

fn is_system_path(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    SYSTEM_PATH_PREFIXES.iter().any(|p| s.starts_with(p))
        || s.to_ascii_lowercase().starts_with(r"c:\windows\")
}

/// Heuristic: a process is "system" if it runs as another user (root, daemons)
/// or its executable lives in an OS directory.
fn is_system_process(pid: u32, process: Option<&Process>, current_uid: Option<&Uid>) -> bool {
    if pid <= 1 {
        return true;
    }
    let Some(p) = process else {
        // We could not even read the process: it is not ours.
        return true;
    };
    if let (Some(owner), Some(me)) = (p.user_id(), current_uid) {
        if owner != me {
            return true;
        }
    }
    p.exe().is_some_and(is_system_path)
}

fn current_uid(sys: &mut System) -> Option<Uid> {
    let me = sysinfo::get_current_pid().ok()?;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[me]),
        false,
        ProcessRefreshKind::nothing().with_user(UpdateKind::OnlyIfNotSet),
    );
    sys.process(me)?.user_id().cloned()
}

fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_memory()
        .with_cpu()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet)
}

/// Collect sockets. `listening_only` keeps TCP LISTEN sockets and all UDP sockets
/// (UDP has no connection state, a bound UDP socket is effectively "listening").
pub fn list_ports(sys: &mut System, listening_only: bool) -> Result<Vec<PortEntry>, String> {
    let iter = iterate_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP | ProtocolFlags::UDP,
    )
    .map_err(|e| format!("Failed to read sockets: {e}"))?;

    // Individual sockets can fail (e.g. a process exiting mid-scan); skip them
    // instead of failing the whole listing.
    let sockets: Vec<_> = iter.filter_map(Result::ok).collect();

    // Refresh only the processes we actually need.
    let pids: Vec<Pid> = sockets
        .iter()
        .flat_map(|s| s.associated_pids.iter().copied())
        .collect::<HashSet<u32>>()
        .into_iter()
        .map(Pid::from_u32)
        .collect();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, process_refresh_kind());
    let me = current_uid(sys);

    type Key = (&'static str, u16, String, Option<(String, u16)>, Option<u32>);
    let mut index: HashMap<Key, usize> = HashMap::new();
    let mut out: Vec<PortEntry> = Vec::new();

    for socket in sockets {
        let (protocol, local_addr, port, state, remote) = match &socket.protocol_socket_info {
            ProtocolSocketInfo::Tcp(tcp) => {
                if listening_only && tcp.state != TcpState::Listen {
                    continue;
                }
                let remote = (tcp.state != TcpState::Listen)
                    .then(|| (fmt_addr(tcp.remote_addr), tcp.remote_port));
                ("TCP", tcp.local_addr, tcp.local_port, tcp.state.to_string(), remote)
            }
            ProtocolSocketInfo::Udp(udp) => {
                ("UDP", udp.local_addr, udp.local_port, "BOUND".to_string(), None)
            }
        };

        // Port 0 means "not bound yet" — meaningless to the user.
        if port == 0 {
            continue;
        }

        let pids: Vec<Option<u32>> = if socket.associated_pids.is_empty() {
            vec![None]
        } else {
            socket.associated_pids.iter().copied().map(Some).collect()
        };

        for pid in pids {
            let key: Key = (protocol, port, state.clone(), remote.clone(), pid);
            if let Some(&i) = index.get(&key) {
                let addr = fmt_addr(local_addr);
                if !out[i].local_addrs.contains(&addr) {
                    out[i].local_addrs.push(addr);
                }
                continue;
            }

            let proc_ = pid.and_then(|p| sys.process(Pid::from_u32(p)));
            let command = proc_.and_then(|p| {
                let parts: Vec<String> = p
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect();
                (!parts.is_empty()).then(|| parts.join(" "))
            });

            index.insert(key, out.len());
            out.push(PortEntry {
                protocol,
                local_addrs: vec![fmt_addr(local_addr)],
                port,
                state: state.clone(),
                remote_addr: remote.as_ref().map(|r| r.0.clone()),
                remote_port: remote.as_ref().map(|r| r.1),
                pid,
                process_name: proc_.map(|p| p.name().to_string_lossy().into_owned()),
                exe_path: proc_.and_then(|p| p.exe()).map(|e| e.display().to_string()),
                command,
                memory: proc_.map(|p| p.memory()),
                cpu_usage: proc_.map(|p| p.cpu_usage()),
                is_system: pid.is_none_or(|p| is_system_process(p, proc_, me.as_ref())),
            });
        }
    }

    for entry in &mut out {
        // IPv4 before IPv6 for a stable, readable order.
        entry.local_addrs.sort_by_key(|a| (a.contains(':'), a.clone()));
    }
    out.sort_by(|a, b| {
        a.port
            .cmp(&b.port)
            .then(a.protocol.cmp(b.protocol))
            .then(a.pid.cmp(&b.pid))
    });
    Ok(out)
}

/// Terminate a process. Sends SIGTERM first (graceful) unless `force` is set,
/// in which case SIGKILL is used. On Windows there is only a hard kill.
pub fn kill_process(sys: &mut System, pid: u32, force: bool) -> Result<KillResult, String> {
    if pid <= 1 {
        return Err("Refusing to kill a system process (PID 0/1).".into());
    }
    if pid == std::process::id() {
        return Err("Refusing to kill PortPilot itself.".into());
    }

    let me = current_uid(sys);
    let spid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[spid]), true, process_refresh_kind());
    let process = sys
        .process(spid)
        .ok_or_else(|| format!("Process {pid} no longer exists."))?;

    if is_system_process(pid, Some(process), me.as_ref()) {
        return Err(format!(
            "{} (PID {pid}) is a system process and is protected.",
            process.name().to_string_lossy()
        ));
    }

    let ok = if force {
        process.kill_with(Signal::Kill).unwrap_or_else(|| process.kill())
    } else {
        // `None` means the signal is not supported on this platform (Windows).
        match process.kill_with(Signal::Term) {
            Some(ok) => ok,
            None => process.kill(),
        }
    };

    if ok {
        Ok(KillResult { pid, forced: force })
    } else {
        Err(format!("Could not terminate process {pid}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, UdpSocket};

    #[test]
    fn finds_own_listening_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut sys = System::new().unwrap();

        let ports = list_ports(&mut sys, true).unwrap();
        let entry = ports
            .iter()
            .find(|p| p.port == port && p.protocol == "TCP")
            .expect("own port should be listed");

        assert_eq!(entry.state, "LISTEN");
        assert_eq!(entry.pid, Some(std::process::id()));
        assert_eq!(entry.local_addrs, vec!["127.0.0.1".to_string()]);
        assert!(entry.process_name.is_some());
        assert!(!entry.is_system, "test binary must not be flagged as system");
    }

    #[test]
    fn merges_ipv4_and_ipv6_on_same_port() {
        let v4 = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = v4.local_addr().unwrap().port();
        // Prefer IPv6 loopback; fall back to a second IPv4 loopback address
        // (Linux CI containers often have no IPv6).
        let (_second, expected) = match TcpListener::bind(("::1", port)) {
            Ok(l) => (l, vec!["127.0.0.1", "::1"]),
            Err(_) => match TcpListener::bind(("127.0.0.2", port)) {
                Ok(l) => (l, vec!["127.0.0.1", "127.0.0.2"]),
                Err(_) => return, // neither available on this machine
            },
        };
        let mut sys = System::new().unwrap();

        let rows: Vec<_> = list_ports(&mut sys, true)
            .unwrap()
            .into_iter()
            .filter(|p| p.port == port && p.protocol == "TCP")
            .collect();

        assert_eq!(rows.len(), 1, "IPv4 + IPv6 should be one row: {rows:?}");
        assert_eq!(rows[0].local_addrs, expected);
    }

    #[test]
    fn skips_port_zero_and_keeps_udp() {
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = udp.local_addr().unwrap().port();
        let mut sys = System::new().unwrap();
        let ports = list_ports(&mut sys, true).unwrap();
        assert!(ports.iter().all(|p| p.port != 0));
        assert!(ports.iter().any(|p| p.port == port && p.protocol == "UDP"));
    }

    #[test]
    fn system_path_detection() {
        assert!(is_system_path(Path::new("/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter")));
        assert!(is_system_path(Path::new("/usr/libexec/rapportd")));
        assert!(is_system_path(Path::new(r"C:\Windows\System32\svchost.exe")));
        assert!(!is_system_path(Path::new("/usr/local/bin/node")));
        assert!(!is_system_path(Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")));
    }

    #[test]
    fn refuses_to_kill_self_and_init() {
        let mut sys = System::new().unwrap();
        assert!(kill_process(&mut sys, std::process::id(), false).is_err());
        assert!(kill_process(&mut sys, 1, true).is_err());
        assert!(kill_process(&mut sys, 0, true).is_err());
    }

    #[test]
    fn kills_child_process() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let mut sys = System::new().unwrap();
        kill_process(&mut sys, child.id(), false).unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
    }
}
