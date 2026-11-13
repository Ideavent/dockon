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
import { request, type Reply } from "./ipc/client";
import { RuntimePage } from "./features/runtimes/RuntimePage";
import { ProjectWizard } from "./features/projects/ProjectWizard";

type Project = {
  manifest: {
    id: string;
    name: string;
    root: string;
    executable: string;
    runtime?: { installation_id: string };
    provider?: "native" | "container";
    health?: { type: "http" | "tcp" };
    services?: { provider?: "native" | "container" }[];
  };
  state: string;
  pid: number | null;
  port: number | null;
  error: string | null;
  container_id?: string | null;
  services?: { id: string; state: string; port: number }[];
};
type Event = { seq: number; project_id: string; kind: string; message: string };
type Operation = { id: string; status: string; error: string | null };
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
    foundation: "Geliştirme sürümü · Sprint 03",
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
    foundation: "Development · Sprint 03",
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
function App() {
  const [page, setPage] = useState<"projects" | "runtimes">("projects");
  const [lang, setLang] = useState<"tr" | "en">("tr");
  const t = words[lang];
  const desktop = "__TAURI_INTERNALS__" in window;
  const cache = useQueryClient();
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState("");
  const [importing, setImporting] = useState(false);
  const [wizard, setWizard] = useState(false);
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
  const runtimes = useQuery({
    queryKey: ["runtimes"],
    queryFn: () =>
      request<{ id: string; runtime: string; version: string }[]>(
        "runtime_installed",
      ),
    enabled: connected,
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
        for (let attempt = 0; attempt < 1800; attempt++) {
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
  const providers = new Set(
    items.flatMap((project) => [
      project.manifest.provider || "native",
      ...(project.manifest.services || []).map(
        (service) => service.provider || "native",
      ),
    ]),
  );
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
        <button
          className={page === "projects" ? "nav-current" : "nav-link"}
          onClick={() => setPage("projects")}
        >
          <FolderGit2 size={18} />
          {t.projects}
          <span>{projects.data ? items.length : "—"}</span>
        </button>
        <button
          className={page === "runtimes" ? "nav-current" : "nav-link"}
          onClick={() => setPage("runtimes")}
        >
          <Box size={18} />
          {lang === "tr" ? "Çalışma ortamları" : "Runtimes"}
        </button>
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
            WORKSPACE <span className="slash">/</span>{" "}
            {page === "projects"
              ? t.projects
              : lang === "tr"
                ? "Çalışma ortamları"
                : "Runtimes"}
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
          {page === "runtimes" ? (
            <RuntimePage connected={connected} lang={lang} />
          ) : (
            <>
              <section className="page-heading">
                <div>
                  <div className="eyebrow">BUILD SOMETHING GREAT</div>
                  <h1>{t.projects}</h1>
                  <p>{t.subtitle}</p>
                </div>
                <div className="heading-actions">
                  <Button
                    variant="primary"
                    disabled={
                      !connected || projects.isError || action.isPending
                    }
                    onClick={() => setWizard(true)}
                  >
                    <Plus size={17} />
                    {lang === "tr" ? "Yeni / içe aktar" : "New / import"}
                  </Button>
                  <Button
                    disabled={
                      !connected || projects.isError || action.isPending
                    }
                    onClick={() => setImporting(true)}
                  >
                    <Plus size={17} />
                    {lang === "tr" ? "Manifest içe aktar" : "Import manifest"}
                  </Button>
                </div>
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
                  <strong className="stat-word">
                    {!connected || projects.isError
                      ? "—"
                      : providers.size > 1
                        ? "Hybrid"
                        : providers.has("container")
                          ? "Container"
                          : t.native}
                  </strong>
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
                        className={
                          "status status-" + project.state.toLowerCase()
                        }
                      >
                        <Circle size={6} fill="currentColor" />
                        {project.state}
                      </span>
                    </div>
                    <h2>{project.manifest.name}</h2>
                    <p className="project-path" title={project.manifest.root}>
                      {project.manifest.root.replace(/^\\\\\?\\/, "")}
                    </p>
                    <div className="project-meta">
                      {project.manifest.runtime && (
                        <span title={project.manifest.runtime.installation_id}>
                          {(() => {
                            const runtime = runtimes.data?.find(
                              (r) =>
                                r.id ===
                                project.manifest.runtime?.installation_id,
                            );
                            return runtime
                              ? `${runtime.runtime} ${runtime.version}`
                              : lang === "tr"
                                ? "Sürüm kaydı yüklenemedi"
                                : "Runtime record unavailable";
                          })()}
                        </span>
                      )}
                      <span>
                        <Box size={13} />
                        {project.manifest.provider || "native"}
                      </span>
                      <span>
                        {project.container_id
                          ? project.container_id.slice(0, 10)
                          : `PID ${project.pid ?? "—"}`}
                      </span>
                      <span>:{project.port ?? "—"}</span>
                    </div>
                    {!!project.services?.length && (
                      <div className="service-list">
                        {project.services.map((service) => (
                          <div key={service.id}>
                            <span>{service.id}</span>
                            <span>
                              {service.state} · :{service.port}
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                    {project.error && (
                      <p className="project-error">{project.error}</p>
                    )}
                    <div className="card-actions">
                      <Button
                        variant={
                          ["RUNNING", "DEGRADED"].includes(project.state)
                            ? "secondary"
                            : "primary"
                        }
                        disabled={
                          projects.isError ||
                          action.isPending ||
                          project.state === "STARTING" ||
                          project.state === "STOPPING"
                        }
                        onClick={() =>
                          action.mutate({
                            method: ["RUNNING", "DEGRADED"].includes(
                              project.state,
                            )
                              ? "stop"
                              : "start",
                            params: { id: project.manifest.id },
                          })
                        }
                      >
                        {["RUNNING", "DEGRADED"].includes(project.state) ? (
                          <Square size={13} />
                        ) : (
                          <Play size={13} />
                        )}{" "}
                        {["RUNNING", "DEGRADED"].includes(project.state)
                          ? t.stop
                          : t.start}
                      </Button>
                      {project.state === "FAILED" && (
                        <Button
                          disabled={action.isPending || projects.isError}
                          onClick={() =>
                            action.mutate({
                              method: "stop",
                              params: { id: project.manifest.id },
                            })
                          }
                        >
                          <Square size={13} />
                          {t.stop}
                        </Button>
                      )}
                      {project.port &&
                        project.state === "RUNNING" &&
                        project.manifest.health?.type !== "tcp" && (
                          <a
                            className="open-link"
                            href={`http://127.0.0.1:${project.port}`}
                            target="_blank"
                            rel="noreferrer"
                            aria-label={t.open}
                            onClick={(event) => {
                              if (desktop) {
                                event.preventDefault();
                                invoke("open_project", {
                                  id: project.manifest.id,
                                }).catch((error) => setError(String(error)));
                              }
                            }}
                          >
                            <ArrowUpRight size={18} />
                          </a>
                        )}
                    </div>
                  </article>
                ))}
                {connected && projects.isSuccess && !items.length && (
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
                  {!visible.length && (
                    <div className="log-empty">{t.noEvents}</div>
                  )}
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
            </>
          )}
        </div>
        <footer>
          <span>
            <Circle size={6} fill="currentColor" /> DOCKON LOCAL ENGINE
          </span>
          <span>v0.1.0 · {t.foundation}</span>
        </footer>
      </main>
      {wizard && <ProjectWizard lang={lang} onClose={() => setWizard(false)} />}
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
