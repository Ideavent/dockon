import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  QueryClient,
  QueryClientProvider,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  Box,
  FolderGit2,
  Activity,
  Play,
  Square,
  RefreshCw,
  Terminal,
  ArrowUpRight,
  Layers3,
  Globe,
  Plus,
  Circle,
} from "lucide-react";
import { Button } from "./components/ui/button";
import "./styles.css";

type Project = {
  manifest: { id: string; name: string; root: string; executable: string };
  state: string;
  pid: number | null;
  port: number | null;
  error: string | null;
};
type Event = { seq: number; project_id: string; kind: string; message: string };
type Operation = { id: string; status: string; error: string | null };
type Reply<T> = { ok: boolean; data: T; error?: string };
const words = {
  tr: {
    projects: "Projeler",
    subtitle: "Geliştirme ortamların. Kontrol sende.",
    connected: "Daemon bağlı",
    disconnected: "Daemon bağlantısı yok",
    start: "Başlat",
    stop: "Durdur",
    running: "Çalışan",
    registered: "Kayıtlı proje",
    activity: "Canlı günlükler",
    refresh: "Yeniden bağlan",
    empty: "Henüz proje yok",
    emptyHelp: "Bir Dockon proje manifesti içe aktararak başla.",
    import: "Proje içe aktar",
    manifest: "Manifest dosyasının tam yolu",
    local: "Yerel çalışma alanı",
    noEvents: "Proje olayları burada görünecek.",
    foundation: "Temel sürüm · Sprint 01",
    native: "Yerel süreç",
    open: "Tarayıcıda aç",
    cancel: "Vazgeç",
    offline: "Masaüstü bağlantısı gerekli",
    offlineHelp:
      "Gerçek proje yönetimi için Dockon masaüstü uygulamasını çalıştır.",
    logs: "Etkinlik",
    all: "Tüm projeler",
  },
  en: {
    projects: "Projects",
    subtitle: "Your development environments. In your control.",
    connected: "Daemon connected",
    disconnected: "Daemon disconnected",
    start: "Start",
    stop: "Stop",
    running: "Running",
    registered: "Registered projects",
    activity: "Live logs",
    refresh: "Reconnect",
    empty: "No projects yet",
    emptyHelp: "Import a Dockon project manifest to get started.",
    import: "Import project",
    manifest: "Absolute path to manifest file",
    local: "Local workspace",
    noEvents: "Project events will appear here.",
    foundation: "Foundation · Sprint 01",
    native: "Native process",
    open: "Open in browser",
    cancel: "Cancel",
    offline: "Desktop connection required",
    offlineHelp:
      "Launch the Dockon desktop application to manage real projects.",
    logs: "Activity",
    all: "All projects",
  },
};
async function request<T>(method: string, params?: object): Promise<T> {
  const response = await invoke<Reply<T>>("request", {
    command: { method, ...(params ? { params } : {}) },
  }).catch((error) => {
    throw new Error(String(error));
  });
  if (!response.ok) throw new Error(response.error || "Daemon request failed");
  return response.data;
}
function App() {
  const [lang, setLang] = useState<"tr" | "en">("tr");
  const t = words[lang];
  const desktop = "__TAURI_INTERNALS__" in window;
  const cache = useQueryClient();
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState("");
  const [importing, setImporting] = useState(false);
  const [path, setPath] = useState("");
  const [events, setEvents] = useState<Event[]>([]);
  const [selected, setSelected] = useState("");
  const cursor = useRef(0);
  const epoch = useRef("");
  const connect = useMutation({
    mutationFn: () => invoke<Reply<unknown>>("connect"),
    onSuccess: (reply) => {
      setConnected(reply.ok);
      setError(reply.error || "");
      cursor.current = 0;
      setEvents([]);
    },
    onError: (e) => {
      setConnected(false);
      setError(String(e));
    },
  });
  useEffect(() => {
    if (desktop) connect.mutate();
  }, []);
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => request<Project[]>("list"),
    enabled: connected,
    refetchInterval: 1500,
    retry: false,
  });
  useEffect(() => {
    if (!connected) return;
    const channel = new Channel<{
      epoch: string;
      events: Event[];
      truncated: boolean;
    }>();
    channel.onmessage = (batch) => {
      const changed = batch.epoch !== epoch.current;
      if (changed) {
        epoch.current = batch.epoch;
        cursor.current = 0;
      }
      const next = batch.events.filter((event) => event.seq > cursor.current);
      if (next.length) cursor.current = next[next.length - 1].seq;
      if (changed || next.length)
        setEvents((old) => [...(changed ? [] : old), ...next].slice(-300));
      if (
        changed ||
        next.some((event) => event.kind === "state" || event.kind === "error")
      )
        cache.invalidateQueries({ queryKey: ["projects"] });
      if (batch.truncated)
        setError(
          lang === "tr"
            ? "Günlük tamponu doldu; eski olayların bir bölümü artık mevcut değil."
            : "Log buffer overflow: some earlier events are no longer available.",
        );
    };
    invoke("watch_events", { channel }).catch((e) => setError(String(e)));
    return () => {
      invoke("stop_events").catch(() => {});
    };
  }, [connected]);
  const action = useMutation({
    mutationFn: async ({
      method,
      params,
    }: {
      method: string;
      params: object;
    }) => {
      const reply = await request<Operation>(method, params);
      if (method === "start" || method === "stop") {
        for (let attempt = 0; attempt < 200; attempt++) {
          const operation = await request<Operation>("operation", {
            id: reply.id,
          });
          if (operation.status === "completed") return operation;
          if (operation.status !== "running")
            throw new Error(operation.error || operation.status);
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
        throw new Error(`Operation ${reply.id}: status check timed out`);
      }
      return reply;
    },
    onSuccess: () => {
      setError("");
      setImporting(false);
      cache.invalidateQueries({ queryKey: ["projects"] });
    },
    onError: (e) => setError(String(e)),
    onSettled: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const items = projects.data || [];
  const visible = selected
    ? events.filter((e) => e.project_id === selected)
    : events;
  const failure = error || projects.error?.message;
  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-icon">
            <Layers3 size={23} />
          </div>
          dockon<span className="version">alpha</span>
        </div>
        <div className="workspace-label">WORKSPACE</div>
        <div className="nav-current">
          <FolderGit2 size={18} />
          {t.projects}
          <span>{items.length}</span>
        </div>
        <a className="nav-link" href="#activity">
          <Activity size={18} />
          {t.logs}
        </a>
        <div className="sidebar-bottom">
          <div className="local-icon">
            <Box size={19} />
          </div>
          <div>
            {t.local}
            <small>{t.foundation}</small>
          </div>
        </div>
      </aside>
      <main>
        <header>
          <span>
            WORKSPACE <span className="slash">/</span> {t.projects}
          </span>
          <div className="header-tools">
            <button
              className="language"
              onClick={() => setLang(lang === "tr" ? "en" : "tr")}
            >
              <Globe size={14} />
              {lang.toUpperCase()}
            </button>
            <span
              className={
                "connection " + (connected && !projects.isError ? "online" : "")
              }
            >
              <Circle size={7} fill="currentColor" />
              {connected && !projects.isError ? t.connected : t.disconnected}
            </span>
          </div>
        </header>
        <div className="content">
          <section className="page-heading">
            <div>
              <div className="eyebrow">BUILD SOMETHING GREAT</div>
              <h1>{t.projects}</h1>
              <p>{t.subtitle}</p>
            </div>
            <Button
              variant="primary"
              disabled={!connected || projects.isError || action.isPending}
              onClick={() => setImporting(true)}
            >
              <Plus size={17} />
              {t.import}
            </Button>
          </section>
          {!desktop && (
            <div className="notice">
              <strong>{t.offline}</strong>
              <p>{t.offlineHelp}</p>
            </div>
          )}
          {failure && (
            <div className="error" role="alert">
              {failure}
            </div>
          )}
          {desktop && (!connected || projects.isError) && (
            <Button
              disabled={connect.isPending}
              onClick={() => connect.mutate()}
            >
              <RefreshCw size={15} />
              {t.refresh}
            </Button>
          )}
          <section className="stats">
            <div>
              <span>{t.registered}</span>
              <strong>
                {connected && !projects.isError && projects.data
                  ? items.length
                  : "—"}
              </strong>
              <FolderGit2 size={20} />
            </div>
            <div>
              <span>{t.running}</span>
              <strong>
                {connected && !projects.isError && projects.data
                  ? items.filter((p) => p.state === "RUNNING").length
                  : "—"}
              </strong>
              <Activity size={20} />
            </div>
            <div>
              <span>PROVIDER</span>
              <strong className="stat-word">{t.native}</strong>
              <Box size={20} />
            </div>
          </section>
          <div className="section-label">
            {t.all}
            <span>{items.length}</span>
          </div>
          <section className="project-grid">
            {items.map((project) => (
              <article className="project-card" key={project.manifest.id}>
                <div className="card-top">
                  <div className="project-icon">
                    <Terminal size={22} />
                  </div>
                  <span
                    className={"status status-" + project.state.toLowerCase()}
                  >
                    <Circle size={6} fill="currentColor" />
                    {project.state}
                  </span>
                </div>
                <h2>{project.manifest.name}</h2>
                <p className="project-path" title={project.manifest.root}>
                  {project.manifest.root}
                </p>
                <div className="project-meta">
                  <span>
                    <Box size={13} />
                    native
                  </span>
                  <span>PID {project.pid ?? "—"}</span>
                  <span>:{project.port ?? "—"}</span>
                </div>
                {project.error && (
                  <p className="project-error">{project.error}</p>
                )}
                <div className="card-actions">
                  <Button
                    variant={
                      project.state === "RUNNING" ? "secondary" : "primary"
                    }
                    disabled={
                      projects.isError ||
                      action.isPending ||
                      project.state === "STARTING" ||
                      project.state === "STOPPING"
                    }
                    onClick={() =>
                      action.mutate({
                        method: project.state === "RUNNING" ? "stop" : "start",
                        params: { id: project.manifest.id },
                      })
                    }
                  >
                    {project.state === "RUNNING" ? (
                      <Square size={13} />
                    ) : (
                      <Play size={13} />
                    )}{" "}
                    {project.state === "RUNNING" ? t.stop : t.start}
                  </Button>
                  {project.port && project.state === "RUNNING" && (
                    <a
                      className="open-link"
                      href={`http://127.0.0.1:${project.port}`}
                      target="_blank"
                      rel="noreferrer"
                      aria-label={t.open}
                    >
                      <ArrowUpRight size={18} />
                    </a>
                  )}
                </div>
              </article>
            ))}
            {connected && !projects.isPending && !items.length && (
              <div className="empty">
                <FolderGit2 size={32} />
                <h2>{t.empty}</h2>
                <p>{t.emptyHelp}</p>
              </div>
            )}
          </section>
          <section className="logs" id="activity">
            <div className="log-header">
              <h2>
                <Terminal size={16} />
                {t.activity}
              </h2>
              <select
                aria-label={t.all}
                value={selected}
                onChange={(e) => setSelected(e.target.value)}
              >
                <option value="">{t.all}</option>
                {items.map((p) => (
                  <option key={p.manifest.id} value={p.manifest.id}>
                    {p.manifest.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="log-body" role="log">
              {!visible.length && <div className="log-empty">{t.noEvents}</div>}
              {visible.map((event) => (
                <div
                  className={
                    "log-line " +
                    (event.kind === "error" || event.kind === "stderr"
                      ? "log-error"
                      : "")
                  }
                  key={event.seq}
                >
                  <span>{String(event.seq).padStart(4, "0")}</span>
                  <span>{event.project_id}</span>
                  <code>{event.message}</code>
                </div>
              ))}
            </div>
          </section>
        </div>
        <footer>
          <span>
            <Circle size={6} fill="currentColor" /> DOCKON LOCAL ENGINE
          </span>
          <span>v0.1.0 · {t.foundation}</span>
        </footer>
      </main>
      {importing && (
        <div className="modal-backdrop">
          <form
            className="modal"
            onSubmit={(e) => {
              e.preventDefault();
              action.mutate({
                method: "register",
                params: { manifest_path: path },
              });
            }}
          >
            <h2>{t.import}</h2>
            <label htmlFor="manifest">{t.manifest}</label>
            <input
              id="manifest"
              autoFocus
              value={path}
              onChange={(e) => setPath(e.target.value)}
              required
            />
            <p>dockon.project.json</p>
            <div>
              <Button type="button" onClick={() => setImporting(false)}>
                {t.cancel}
              </Button>
              <Button variant="primary" disabled={action.isPending}>
                {t.import}
              </Button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}
const client = new QueryClient();
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
