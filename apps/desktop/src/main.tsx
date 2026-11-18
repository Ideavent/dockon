import React, { useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  QueryClient,
  QueryClientProvider,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  Activity,
  ArrowUpRight,
  Boxes,
  FolderKanban,
  FolderOpen,
  LoaderCircle,
  Menu,
  MoreHorizontal,
  Network,
  Play,
  Plus,
  RefreshCw,
  ScrollText,
  Settings2,
  Square,
} from "lucide-react";
import { Button } from "./components/ui/button";
import {
  request,
  waitOperation,
  type Operation,
  type Reply,
} from "./ipc/client";
import { RuntimePage } from "./features/runtimes/RuntimePage";
import { ProjectWizard } from "./features/projects/ProjectWizard";
import { RouterPage } from "./features/routing/RouterPage";
import { DiagnosticsPage } from "./features/diagnostics/DiagnosticsPage";
import { ProjectSettings } from "./features/projects/ProjectSettings";
import { CloseDialog } from "./features/settings/CloseDialog";
import { SystemSettings } from "./features/settings/SystemSettings";
import "./styles.css";

type Folder = {
  name: string;
  path: string;
  static_site?: boolean;
};
type Project = {
  manifest: {
    id: string;
    name: string;
    root: string;
    provider?: "native" | "container";
    runtime?: { installation_id: string };
    health?: { type: "http" | "tcp" };
    services?: {
      id: string;
      runtime?: { installation_id: string };
      container?: { image: string };
    }[];
  };
  state: string;
  port: number | null;
  error: string | null;
  services?: { id: string; state: string; port: number; smtp_port?: number }[];
};
const databaseNames = { postgres: "PostgreSQL", mysql: "MySQL" };
type DatabaseEngine = keyof typeof databaseNames;
type DatabaseTarget = { id: string; engine: DatabaseEngine };
type DatabaseConnection = {
  host: string;
  port: number;
  user: string;
  database: string;
  password: string;
};

function getDatabaseConnection(target: DatabaseTarget) {
  return request<DatabaseConnection>("database_connection", target);
}

function databaseUrl(engine: DatabaseEngine, connection: DatabaseConnection) {
  const encode = encodeURIComponent;
  return `${engine === "postgres" ? "postgresql" : "mysql"}://${encode(connection.user)}:${encode(connection.password)}@${connection.host}:${connection.port}/${encode(connection.database)}`;
}

type Event = { seq: number; project_id: string; kind: string; message: string };
const pageIcons = {
  home: FolderKanban,
  logs: ScrollText,
  routing: Network,
  settings: Settings2,
  diagnostics: Activity,
};
type Page = keyof typeof pageIcons;
const desktop = "__TAURI_INTERNALS__" in window;
const motionKey = "dockon.reduce-motion";
const normalizePath = (path: string) =>
  path
    .replace(/^\\\\\?\\/, "")
    .replaceAll("\\", "/")
    .replace(/\/$/, "")
    .toLowerCase();
const active = (project: Project) =>
  ["RUNNING", "DEGRADED"].includes(project.state);

function ActionMenu({
  label,
  icon,
  children,
}: {
  label: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  const menu = useRef<HTMLDetailsElement>(null);
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    const closeOutside = (event: PointerEvent) => {
      if (!menu.current?.contains(event.target as Node) && menu.current)
        menu.current.open = false;
    };
    document.addEventListener("pointerdown", closeOutside);
    return () => document.removeEventListener("pointerdown", closeOutside);
  }, []);
  return (
    <details
      ref={menu}
      className="dock-menu"
      onToggle={(event) => setExpanded(event.currentTarget.open)}
      onKeyDown={(event) => {
        if (event.key === "Escape" && menu.current?.open) {
          menu.current.open = false;
          menu.current.querySelector("summary")?.focus();
        }
      }}
    >
      <summary
        className="dock-control"
        role="button"
        aria-expanded={expanded}
        aria-label={label}
        title={label}
      >
        {icon}
      </summary>
      <div
        className="dock-menu-items"
        onClick={(event) => {
          if ((event.target as Element).closest("button") && menu.current) {
            menu.current.open = false;
            menu.current.querySelector("summary")?.focus();
          }
        }}
      >
        {children}
      </div>
    </details>
  );
}

function App() {
  const [page, setPage] = useState<Page>("home");
  const [settingsTab, setSettingsTab] = useState<"system" | "runtimes">(
    "system",
  );
  const [closeRequested, setCloseRequested] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [reduceMotion, setReduceMotion] = useState(
    () => localStorage.getItem(motionKey) === "true",
  );
  const [lang, setLang] = useState<"tr" | "en">("tr");
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [page, settingsTab]);
  const tr = lang === "tr";
  const t = {
    open: tr ? "Tarayıcıda aç" : "Open in browser",
    start: tr ? "Başlat" : "Start",
    stop: tr ? "Durdur" : "Stop",
    import: tr ? "Manifest içe aktar" : "Import manifest",
    cancel: tr ? "Vazgeç" : "Cancel",
    manifest: tr
      ? "Manifest dosyasının tam yolu"
      : "Absolute path to manifest file",
  };
  const pages = {
    home: tr ? "Projeler" : "Projects",
    settings: tr ? "Ayarlar" : "Settings",
    routing: tr ? "Yerel adresler" : "Local addresses",
    diagnostics: tr ? "Tanılama" : "Diagnostics",
    logs: tr ? "Günlükler" : "Logs",
  };
  const cache = useQueryClient();
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState("");
  const [importing, setImporting] = useState(false);
  const [wizard, setWizard] = useState(false);
  const [folder, setFolder] = useState<Folder>();
  const [settingsProject, setSettingsProject] = useState<Project>();
  const [clipboardNotice, setClipboardNotice] = useState("");
  useEffect(() => {
    if (!clipboardNotice) return;
    const timer = setTimeout(() => setClipboardNotice(""), 3000);
    return () => clearTimeout(timer);
  }, [clipboardNotice]);
  const [restoreBackup, setRestoreBackup] = useState("");
  const [path, setPath] = useState("");
  const [events, setEvents] = useState<Event[]>([]);
  const [selected, setSelected] = useState("");
  const refresh = () => {
    for (const key of ["projects", "workspace-folders"]) {
      void cache.invalidateQueries({ queryKey: [key] });
    }
  };
  const connect = useMutation({
    mutationFn: () => invoke<Reply<unknown>>("connect"),
    onSuccess: (reply) => {
      setConnected(reply.ok);
      setError(reply.error || "");
      refresh();
    },
    onError: (error) => {
      setConnected(false);
      setError(String(error));
    },
  });
  useEffect(() => {
    if (desktop) connect.mutate();
  }, []);
  useEffect(() => {
    if (!desktop) return;
    const listeners = [
      listen<string>("desktop-error", (event) => setError(event.payload)),
      listen("desktop-close-requested", () => setCloseRequested(true)),
      listen<boolean>("desktop-quitting", (event) =>
        setQuitting(event.payload),
      ),
    ];
    for (const listener of listeners) {
      void listener.catch((error) => setError(String(error)));
    }
    return () => {
      for (const listener of listeners) {
        void listener.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, []);
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => request<Project[]>("list"),
    enabled: connected,
    refetchInterval: 1500,
    retry: false,
  });
  const online = connected && !projects.isError;
  const workspace = useQuery({
    queryKey: ["workspace-folders"],
    queryFn: () =>
      request<{ root: string; folders: Folder[] }>("workspace_folders"),
    enabled: online,
    refetchInterval: 3000,
  });
  useEffect(() => {
    if (!connected || page !== "logs") return;
    let epoch = "";
    const channel = new Channel<{ epoch: string; events: Event[] }>();
    channel.onmessage = (batch) => {
      const changed = epoch !== batch.epoch;
      epoch = batch.epoch;
      setEvents((previous) => {
        const kept = changed ? [] : previous;
        const after = kept.at(-1)?.seq ?? 0;
        return [
          ...kept,
          ...batch.events.filter((event) => event.seq > after),
        ].slice(-300);
      });
    };
    void invoke("watch_events", { channel }).catch((error) =>
      setError(String(error)),
    );
    return () => {
      void invoke("stop_events").catch(() => {});
    };
  }, [connected, page]);
  const action = useMutation({
    mutationFn: async ({
      method,
      params,
    }: {
      method: string;
      params: object;
    }) => {
      const reply = await request<Operation>(method, params);
      const outcome = ["start", "stop", "postgres_restore"].includes(method)
        ? waitOperation(reply.id)
        : reply;
      if (method === "start" && "id" in params) {
        await outcome;
        await invoke("prepare_project_address", { id: params.id });
      }
      return outcome;
    },
    onMutate: () => setError(""),
    onSuccess: () => {
      setImporting(false);
    },
    onError: (error) => setError(String(error)),
    onSettled: refresh,
  });
  const connectionDetails = useMutation({
    mutationFn: getDatabaseConnection,
    onMutate: () => {
      setError("");
      setRestoreBackup("");
    },
    onError: (error) => setError(String(error)),
  });
  const copyConnection = useMutation({
    mutationFn: async (target: DatabaseTarget) => {
      const connection = await getDatabaseConnection(target);
      await navigator.clipboard.writeText(
        databaseUrl(target.engine, connection),
      );
    },
    onMutate: () => {
      setError("");
      setClipboardNotice("");
    },
    onSuccess: () =>
      setClipboardNotice(
        tr
          ? "Bağlantı adresi panoya kopyalandı."
          : "Connection URL copied to clipboard.",
      ),
    onError: (error) => setError(String(error)),
  });
  const postgresBackup = useMutation({
    mutationFn: (id: string) => request<string>("postgres_backup", { id }),
    onSuccess: () =>
      cache.invalidateQueries({ queryKey: ["postgres-backups"] }),
    onError: (error) => setError(String(error)),
  });
  const postgresBackups = useQuery({
    queryKey: ["postgres-backups", connectionDetails.variables?.id],
    queryFn: () =>
      request<string[]>("postgres_backups", {
        id: connectionDetails.variables?.id,
      }),
    enabled:
      connectionDetails.isSuccess &&
      connectionDetails.variables?.engine === "postgres",
  });
  const items = projects.data ?? [];
  const registered = new Set(
    items.map((entry) => normalizePath(entry.manifest.root)),
  );
  const folders =
    workspace.data?.folders.filter(
      (entry) => !registered.has(normalizePath(entry.path)),
    ) ?? [];
  const failure = error || projects.error?.message || workspace.error?.message;
  const launchTool = (command: string, params?: Record<string, unknown>) => {
    setError("");
    void invoke(command, params).catch((error) => setError(String(error)));
  };
  const openProject = (id: string, inbox?: boolean) => {
    setError("");
    void invoke("open_project", { id, inbox }).catch((error) =>
      setError(String(error)),
    );
  };
  const setupProject = (entry?: Folder) => {
    setFolder(entry);
    setWizard(true);
  };
  const changeProject = (project: Project, method: "start" | "stop") =>
    action.mutate({ method, params: { id: project.manifest.id } });
  const labels: Record<string, string> = tr
    ? {
        RUNNING: "Çalışıyor",
        STOPPED: "Durdu",
        FAILED: "Hata",
        STARTING: "Başlatılıyor",
        STOPPING: "Durduruluyor",
        DEGRADED: "Sorunlu",
      }
    : {
        RUNNING: "Running",
        STOPPED: "Stopped",
        FAILED: "Failed",
        STARTING: "Starting",
        STOPPING: "Stopping",
        DEGRADED: "Degraded",
      };
  return (
    <div
      className="dock-shell"
      data-motion={reduceMotion ? "reduced" : undefined}
    >
      <header className="dock-header">
        <img
          className="dock-brand-product"
          src="/dockon-logo.svg"
          alt="Dockon"
        />
        <div className="dock-header-actions">
          {online && (
            <span className="dock-connection" role="status">
              <span className="dock-dot dock-dot-active" aria-hidden="true" />
              {tr ? "Bağlı" : "Connected"}
            </span>
          )}
          <ActionMenu label={tr ? "Menü" : "Menu"} icon={<Menu size={19} />}>
            <button
              disabled={!online}
              onClick={() => launchTool("open_workspace")}
            >
              <FolderOpen size={15} aria-hidden="true" />
              {tr ? "www klasörünü aç" : "Open www folder"}
            </button>
            <button disabled={!online} onClick={() => setImporting(true)}>
              {t.import}
            </button>
            <hr />
            <button onClick={() => setLang(tr ? "en" : "tr")}>
              {tr ? "English" : "Türkçe"}
            </button>
          </ActionMenu>
        </div>
      </header>
      <nav
        className="dock-nav"
        aria-label={tr ? "Ana gezinme" : "Main navigation"}
      >
        {(Object.keys(pageIcons) as Page[]).map((item) => {
          const Icon = pageIcons[item];
          return (
            <button
              key={item}
              type="button"
              aria-current={page === item ? "page" : undefined}
              onClick={() => {
                if (item === "logs") setSelected("");
                if (item === "settings" && page !== "settings")
                  setSettingsTab("system");
                setPage(item);
              }}
            >
              <Icon size={16} aria-hidden="true" />
              <span>{pages[item]}</span>
            </button>
          );
        })}
      </nav>
      <main className="dock-main" key={page}>
        {failure && (
          <div className="error dock-error" role="alert">
            {failure}
            <button
              className="dock-control"
              onClick={() => setError("")}
              hidden={!error}
              aria-label={tr ? "Kapat" : "Dismiss"}
            >
              ×
            </button>
          </div>
        )}
        {page === "settings" && (
          <>
            <div
              className="settings-tabs"
              role="group"
              aria-label={tr ? "Ayar kategorileri" : "Settings categories"}
            >
              <button
                type="button"
                aria-pressed={settingsTab === "system"}
                onClick={() => setSettingsTab("system")}
              >
                <Settings2 size={16} aria-hidden="true" />
                {tr ? "Sistem" : "System"}
              </button>
              <button
                type="button"
                aria-pressed={settingsTab === "runtimes"}
                onClick={() => setSettingsTab("runtimes")}
              >
                <Boxes size={16} aria-hidden="true" />
                {tr ? "Sürümler ve servisler" : "Versions and services"}
              </button>
            </div>
            <div className="settings-content" key={settingsTab}>
              {settingsTab === "system" ? (
                <SystemSettings
                  desktop={desktop}
                  lang={lang}
                  reduceMotion={reduceMotion}
                  onReduceMotion={(value) => {
                    localStorage.setItem(motionKey, String(value));
                    setReduceMotion(value);
                  }}
                />
              ) : (
                <RuntimePage connected={online} lang={lang} />
              )}
            </div>
          </>
        )}
        {page === "routing" && <RouterPage connected={online} lang={lang} />}
        {page === "diagnostics" && (
          <DiagnosticsPage connected={online} lang={lang} />
        )}
        {page === "logs" && (
          <section className="logs">
            <div className="log-header">
              <h1>{tr ? "Proje günlükleri" : "Project logs"}</h1>
              <select
                aria-label={tr ? "Proje filtresi" : "Project filter"}
                value={selected}
                onChange={(event) => setSelected(event.target.value)}
              >
                <option value="">{tr ? "Tüm projeler" : "All projects"}</option>
                {items.map((project) => (
                  <option key={project.manifest.id} value={project.manifest.id}>
                    {project.manifest.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="log-body" role="log">
              {events
                .filter((event) => !selected || event.project_id === selected)
                .map((event) => (
                  <div
                    className={
                      "log-line " +
                      (["error", "stderr"].includes(event.kind)
                        ? "log-error"
                        : "")
                    }
                    key={event.seq}
                  >
                    <span>{event.seq}</span>
                    <span>{event.project_id}</span>
                    <code>{event.message}</code>
                  </div>
                ))}
              {!events.some(
                (event) => !selected || event.project_id === selected,
              ) && (
                <p className="log-empty">
                  {tr ? "Henüz günlük kaydı yok." : "No log entries yet."}
                </p>
              )}
            </div>
          </section>
        )}
        {page === "home" && (
          <>
            {!online && (
              <section
                className="dock-stack"
                aria-label={tr ? "Bağlantı" : "Connection"}
              >
                <div className="dock-stack-state" role="status">
                  <span
                    className={
                      connect.isPending
                        ? "dock-dot dock-dot-pending"
                        : "dock-dot"
                    }
                    aria-hidden="true"
                  />
                  <strong>
                    {!desktop
                      ? tr
                        ? "Masaüstü bağlantısı gerekli"
                        : "Desktop connection required"
                      : connect.isPending
                        ? tr
                          ? "Bağlanıyor…"
                          : "Connecting…"
                        : tr
                          ? "Bağlantı kesildi"
                          : "Disconnected"}
                  </strong>
                </div>
                {desktop && (
                  <Button
                    disabled={connect.isPending}
                    onClick={() => connect.mutate()}
                  >
                    <RefreshCw
                      className={connect.isPending ? "dock-spin" : undefined}
                      size={15}
                      aria-hidden="true"
                    />
                    {tr ? "Yeniden bağlan" : "Reconnect"}
                  </Button>
                )}
              </section>
            )}
            <section aria-labelledby="projects-heading">
              <div className="dock-section-title">
                <h1 id="projects-heading">{pages.home}</h1>
                <Button
                  variant="primary"
                  disabled={!online || !workspace.data}
                  onClick={() => setupProject()}
                >
                  <Plus size={17} aria-hidden="true" />
                  {tr ? "Yeni proje" : "New project"}
                </Button>
              </div>
              <ul className="dock-projects">
                {items.map((project) => {
                  const id = project.manifest.id;
                  const engines = (
                    Object.keys(databaseNames) as DatabaseEngine[]
                  ).filter((engine) =>
                    project.manifest.services?.some(
                      (service) => service.id === engine,
                    ),
                  );
                  const canOpen =
                    project.state === "RUNNING" &&
                    project.port &&
                    project.manifest.health?.type !== "tcp";
                  const requestedAction =
                    action.isPending &&
                    "id" in action.variables.params &&
                    action.variables.params.id === id
                      ? action.variables.method
                      : undefined;
                  const pendingLabel =
                    requestedAction === "start" || project.state === "STARTING"
                      ? labels.STARTING
                      : requestedAction === "stop" ||
                          project.state === "STOPPING"
                        ? labels.STOPPING
                        : undefined;
                  const toggleLabel =
                    pendingLabel ?? (active(project) ? t.stop : t.start);
                  return (
                    <li className="dock-project" key={id}>
                      <span
                        className={
                          "dock-dot " +
                          (pendingLabel
                            ? "dock-dot-pending"
                            : active(project)
                              ? "dock-dot-active"
                              : project.error
                                ? "dock-dot-error"
                                : "")
                        }
                      />
                      <div className="dock-project-name">
                        <strong title={project.manifest.root}>
                          {project.manifest.name}
                        </strong>
                        <span
                          className={project.error ? "dock-project-error" : ""}
                          title={project.error ?? undefined}
                        >
                          {project.error ||
                            pendingLabel ||
                            labels[project.state] ||
                            project.state}
                        </span>
                      </div>
                      {canOpen && (
                        <button
                          className="dock-control"
                          disabled={!online}
                          title={t.open}
                          aria-label={t.open}
                          onClick={() => openProject(id)}
                        >
                          <ArrowUpRight size={17} />
                        </button>
                      )}
                      <button
                        className="dock-control dock-project-toggle"
                        disabled={!online || !!pendingLabel || action.isPending}
                        title={toggleLabel}
                        aria-label={toggleLabel}
                        onClick={() =>
                          changeProject(
                            project,
                            active(project) ? "stop" : "start",
                          )
                        }
                      >
                        {pendingLabel ? (
                          <LoaderCircle
                            className="dock-spin"
                            size={15}
                            aria-hidden="true"
                          />
                        ) : active(project) ? (
                          <Square size={14} />
                        ) : (
                          <Play size={15} />
                        )}
                        <span>{toggleLabel}</span>
                      </button>
                      <ActionMenu
                        label={tr ? "Proje işlemleri" : "Project actions"}
                        icon={<MoreHorizontal size={18} />}
                      >
                        <button
                          disabled={!online}
                          onClick={() => {
                            setSettingsProject(project);
                          }}
                        >
                          {tr ? "Proje ayarları" : "Project settings"}
                        </button>
                        <button
                          disabled={!online}
                          onClick={() => launchTool("project_terminal", { id })}
                        >
                          {tr ? "Proje terminali" : "Project terminal"}
                        </button>
                        <button
                          onClick={() => {
                            setSelected(id);
                            setPage("logs");
                          }}
                        >
                          {pages.logs}
                        </button>
                        {engines.length > 0 && <hr />}
                        {engines.map((engine) => {
                          const ready = project.services?.some(
                            (service) =>
                              service.id === engine &&
                              service.state === "RUNNING",
                          );
                          return (
                            <button
                              key={engine}
                              type="button"
                              disabled={
                                !online || !ready || connectionDetails.isPending
                              }
                              onClick={() =>
                                connectionDetails.mutate({ id, engine })
                              }
                            >
                              {databaseNames[engine]} ·{" "}
                              {tr ? "Bağlantı bilgileri" : "Connection details"}
                            </button>
                          );
                        })}
                        {project.state === "FAILED" && (
                          <button
                            disabled={action.isPending}
                            onClick={() => changeProject(project, "stop")}
                          >
                            {t.stop}
                          </button>
                        )}
                        {project.services?.some(
                          (service) =>
                            service.id === "mailpit" &&
                            service.state === "RUNNING" &&
                            service.smtp_port,
                        ) && (
                          <button onClick={() => openProject(id, true)}>
                            {tr ? "Proje gelen kutusu" : "Project inbox"}
                          </button>
                        )}
                      </ActionMenu>
                    </li>
                  );
                })}
                {folders.map((entry) => (
                  <li className="dock-project" key={entry.path}>
                    <span className="dock-dot" />
                    <div className="dock-project-name">
                      <strong title={entry.path}>{entry.name}</strong>
                      <span>
                        {entry.static_site
                          ? tr
                            ? "Statik site"
                            : "Static site"
                          : tr
                            ? "www klasörü"
                            : "www folder"}
                      </span>
                    </div>
                    <button
                      className="dock-control"
                      disabled={!online}
                      onClick={() => setupProject(entry)}
                    >
                      {tr ? "Ayarla" : "Configure"}
                    </button>
                  </li>
                ))}
              </ul>
              {workspace.isSuccess &&
                projects.isSuccess &&
                !folders.length &&
                !items.length && (
                  <p className="dock-empty">
                    {tr
                      ? "Yeni proje oluştur veya mevcut bir klasörü içe aktar."
                      : "Create a project or import an existing folder."}
                  </p>
                )}
            </section>
          </>
        )}
      </main>
      <footer className="dock-footer">
        <span lang="en">Made by</span>
        <span className="dock-brand-maker">
          <img src="/ideavent-logo.png" alt="Ideavent" />
        </span>
      </footer>
      {wizard && (
        <ProjectWizard
          lang={lang}
          folder={folder}
          workspaceRoot={workspace.data?.root}
          onClose={() => setWizard(false)}
        />
      )}
      {settingsProject &&
        (() => {
          const project =
            items.find(
              (entry) => entry.manifest.id === settingsProject.manifest.id,
            ) ?? settingsProject;
          return (
            <ProjectSettings
              lang={lang}
              project={project}
              projects={items}
              openLogs={() => {
                setSelected(project.manifest.id);
                setSettingsProject(undefined);
                setPage("logs");
              }}
              openWeb={() => openProject(project.manifest.id)}
              startWeb={() => changeProject(project, "start")}
              onDownloads={() => {
                setSettingsProject(undefined);
                setSettingsTab("runtimes");
                setPage("settings");
              }}
              openInbox={() => openProject(project.manifest.id, true)}
              openTerminal={() =>
                launchTool("project_terminal", { id: project.manifest.id })
              }
              onClose={() => setSettingsProject(undefined)}
            />
          );
        })()}
      {clipboardNotice && (
        <p className="dock-clipboard-notice" role="status">
          {clipboardNotice}
        </p>
      )}
      {connectionDetails.data && (
        <div className="modal-backdrop">
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-label={
              tr
                ? "Veritabanı bağlantı bilgileri"
                : "Database connection details"
            }
          >
            <h2>
              {connectionDetails.variables &&
                databaseNames[connectionDetails.variables.engine]}
            </h2>
            <p>
              {connectionDetails.data.host}:{connectionDetails.data.port} /{" "}
              {connectionDetails.data.database}
            </p>
            <p>
              {lang === "tr" ? "Kullanıcı" : "User"}{" "}
              {connectionDetails.data.user}
            </p>
            <label htmlFor="database-password">
              {lang === "tr" ? "Parola" : "Password"}
            </label>
            <input
              id="database-password"
              value={connectionDetails.data.password}
              readOnly
            />
            <div className="dialog-actions">
              <Button
                disabled={copyConnection.isPending}
                onClick={() => {
                  if (connectionDetails.variables)
                    copyConnection.mutate(connectionDetails.variables);
                }}
              >
                {tr ? "Bağlantı adresini kopyala" : "Copy connection URL"}
              </Button>
              <Button
                onClick={() => {
                  connectionDetails.reset();
                  setRestoreBackup("");
                }}
              >
                {lang === "tr" ? "Kapat" : "Close"}
              </Button>
              {connectionDetails.variables?.engine === "postgres" && (
                <Button
                  disabled={postgresBackup.isPending}
                  onClick={() =>
                    postgresBackup.mutate(connectionDetails.variables!.id)
                  }
                >
                  {lang === "tr" ? "SQL yedeği oluştur" : "Create SQL backup"}
                </Button>
              )}
            </div>
            {connectionDetails.variables?.engine === "postgres" &&
              !!postgresBackups.data?.length && (
                <>
                  <label htmlFor="postgres-backup">
                    {lang === "tr" ? "SQL yedeği" : "SQL backup"}
                  </label>
                  <select
                    id="postgres-backup"
                    value={restoreBackup}
                    onChange={(event) => setRestoreBackup(event.target.value)}
                  >
                    <option value="">
                      {lang === "tr" ? "Yedek seç" : "Choose backup"}
                    </option>
                    {postgresBackups.data.map((backup) => (
                      <option key={backup}>{backup}</option>
                    ))}
                  </select>
                  <Button
                    disabled={action.isPending || !restoreBackup}
                    onClick={() =>
                      action.mutate({
                        method: "postgres_restore",
                        params: {
                          id: connectionDetails.variables?.id,
                          backup: restoreBackup,
                        },
                      })
                    }
                  >
                    {lang === "tr"
                      ? "Yedeği yeni veritabanına yükle"
                      : "Restore backup to a new database"}
                  </Button>
                  {action.variables?.method === "postgres_restore" &&
                    action.data?.status === "completed" && (
                      <p>dockon_restore_{action.data.id.replaceAll("-", "")}</p>
                    )}
                </>
              )}
            {error && <p role="alert">{error}</p>}
          </section>
        </div>
      )}
      {importing && (
        <div className="modal-backdrop">
          <form
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-label={t.import}
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
            <div className="dialog-actions">
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
      {closeRequested && (
        <CloseDialog
          lang={lang}
          quitting={quitting}
          onDismiss={() => setCloseRequested(false)}
        />
      )}
      {quitting && !closeRequested && (
        <p className="dock-clipboard-notice close-progress" role="status">
          <LoaderCircle className="dock-spin" size={16} aria-hidden="true" />
          {tr ? "Servisler durduruluyor…" : "Stopping services…"}
        </p>
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
