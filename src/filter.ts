import type { PortEntry } from "./api";

export type SortKey = "port" | "protocol" | "processName" | "pid" | "memory" | "cpuUsage" | "state";
export type SortDir = "asc" | "desc";
export type ProtocolFilter = "ALL" | "TCP" | "UDP";

/**
 * Search syntax:
 *   3000        → exact port (or exact PID)
 *   :3000       → exact port only
 *   8000-8999   → port range (":8000-8999" also works)
 *   node        → process name / address / command contains "node"
 * Multiple space-separated terms are AND-ed.
 */
export function matchesQuery(entry: PortEntry, query: string): boolean {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  return terms.every((term) => {
    const range = term.match(/^:?(\d+)-(\d+)$/);
    if (range) {
      const [from, to] = [Number(range[1]), Number(range[2])];
      return entry.port >= Math.min(from, to) && entry.port <= Math.max(from, to);
    }
    const exact = term.match(/^(:?)(\d+)$/);
    if (exact) {
      const n = Number(exact[2]);
      // ":3000" is strictly a port; a bare number also matches a PID.
      return entry.port === n || (exact[1] === "" && entry.pid === n);
    }
    const haystack = [
      entry.processName ?? "",
      ...entry.localAddrs,
      entry.remoteAddr ?? "",
      entry.state,
      entry.command ?? "",
    ]
      .join(" ")
      .toLowerCase();
    return haystack.includes(term);
  });
}

export function filterAndSort(
  entries: PortEntry[],
  query: string,
  protocol: ProtocolFilter,
  sortKey: SortKey,
  sortDir: SortDir,
): PortEntry[] {
  const dir = sortDir === "asc" ? 1 : -1;
  return entries
    .filter((e) => protocol === "ALL" || e.protocol === protocol)
    .filter((e) => matchesQuery(e, query))
    .sort((a, b) => {
      const av = a[sortKey];
      const bv = b[sortKey];
      // Nulls always go last regardless of direction.
      if (av == null && bv == null) return 0;
      if (av == null) return 1;
      if (bv == null) return -1;
      const cmp =
        typeof av === "string" && typeof bv === "string"
          ? av.localeCompare(bv, undefined, { sensitivity: "base" })
          : (av as number) - (bv as number);
      return cmp * dir || a.port - b.port;
    });
}

export function formatBytes(bytes: number | null): string {
  if (bytes == null) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Stable key for a row; a port can be bound by several processes. */
export const rowKey = (e: PortEntry) =>
  `${e.protocol}|${e.port}|${e.state}|${e.remoteAddr}|${e.remotePort}|${e.pid}`;

/** Returns the port number if the query is a single port ("3000" or ":3000"), otherwise null. */
export function exactPortQuery(query: string): number | null {
  const m = query.trim().match(/^:?(\d{1,5})$/);
  if (!m) return null;
  const port = Number(m[1]);
  return port > 0 && port <= 65535 ? port : null;
}

/** Entries that actually occupy a port (TCP LISTEN or bound UDP). */
export const occupiers = (entries: PortEntry[], port: number) =>
  entries.filter((e) => e.port === port && (e.state === "LISTEN" || e.protocol === "UDP"));

/** Well-known OS services that grab common development ports. */
export const PORT_HINTS: Record<number, string> = {
  5000: "On macOS, AirPlay Receiver (ControlCenter) uses ports 5000 and 7000. Turn it off in System Settings → General → AirDrop & Handoff.",
  7000: "On macOS, AirPlay Receiver (ControlCenter) uses ports 5000 and 7000. Turn it off in System Settings → General → AirDrop & Handoff.",
};
