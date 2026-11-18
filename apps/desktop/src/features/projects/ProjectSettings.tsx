import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";
import { SystemPermissions } from "../routing/SystemPermissions";
import type { RouterStatus } from "../routing/RouterPage";
import {
  CacheSettings,
  cacheNames,
  type CacheService,
} from "../runtimes/CacheSettings";
import { InstallVersion } from "../runtimes/InstallVersion";
import { runtimeNames } from "../runtimes/RuntimePage";

import {
  DatabaseSnapshots,
  ProjectCheckPanel,
  ProjectSharePanel,
  ProjectTasksPanel,
  type SnapshotProject,
  type Task,
} from "./ProjectFeatures";

type Installation = {
  id: string;
  runtime: string;
  version: string;
  platform: string;
};
type ProjectTools = {
  composer?: string | null;
  java?: string | null;
  android_sdk?: string | null;
  android_tools?: string | null;
};
type Project = {
  state: string;
  manifest: {
    id: string;
    name: string;
    root: string;
    provider?: "native" | "container";
    health?: { type: string };
    tools?: ProjectTools;
    template?: string | null;
    tasks?: Task[];
    runtime?: { installation_id: string };
    services?: {
      id: string;
      runtime?: { installation_id: string };
      container?: { image: string };
    }[];
  };
  services?: { id: string; state: string; port: number; smtp_port?: number }[];
};
export type ProjectSettingsTab =
  "versions" | CacheService | "database" | "mail" | "tasks" | "check" | "share";
type Tab = ProjectSettingsTab;
const databases = {
  mysql: "MySQL",
  postgres: "PostgreSQL",
};

export function ProjectSettings({
  project,
  lang,
  onClose,
  onDownloads,
  openInbox,
  openTerminal,
  openLogs,
  openWeb,
  startWeb,
  projects,
  initialTab = "versions",
}: {
  project: Project;
  lang: "tr" | "en";
  onClose: () => void;
  onDownloads: () => void;
  openInbox: () => void;
  openTerminal: () => void;
  openLogs: () => void;
  openWeb: () => void;
  startWeb: () => void;
  projects: SnapshotProject[];
  initialTab?: Tab;
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const [tab, setTab] = useState<Tab>(initialTab);
  const { id, name, root } = project.manifest;
  const [sdkDraft, setSdkDraft] = useState<string>();
  const stopped = project.state === "STOPPED";
  const running = project.state === "RUNNING";
  const router = useQuery({
    queryKey: ["router"],
    queryFn: () => request<RouterStatus>("router_status"),
    enabled: tab === "mail",
  });
  const runtimes = useQuery({
    queryKey: ["runtimes"],
    queryFn: () => request<Installation[]>("runtime_installed"),
  });
  const route = router.data?.settings?.routes.find(
    (item) => item.project_id === id,
  );
  const settings = router.data?.settings;
  const folderName =
    root.split(/[\\/]/).filter(Boolean).at(-1)?.toLowerCase() ?? id;
  const domainSuffix = settings?.domain_suffix ?? "test";
  const hostname = route?.domain || `${folderName}.${domainSuffix}`;
  const https = route?.https ?? settings?.default_https ?? false;
  const port = https ? settings?.https_port : settings?.http_port;
  const standardPort = https ? 443 : 80;
  const save = useMutation({
    mutationFn: (https: boolean) =>
      request<RouterStatus>("workspace_route", { id, https }),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["router"] }),
  });
  const component = useMutation({
    mutationFn: async ({
      kind,
      runtime,
    }: {
      kind: "server" | "php" | "runtime";
      runtime: string;
    }) => {
      const params = { id, runtime_installation: runtime };
      if (kind === "server") {
        await waitOperation(
          (await request<Operation>("project_switch_static_server", params)).id,
        );
      } else {
        await request(
          kind === "php" ? "project_php_version" : "project_runtime_version",
          params,
        );
      }
    },
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const attachMail = useMutation({
    mutationFn: (enabled: boolean) =>
      request("project_set_mail", { id, enabled }),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const attachDatabase = useMutation({
    mutationFn: (engine: keyof typeof databases) =>
      request(
        engine === "postgres" ? "project_add_postgres" : "project_add_database",
        { id, ...(engine === "postgres" ? {} : { engine }) },
      ),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const selected = runtimes.data?.find(
    (entry) => entry.id === project.manifest.runtime?.installation_id,
  );
  const tools = project.manifest.tools ?? {};
  const setTools = useMutation({
    mutationFn: (options: ProjectTools) =>
      request("project_tools_set", { id, tools: options }),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const isWebProfile =
    selected?.runtime === "apache" || selected?.runtime === "nginx";
  const phpId = project.manifest.services?.find(
    (service) => service.id === "php-cgi",
  )?.runtime?.installation_id;
  const choices =
    runtimes.data?.filter((entry) =>
      isWebProfile
        ? entry.runtime === "nginx" ||
          (entry.runtime === "apache" && entry.platform.startsWith("windows-"))
        : entry.runtime === selected?.runtime,
    ) ?? [];
  const php =
    runtimes.data?.filter((entry) => entry.runtime === "php-cgi") ?? [];
  const phpProject = selected?.runtime === "php" || Boolean(phpId);
  const language = phpProject
    ? "PHP"
    : selected && runtimeNames[selected.runtime];
  const flutter = selected?.runtime === "flutter";
  const sdkPath = sdkDraft ?? tools.android_sdk ?? "";
  const hasMail = Boolean(
    project.manifest.services?.some((service) => service.id === "mailpit"),
  );
  const mail = project.services?.find(
    (service) => service.id === "mailpit" && service.state === "RUNNING",
  );
  const canRoute = project.manifest.health?.type !== "tcp" || hasMail;
  const mailPort = settings?.http_port;
  const windows = navigator.userAgent.includes("Windows");
  const namedWebmailPort =
    windows && !hostname.endsWith(".localhost") ? 80 : mailPort;
  const namedProjectPort =
    windows && !hostname.endsWith(".localhost") ? standardPort : port;
  const tabs: Record<Tab, string> = {
    versions: language ?? (tr ? "Sürümler" : "Versions"),
    tasks: tr ? "Görevler" : "Tasks",
    check: tr ? "Durum kontrolü" : "Health checks",
    share: tr ? "Ön ayar ve paylaşım" : "Presets and sharing",
    ...cacheNames,
    database: tr ? "Veritabanı" : "Database",
    mail: tr ? "Posta ve adres" : "Mail and address",
  };
  const renderRuntime = (entry: Installation) => (
    <option key={entry.id} value={entry.id}>
      {runtimeNames[entry.runtime] || entry.runtime} {entry.version}
    </option>
  );
  return (
    <div className="modal-backdrop">
      <section
        className="modal project-wizard"
        role="dialog"
        aria-modal="true"
        aria-label={tr ? "Proje ayarları" : "Project settings"}
      >
        <h2>{name}</h2>
        <nav
          className="runtime-tabs"
          aria-label={tr ? "Proje ayarı sekmeleri" : "Project settings tabs"}
        >
          {(Object.entries(tabs) as [Tab, string][]).map(([id, label]) => (
            <button
              key={id}
              type="button"
              aria-pressed={tab === id}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </nav>
        {tab === "tasks" && (
          <ProjectTasksPanel
            id={id}
            tr={tr}
            runtime={selected?.runtime}
            php={phpProject}
            running={running}
            favorites={project.manifest.tasks ?? []}
            openLogs={openLogs}
            openWeb={openWeb}
            startWeb={startWeb}
          />
        )}
        {tab === "check" && (
          <ProjectCheckPanel
            id={id}
            tr={tr}
            onAction={(action) => {
              if (action !== "source") setTab(action);
            }}
          />
        )}
        {tab === "share" && <ProjectSharePanel id={id} tr={tr} name={name} />}
        {tab === "versions" && (
          <>
            {selected && (
              <label>
                {isWebProfile
                  ? tr
                    ? "Web sunucusu"
                    : "Web server"
                  : tr
                    ? `${language ?? selected.runtime} sürümü`
                    : `${language ?? selected.runtime} version`}
                <select
                  value={selected.id}
                  disabled={
                    (!stopped && !(isWebProfile && running)) ||
                    component.isPending
                  }
                  onChange={(event) =>
                    component.mutate({
                      kind: isWebProfile ? "server" : "runtime",
                      runtime: event.target.value,
                    })
                  }
                >
                  {choices.map(renderRuntime)}
                </select>
              </label>
            )}
            {(isWebProfile || phpId) && (
              <label>
                PHP FastCGI
                <select
                  value={phpId ?? ""}
                  disabled={!stopped || component.isPending || !php.length}
                  onChange={(event) =>
                    component.mutate({
                      kind: "php",
                      runtime: event.target.value,
                    })
                  }
                >
                  <option value="" disabled>
                    {tr ? "Sürüm seç" : "Choose a version"}
                  </option>
                  {php.map(renderRuntime)}
                </select>
                {!phpId && (
                  <small>
                    {tr
                      ? "PHP FastCGI için public/index.php gerekir."
                      : "PHP FastCGI needs public/index.php."}
                  </small>
                )}
              </label>
            )}
            {phpProject && (
              <>
                <label>
                  Composer
                  <select
                    value={tools.composer ?? ""}
                    disabled={!stopped || setTools.isPending}
                    onChange={(event) =>
                      setTools.mutate({
                        ...tools,
                        composer: event.target.value || null,
                      })
                    }
                  >
                    <option value="">
                      {tr
                        ? "Composer atanmamış"
                        : "No Composer version selected"}
                    </option>
                    {runtimes.data
                      ?.filter((item) => item.runtime === "composer")
                      .map(renderRuntime)}
                  </select>
                </label>
                <InstallVersion runtime="composer" label="Composer" tr={tr} />
              </>
            )}
            {flutter && (
              <>
                <label>
                  Java JDK
                  <select
                    value={tools.java ?? ""}
                    disabled={!stopped || setTools.isPending}
                    onChange={(event) =>
                      setTools.mutate({
                        ...tools,
                        java: event.target.value || null,
                      })
                    }
                  >
                    <option value="">
                      {tr ? "Sistem JDK seçimi" : "System JDK selection"}
                    </option>
                    {runtimes.data
                      ?.filter((item) => item.runtime === "java")
                      .map(renderRuntime)}
                  </select>
                </label>
                <InstallVersion runtime="java" label="Java JDK" tr={tr} />
                <label>
                  Android SDK Tools
                  <select
                    value={tools.android_tools ?? ""}
                    disabled={!stopped || setTools.isPending}
                    onChange={(event) =>
                      setTools.mutate({
                        ...tools,
                        android_tools: event.target.value || null,
                      })
                    }
                  >
                    <option value="">
                      {tr ? "Mevcut SDK" : "Existing SDK"}
                    </option>
                    {runtimes.data
                      ?.filter((item) => item.runtime === "android-tools")
                      .map(renderRuntime)}
                  </select>
                </label>
                <InstallVersion
                  runtime="android-tools"
                  label="Android SDK Tools"
                  tr={tr}
                />
                {tools.android_tools && (
                  <div className="project-location">
                    <Button type="button" onClick={openTerminal}>
                      {tr
                        ? "Android SDK terminalini aç"
                        : "Open Android SDK terminal"}
                    </Button>
                    <code>sdkmanager --list</code>
                    <code>sdkmanager --licenses</code>
                  </div>
                )}
                <label>
                  {tr ? "Android SDK klasörü" : "Android SDK directory"}
                  <input
                    value={sdkPath}
                    disabled={!stopped || setTools.isPending}
                    onChange={(event) => setSdkDraft(event.target.value)}
                  />
                </label>
                <Button
                  type="button"
                  disabled={
                    !stopped ||
                    setTools.isPending ||
                    sdkPath.trim() === (tools.android_sdk ?? "")
                  }
                  onClick={() =>
                    setTools.mutate(
                      { ...tools, android_sdk: sdkPath.trim() || null },
                      { onSuccess: () => setSdkDraft(undefined) },
                    )
                  }
                >
                  {tr ? "SDK klasörünü kaydet" : "Save SDK directory"}
                </Button>
              </>
            )}
            {selected &&
              !["rust", "apache", "nginx"].includes(selected.runtime) && (
                <InstallVersion
                  key={selected.runtime}
                  runtime={selected.runtime}
                  label={runtimeNames[selected.runtime] ?? selected.runtime}
                  tr={tr}
                />
              )}
            {phpId && (
              <InstallVersion runtime="php-cgi" label="PHP FastCGI" tr={tr} />
            )}
            {setTools.error && <p role="alert">{setTools.error.message}</p>}
            {!selected && !runtimes.isPending && (
              <p>
                {tr
                  ? "Bu projenin çalışma zamanı manifest tarafından belirleniyor."
                  : "This project's runtime is defined by its manifest."}
              </p>
            )}
            {component.isPending && (
              <p role="status">
                {tr ? "Sürüm uygulanıyor…" : "Applying version…"}
              </p>
            )}
            {(component.error || runtimes.error) && (
              <p role="alert">{(component.error || runtimes.error)?.message}</p>
            )}
            <Button onClick={onDownloads}>
              {tr ? "Sürümleri indir" : "Download versions"}
            </Button>
          </>
        )}
        {(tab === "redis" || tab === "memcached") && (
          <CacheSettings
            key={tab}
            service={tab}
            connected
            lang={lang}
            project={project}
          />
        )}
        {tab === "database" && (
          <section className="cache-settings">
            <p>
              {tr
                ? "Veritabanı seçilen proje ile birlikte yerel olarak başlar."
                : "The database runs locally with the selected project."}
            </p>
            {(
              Object.entries(databases) as [keyof typeof databases, string][]
            ).map(([engine, name]) => {
              const assigned = project.manifest.services?.some(
                (entry) => entry.id === engine,
              );
              const status = project.services?.find(
                (entry) => entry.id === engine && entry.state === "RUNNING",
              );
              return (
                <div key={engine}>
                  <div className="heading-actions">
                    <span>
                      {name} {status && <code>127.0.0.1:{status.port}</code>}
                    </span>
                    {assigned ? (
                      <small>{tr ? "Atandı" : "Assigned"}</small>
                    ) : (
                      <Button
                        disabled={!stopped || attachDatabase.isPending}
                        onClick={() => attachDatabase.mutate(engine)}
                      >
                        {tr ? "Projeye ekle" : "Add to project"}
                      </Button>
                    )}
                  </div>
                  {assigned && (
                    <DatabaseSnapshots
                      id={id}
                      engine={engine}
                      running={Boolean(status)}
                      tr={tr}
                      projects={projects}
                    />
                  )}
                </div>
              );
            })}
            {attachDatabase.error && (
              <p role="alert">{attachDatabase.error.message}</p>
            )}
          </section>
        )}
        {tab === "mail" && (
          <>
            {hasMail && (
              <div className="project-location">
                <span>
                  {tr ? "Projeye özel gelen kutusu" : "Project inbox"}
                </span>
                <code>
                  http://webmail.{hostname}
                  {namedWebmailPort && namedWebmailPort !== 80
                    ? `:${namedWebmailPort}`
                    : ""}
                  /
                </code>
                {windows && !hostname.endsWith(".localhost") ? (
                  <>
                    <span>
                      SMTP · <code>mail.{hostname}</code>
                    </span>
                    <span>
                      HTTP API · <code>http://mail.{hostname}/api/v1/</code>
                    </span>
                  </>
                ) : (
                  mail?.smtp_port && (
                    <span>
                      SMTP · <code>127.0.0.1:{mail.smtp_port}</code>
                    </span>
                  )
                )}
              </div>
            )}
            {mail ? (
              <Button onClick={openInbox}>
                {tr ? "Gelen kutusunu aç" : "Open inbox"}
              </Button>
            ) : !hasMail ? (
              <Button
                disabled={!stopped || attachMail.isPending}
                onClick={() => attachMail.mutate(true)}
              >
                {tr ? "Projeye Mailpit ekle" : "Add Mailpit to project"}
              </Button>
            ) : (
              <p>
                {tr
                  ? "Gelen kutusu proje ile birlikte açılır."
                  : "The inbox starts with the project."}
              </p>
            )}
            {hasMail && stopped && (
              <Button
                disabled={attachMail.isPending}
                onClick={() => attachMail.mutate(false)}
              >
                {tr ? "Proje postasını kapat" : "Disable project mail"}
              </Button>
            )}
            {attachMail.error && <p role="alert">{attachMail.error.message}</p>}
            {canRoute && (
              <>
                <div className="project-location">
                  <span>{tr ? "Yerel adres" : "Local address"}</span>
                  <code>
                    {route && namedProjectPort
                      ? `${https ? "https" : "http"}://${hostname}${namedProjectPort === standardPort ? "" : `:${namedProjectPort}`}`
                      : hostname}
                  </code>
                </div>
                <label className="project-ssl">
                  <input
                    type="checkbox"
                    checked={https}
                    disabled={save.isPending || router.isPending}
                    onChange={(event) => save.mutate(event.target.checked)}
                  />
                  {tr ? "SSL kullan (HTTPS)" : "Use SSL (HTTPS)"}
                </label>
                {route?.https && (
                  <p role="status">
                    {router.data?.certificate_trusted
                      ? tr
                        ? "Dockon sertifikası Windows’ta güvenilir."
                        : "Windows trusts the Dockon certificate."
                      : tr
                        ? "Tarayıcı uyarısını kaldırmak için aşağıdan Dockon sertifikasına güven."
                        : "Trust the Dockon certificate below to clear the browser warning."}
                  </p>
                )}
                {!router.data?.running && (
                  <Button
                    variant="secondary"
                    disabled={save.isPending || router.isPending}
                    onClick={() => save.mutate(https)}
                  >
                    {tr ? "Yerel erişimi başlat" : "Start local access"}
                  </Button>
                )}
                {save.error && <p role="alert">{save.error.message}</p>}
                {router.error && <p role="alert">{router.error.message}</p>}
                {route && (
                  <SystemPermissions
                    lang={lang}
                    hostsPending={router.data?.hosts_pending ?? false}
                    trustPending={
                      https &&
                      Boolean(router.data?.root_certificate) &&
                      !router.data?.certificate_trusted
                    }
                  />
                )}
              </>
            )}
          </>
        )}
        <div className="dialog-actions">
          <Button type="button" onClick={onClose}>
            {tr ? "Kapat" : "Close"}
          </Button>
        </div>
      </section>
    </div>
  );
}
