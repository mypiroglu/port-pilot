//! Port and process discovery / control.
//!
//! Sockets come from `netstat2` (cross-platform, no shelling out to `lsof`/`netstat`)
//! and process details come from `sysinfo`.

use std::collections::HashSet;
use std::net::IpAddr;

use netstat2::{
    iterate_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState,
};
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

/// One row in the UI: a socket bound by a process.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortEntry {
    /// "TCP" or "UDP"
    pub protocol: &'static str,
    pub local_addr: String,
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
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillResult {
    pub pid: u32,
    pub forced: bool,
}

fn fmt_addr(addr: IpAddr) -> String {
    match addr {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
        IpAddr::V4(v4) => v4.to_string(),
    }
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

    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );

    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for socket in sockets {
        let (protocol, local_addr, port, state, remote) = match &socket.protocol_socket_info {
            ProtocolSocketInfo::Tcp(tcp) => {
                if listening_only && tcp.state != TcpState::Listen {
                    continue;
                }
                let remote = (tcp.state != TcpState::Listen)
                    .then(|| (fmt_addr(tcp.remote_addr), tcp.remote_port));
                (
                    "TCP",
                    fmt_addr(tcp.local_addr),
                    tcp.local_port,
                    tcp.state.to_string(),
                    remote,
                )
            }
            ProtocolSocketInfo::Udp(udp) => (
                "UDP",
                fmt_addr(udp.local_addr),
                udp.local_port,
                "BOUND".to_string(),
                None,
            ),
        };

        let pids: Vec<Option<u32>> = if socket.associated_pids.is_empty() {
            vec![None]
        } else {
            socket.associated_pids.iter().copied().map(Some).collect()
        };

        for pid in pids {
            // The same socket shows up for IPv4 + IPv6 and for forked children; dedupe.
            let key = (
                protocol,
                local_addr.clone(),
                port,
                state.clone(),
                remote.clone(),
                pid,
            );
            if !seen.insert(key) {
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

            out.push(PortEntry {
                protocol,
                local_addr: local_addr.clone(),
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
            });
        }
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

    let spid = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[spid]), true);
    let process = sys
        .process(spid)
        .ok_or_else(|| format!("Process {pid} no longer exists."))?;

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
        Err(format!(
            "Could not terminate process {pid}. It may belong to another user or the system \
             (try running with elevated privileges)."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

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
        assert!(entry.process_name.is_some());
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
