import { invoke } from "@tauri-apps/api/core";

export interface PortEntry {
  protocol: "TCP" | "UDP";
  localAddr: string;
  port: number;
  state: string;
  remoteAddr: string | null;
  remotePort: number | null;
  pid: number | null;
  processName: string | null;
  exePath: string | null;
  command: string | null;
  memory: number | null;
  cpuUsage: number | null;
}

export interface KillResult {
  pid: number;
  forced: boolean;
}

export const listPorts = (listeningOnly: boolean) =>
  invoke<PortEntry[]>("list_ports", { listeningOnly });

export const killProcess = (pid: number, force: boolean) =>
  invoke<KillResult>("kill_process", { pid, force });
