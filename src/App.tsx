import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { killProcess, listPorts, type PortEntry } from "./api";
import {
  filterAndSort,
  formatBytes,
  rowKey,
  type ProtocolFilter,
  type SortDir,
  type SortKey,
} from "./filter";
import "./App.css";

const REFRESH_OPTIONS = [1, 2, 5, 10];

interface Toast {
  id: number;
  kind: "ok" | "error";
  text: string;
}

function App() {
  const [entries, setEntries] = useState<PortEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [protocol, setProtocol] = useState<ProtocolFilter>("ALL");
  const [listeningOnly, setListeningOnly] = useState(true);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [intervalSec, setIntervalSec] = useState(2);
  const [sortKey, setSortKey] = useState<SortKey>("port");
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const inFlight = useRef(false);
  const searchRef = useRef<HTMLInputElement>(null);

  const notify = useCallback((kind: Toast["kind"], text: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, kind, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
  }, []);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      setEntries(await listPorts(listeningOnly));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, [listeningOnly]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Auto refresh, paused while the window is hidden.
  useEffect(() => {
    if (!autoRefresh) return;
    const id = setInterval(() => {
      if (!document.hidden) refresh();
    }, intervalSec * 1000);
    return () => clearInterval(id);
  }, [autoRefresh, intervalSec, refresh]);

  // Keyboard shortcuts: Cmd/Ctrl+F focus search, Cmd/Ctrl+R refresh, Esc clear.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      } else if (mod && e.key.toLowerCase() === "r") {
        e.preventDefault();
        refresh();
      } else if (e.key === "Escape" && document.activeElement === searchRef.current) {
        setQuery("");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [refresh]);

  const visible = useMemo(
    () => filterAndSort(entries, query, protocol, sortKey, sortDir),
    [entries, query, protocol, sortKey, sortDir],
  );

  const onSort = (key: SortKey) => {
    if (key === sortKey) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    else {
      setSortKey(key);
      setSortDir(key === "memory" || key === "cpuUsage" ? "desc" : "asc");
    }
  };

  const onKill = async (entry: PortEntry, force: boolean) => {
    if (entry.pid == null) return;
    try {
      await killProcess(entry.pid, force);
      notify(
        "ok",
        `${force ? "Force killed" : "Terminated"} ${entry.processName ?? "process"} (PID ${entry.pid}) on port ${entry.port}`,
      );
      // Give the OS a moment to release the socket before re-scanning.
      setTimeout(refresh, 300);
    } catch (e) {
      notify("error", String(e));
    }
  };

  const onReveal = async (path: string) => {
    try {
      await revealItemInDir(path);
    } catch (e) {
      notify("error", String(e));
    }
  };

  const header = (key: SortKey, label: string, className?: string) => (
    <th className={className} onClick={() => onSort(key)} aria-sort={sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none"}>
      {label}
      <span className="sort-indicator">{sortKey === key ? (sortDir === "asc" ? "▲" : "▼") : ""}</span>
    </th>
  );

  return (
    <div className="app">
      <header className="toolbar">
        <div className="brand">
          <span className="logo">⚓</span>
          <h1>PortPilot</h1>
        </div>

        <input
          ref={searchRef}
          className="search"
          type="search"
          placeholder="Search port, process, PID…  (:3000, :8000-8999)"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
          spellCheck={false}
        />

        <div className="segmented" role="group" aria-label="Protocol">
          {(["ALL", "TCP", "UDP"] as ProtocolFilter[]).map((p) => (
            <button key={p} className={protocol === p ? "active" : ""} onClick={() => setProtocol(p)}>
              {p === "ALL" ? "All" : p}
            </button>
          ))}
        </div>

        <label className="toggle">
          <input type="checkbox" checked={listeningOnly} onChange={(e) => setListeningOnly(e.target.checked)} />
          Listening only
        </label>

        <div className="refresh-group">
          <label className="toggle">
            <input type="checkbox" checked={autoRefresh} onChange={(e) => setAutoRefresh(e.target.checked)} />
            Auto
          </label>
          <select value={intervalSec} onChange={(e) => setIntervalSec(Number(e.target.value))} disabled={!autoRefresh} aria-label="Refresh interval">
            {REFRESH_OPTIONS.map((s) => (
              <option key={s} value={s}>
                {s}s
              </option>
            ))}
          </select>
          <button className="icon-btn" onClick={refresh} title="Refresh (⌘R)">
            ↻
          </button>
        </div>
      </header>

      {error && <div className="banner error">{error}</div>}

      <main className="table-wrap">
        <table>
          <thead>
            <tr>
              {header("port", "Port", "num")}
              {header("protocol", "Proto")}
              <th>Address</th>
              {header("state", "State")}
              {header("processName", "Process")}
              {header("pid", "PID", "num")}
              {header("memory", "Memory", "num")}
              {header("cpuUsage", "CPU", "num")}
              <th className="actions-col" />
            </tr>
          </thead>
          <tbody>
            {visible.map((e) => (
              <Row key={rowKey(e)} entry={e} onKill={onKill} onReveal={onReveal} />
            ))}
          </tbody>
        </table>

        {!loading && visible.length === 0 && (
          <div className="empty">{entries.length === 0 ? "No open ports found." : "Nothing matches your filters."}</div>
        )}
        {loading && <div className="empty">Scanning ports…</div>}
      </main>

      <footer className="statusbar">
        <span>
          {visible.length} of {entries.length} sockets
        </span>
        <span className="muted">Rows without a process may belong to another user or the system.</span>
      </footer>

      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}

function Row({
  entry,
  onKill,
  onReveal,
}: {
  entry: PortEntry;
  onKill: (e: PortEntry, force: boolean) => void;
  onReveal: (path: string) => void;
}) {
  // Two-step confirmation instead of a modal dialog.
  const [confirming, setConfirming] = useState<null | "term" | "kill">(null);

  useEffect(() => {
    if (!confirming) return;
    const id = setTimeout(() => setConfirming(null), 3000);
    return () => clearTimeout(id);
  }, [confirming]);

  const click = (mode: "term" | "kill") => {
    if (confirming === mode) {
      setConfirming(null);
      onKill(entry, mode === "kill");
    } else {
      setConfirming(mode);
    }
  };

  const address =
    entry.remoteAddr != null
      ? `${entry.localAddr} → ${entry.remoteAddr}:${entry.remotePort}`
      : entry.localAddr;

  return (
    <tr>
      <td className="num port">{entry.port}</td>
      <td>
        <span className={`badge ${entry.protocol.toLowerCase()}`}>{entry.protocol}</span>
      </td>
      <td className="mono addr" title={address}>
        {address}
      </td>
      <td className="state">{entry.state}</td>
      <td className="process" title={entry.command ?? entry.exePath ?? ""}>
        {entry.processName ?? <span className="muted">unknown</span>}
      </td>
      <td className="num mono">{entry.pid ?? "—"}</td>
      <td className="num">{formatBytes(entry.memory)}</td>
      <td className="num">{entry.cpuUsage == null ? "—" : `${entry.cpuUsage.toFixed(1)}%`}</td>
      <td className="actions">
        <div className="actions-inner">
          {entry.exePath && (
            <button className="ghost" onClick={() => onReveal(entry.exePath!)} title="Show executable in file manager">
              Reveal
            </button>
          )}
          {entry.pid != null && (
            <>
              <button className={`danger ${confirming === "term" ? "confirm" : ""}`} onClick={() => click("term")} title="Send SIGTERM (graceful)">
                {confirming === "term" ? "Sure?" : "Kill"}
              </button>
              <button className={`danger ghost ${confirming === "kill" ? "confirm" : ""}`} onClick={() => click("kill")} title="Send SIGKILL (force)">
                {confirming === "kill" ? "Sure?" : "Force"}
              </button>
            </>
          )}
        </div>
      </td>
    </tr>
  );
}

export default App;
