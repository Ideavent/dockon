import { useEffect, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request } from "../../ipc/client";
import { SystemPermissions } from "./SystemPermissions";
type Route = { project_id: string; domain: string; https: boolean };
type Settings = {
  runtime_installation: string;
  http_port: number;
  https_port: number;
  routes: Route[];
};
type Status = {
  settings: Settings | null;
  running: boolean;
  pid: number | null;
  error: string | null;
  root_certificate: string | null;
  logs: string[];
};
type Plan = {
  token: string;
  before: string;
  after: string;
  warnings: string[];
  hosts_entries: string[];
};
export function RouterPage({
  connected,
  lang,
}: {
  connected: boolean;
  lang: "tr" | "en";
}) {
  const tr = lang === "tr",
    cache = useQueryClient();
  const status = useQuery({
    queryKey: ["router"],
    queryFn: () => request<Status>("router_status"),
    enabled: connected,
    refetchInterval: 2000,
    retry: false,
  });
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () =>
      request<
        {
          manifest: { id: string; name: string; health?: { type: string } };
          state: string;
        }[]
      >("list"),
    enabled: connected,
  });
  const installed = useQuery({
    queryKey: ["runtimes"],
    queryFn: () =>
      request<{ id: string; runtime: string; version: string }[]>(
        "runtime_installed",
      ),
    enabled: connected,
  });
  const [settings, setSettings] = useState<Settings>({
    runtime_installation: "",
    http_port: navigator.userAgent.includes("Windows") ? 80 : 8080,
    https_port: navigator.userAgent.includes("Windows") ? 443 : 8443,
    routes: [],
  });
  const [loaded, setLoaded] = useState(false),
    [plan, setPlan] = useState<Plan | null>(null);
  useEffect(() => {
    if (status.data && !loaded) {
      if (status.data.settings) setSettings(status.data.settings);
      setLoaded(true);
    }
  }, [status.data, loaded]);
  const preview = useMutation({
    mutationFn: () => request<Plan>("router_plan", { settings }),
    onSuccess: setPlan,
  });
  const apply = useMutation({
    mutationFn: () => request("router_apply", { token: plan?.token }),
    onSuccess: () => {
      setPlan(null);
      cache.invalidateQueries({ queryKey: ["router"] });
    },
  });
  const stop = useMutation({
    mutationFn: () => request("router_stop"),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["router"] }),
  });
  const change = (value: Settings) => {
    setSettings(value);
    setPlan(null);
    preview.reset();
    apply.reset();
  };
  const busy = preview.isPending || apply.isPending || stop.isPending;
  const candidates = (projects.data || []).filter(
    (p) => !p.manifest.health || p.manifest.health.type === "http",
  );
  return (
    <>
      <section className="page-heading">
        <div>
          <div className="eyebrow">LOCAL WEB ROUTER</div>
          <h1>{tr ? "Yerel adresler" : "Local domains"}</h1>
          <p>
            {tr
              ? "Projelerine özel alan adı ve yerel HTTPS."
              : "Custom project domains and local HTTPS."}
          </p>
        </div>
        <span
          className={
            "status status-" + (status.data?.running ? "running" : "stopped")
          }
        >
          {status.data?.running
            ? tr
              ? "Yönlendirici çalışıyor"
              : "Router running"
            : status.isSuccess
              ? tr
                ? "Yönlendirici durduruldu"
                : "Router stopped"
              : "—"}
        </span>
      </section>
      {!connected && (
        <div className="notice">
          {tr
            ? "Önce masaüstü daemon bağlantısını kur."
            : "Connect to the desktop daemon first."}
        </div>
      )}
      {[
        status.error,
        projects.error,
        installed.error,
        preview.error,
        apply.error,
        stop.error,
      ]
        .filter(Boolean)
        .map((e, i) => (
          <div className="error" role="alert" key={i}>
            {String(e)}
          </div>
        ))}
      {status.data?.error && (
        <div className="error" role="alert">
          {status.data.error}
        </div>
      )}
      <form
        className="router-form"
        onSubmit={(e) => {
          e.preventDefault();
          preview.mutate();
        }}
      >
        <div className="router-fields">
          <label>
            {tr ? "Caddy sürümü" : "Caddy version"}
            <select
              value={settings.runtime_installation}
              required
              disabled={busy}
              onChange={(e) =>
                change({ ...settings, runtime_installation: e.target.value })
              }
            >
              <option value="">
                {tr ? "Kurulu sürüm seç" : "Select an installed version"}
              </option>
              {installed.data
                ?.filter((r) => r.runtime === "caddy")
                .map((r) => (
                  <option key={r.id} value={r.id}>
                    Caddy {r.version}
                  </option>
                ))}
            </select>
          </label>
          <label>
            HTTP {tr ? "portu" : "port"}
            <input
              type="number"
              min={80}
              max={65535}
              value={settings.http_port}
              disabled={busy}
              onChange={(e) =>
                change({ ...settings, http_port: Number(e.target.value) })
              }
              required
            />
          </label>
          <label>
            HTTPS {tr ? "portu" : "port"}
            <input
              type="number"
              min={443}
              max={65535}
              value={settings.https_port}
              disabled={busy}
              onChange={(e) =>
                change({ ...settings, https_port: Number(e.target.value) })
              }
              required
            />
          </label>
        </div>
        <p className="muted">
          {tr
            ? "Varsayılan uzantı .test. Tam alan adını her proje için değiştirebilirsin. 1024 ve üzeri portlar yönetici izni gerektirmez."
            : "The default suffix is .test. Edit the full domain for each project. Ports 1024 and above do not require administrator privileges."}
        </p>
        <div className="route-list">
          {candidates.map((project) => {
            const route = settings.routes.find(
              (r) => r.project_id === project.manifest.id,
            );
            return (
              <div className="route-row" key={project.manifest.id}>
                <label className="route-toggle">
                  <input
                    type="checkbox"
                    checked={!!route}
                    disabled={busy}
                    onChange={(e) =>
                      change({
                        ...settings,
                        routes: e.target.checked
                          ? [
                              ...settings.routes,
                              {
                                project_id: project.manifest.id,
                                domain:
                                  project.manifest.id
                                    .toLowerCase()
                                    .replace(/_/g, "-") + ".test",
                                https: false,
                              },
                            ]
                          : settings.routes.filter(
                              (r) => r.project_id !== project.manifest.id,
                            ),
                      })
                    }
                  />
                  <span>
                    {project.manifest.name}
                    <small>{project.state}</small>
                  </span>
                </label>
                {route && (
                  <>
                    <input
                      aria-label={
                        (tr ? "Alan adı: " : "Domain: ") + project.manifest.name
                      }
                      value={route.domain}
                      disabled={busy}
                      onChange={(e) =>
                        change({
                          ...settings,
                          routes: settings.routes.map((r) =>
                            r.project_id === route.project_id
                              ? { ...r, domain: e.target.value }
                              : r,
                          ),
                        })
                      }
                      required
                    />
                    <label className="route-toggle">
                      <input
                        type="checkbox"
                        checked={route.https}
                        disabled={busy}
                        onChange={(e) =>
                          change({
                            ...settings,
                            routes: settings.routes.map((r) =>
                              r.project_id === route.project_id
                                ? { ...r, https: e.target.checked }
                                : r,
                            ),
                          })
                        }
                      />
                      HTTPS
                    </label>
                  </>
                )}
              </div>
            );
          })}
        </div>
        {!candidates.length && (
          <p className="muted">
            {tr ? "Önce bir HTTP projesi ekle." : "Add an HTTP project first."}
          </p>
        )}
        <div className="heading-actions">
          <Button
            variant="primary"
            disabled={!connected || busy || !settings.routes.length}
          >
            {preview.isPending
              ? tr
                ? "Doğrulanıyor…"
                : "Validating…"
              : tr
                ? "Değişikliği önizle"
                : "Preview change"}
          </Button>
          {status.data?.running && (
            <Button type="button" disabled={busy} onClick={() => stop.mutate()}>
              {tr ? "Yönlendiriciyi durdur" : "Stop router"}
            </Button>
          )}
        </div>
      </form>
      {plan && (
        <section className="router-plan">
          <h2>{tr ? "Onay öncesi önizleme" : "Review before approval"}</h2>
          <div className="notice">
            <p>
              {tr
                ? "Uygulama ortak yönlendiriciyi kısa süreyle yeniden başlatır. Proje süreçleri çalışmaya devam eder. Proje durumu değişirse yeni önizleme gerekir."
                : "Applying briefly restarts the shared router. Project processes keep running. A project state change requires a new preview."}
            </p>
            <p>
              {tr
                ? "Bu adım hosts dosyasını veya sertifika güven deposunu değiştirmez. HTTPS için ayrıca yerel CA güveni gerekir."
                : "This step does not change hosts or certificate trust. HTTPS requires separate local CA trust."}
            </p>
          </div>
          <div className="config-preview">
            <div>
              <h3>{tr ? "Önce" : "Before"}</h3>
              <pre>
                {plan.before ||
                  (tr ? "Etkin yapılandırma yok" : "No active configuration")}
              </pre>
            </div>
            <div>
              <h3>{tr ? "Sonra" : "After"}</h3>
              <pre>{plan.after}</pre>
            </div>
          </div>
          {!!plan.hosts_entries.length && (
            <>
              <h3>
                {tr
                  ? "Gerekli yerel adres kayıtları"
                  : "Required local address entries"}
              </h3>
              <pre>{plan.hosts_entries.join("\n")}</pre>
              <p className="muted">
                {tr
                  ? "Özel alan adı kullanmak normal DNS yönlendirmesini yerelde değiştirebilir. Sistem izin adımı ayrı uygulanır."
                  : "Custom domains can override normal DNS locally. The system permission step is applied separately."}
              </p>
            </>
          )}
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => apply.mutate()}
          >
            {apply.isPending
              ? tr
                ? "Uygulanıyor…"
                : "Applying…"
              : tr
                ? "Yönlendirici değişikliğini uygula"
                : "Apply router change"}
          </Button>
        </section>
      )}
      {status.data?.root_certificate && (
        <div className="notice">
          <strong>
            {tr ? "Yerel CA sertifikası" : "Local CA certificate"}
          </strong>
          <p>{status.data.root_certificate}</p>
          <p>
            {tr
              ? "Sertifika oluşturulması tarayıcının güveneceği anlamına gelmez."
              : "Creating a certificate does not establish browser trust."}
          </p>
        </div>
      )}
      {connected && status.data?.settings && (
        <SystemPermissions
          lang={lang}
          hasCertificate={!!status.data.root_certificate}
        />
      )}
      {!!status.data?.logs.length && (
        <details className="router-logs">
          <summary>{tr ? "Yönlendirici günlükleri" : "Router logs"}</summary>
          <pre>{status.data.logs.join("\n")}</pre>
        </details>
      )}
    </>
  );
}
