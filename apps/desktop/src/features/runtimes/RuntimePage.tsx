import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Box,
  Download,
  RefreshCw,
  Search,
  ShieldCheck,
  Terminal,
} from "lucide-react";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";

type Installation = {
  id: string;
  runtime: string;
  version: string;
  executable: string;
  root: string;
  source: string;
  archive_hash: string | null;
  signature_verified: boolean;
};
type Catalog = {
  runtime: string;
  platform: string;
  versions: { version: string; support: string }[];
  note: string | null;
  cached: boolean;
  fetched_at: string;
};
type Candidate = {
  runtime: string;
  executable: string;
  version_checked: boolean;
};
type RuntimeUpdates = {
  cached: boolean;
  updates: {
    installation_id: string;
    current_version: string;
    version: string;
    same_major: boolean;
    already_installed: boolean;
  }[];
};
const names: Record<string, string> = {
  node: "Node.js",
  php: "PHP",
  dotnet: ".NET SDK",
  go: "Go",
  caddy: "Caddy",
  apache: "Apache",
  nginx: "Nginx",
  "php-cgi": "PHP CGI",
  python: "Python",
  rust: "Rust / Cargo",
};
const labels = {
  tr: {
    title: "Çalışma ortamları",
    subtitle: "Sürümleri yan yana kur. Her proje için ayrı seç.",
    catalog: "Resmi sürüm kataloğu",
    install: "Sürümü kur",
    installing: "Kuruluyor…",
    installed: "Kurulu sürümler",
    empty: "Henüz bir çalışma zamanı eklenmedi.",
    discover: "Sistem araçlarını bul",
    system: "Sistem araçları",
    optIn: "Bu aracı kullan",
    explicit: "Kullanıcı seçimi",
    digest: "Yayıncı özeti doğrulandı",
    details: "Kaynak ve konum",
    unsigned: "Yayıncı imzası doğrulanmadı",
    cached: "Önbellekteki katalog",
    refresh: "Yenile",
    offline: "Önce masaüstü daemon bağlantısını kur.",
    source: "Kaynak",
    noVersions: "Bu platform için yönetilen indirme mevcut değil.",
    select: "Sürüm",
    systemNote:
      "Bulma işlemi araçları çalıştırmaz. Kullan düğmesiyle seçtiğin aracın sürümü kontrol edilir.",
  },
  en: {
    title: "Runtimes",
    subtitle:
      "Install versions side by side. Choose independently for each project.",
    catalog: "Official release catalog",
    install: "Install version",
    installing: "Installing…",
    installed: "Installed versions",
    empty: "No runtimes added yet.",
    discover: "Find system tools",
    system: "System tools",
    optIn: "Use this tool",
    explicit: "Explicit system selection",
    digest: "Publisher digest verified",
    details: "Source and location",
    unsigned: "Publisher signature not verified",
    cached: "Cached catalog",
    refresh: "Refresh",
    offline: "Connect the desktop daemon first.",
    source: "Source",
    noVersions: "No managed download is available on this platform.",
    select: "Version",
    systemNote:
      "Discovery does not execute tools. Selecting Use checks the chosen executable version.",
  },
};
export function RuntimePage({
  connected,
  lang,
}: {
  connected: boolean;
  lang: "tr" | "en";
}) {
  const t = labels[lang];
  const cache = useQueryClient();
  const [runtime, setRuntime] = useState("node");
  const [version, setVersion] = useState("");
  const [error, setError] = useState("");
  const [systemPath, setSystemPath] = useState("");
  const installed = useQuery({
    queryKey: ["runtimes"],
    queryFn: () => request<Installation[]>("runtime_installed"),
    enabled: connected,
  });
  const catalog = useQuery({
    queryKey: ["runtime-catalog", runtime],
    queryFn: () => request<Catalog>("runtime_catalog", { runtime }),
    enabled: connected,
    retry: false,
    staleTime: 300000,
  });
  const candidates = useQuery({
    queryKey: ["runtime-system"],
    queryFn: () => request<Candidate[]>("runtime_discover"),
    enabled: false,
    retry: false,
  });
  const updates = useQuery({
    queryKey: ["runtime-updates", runtime, installed.dataUpdatedAt],
    queryFn: () => request<RuntimeUpdates>("runtime_updates", { runtime }),
    enabled: false,
    retry: false,
  });
  useEffect(() => {
    if (
      catalog.data &&
      !catalog.data.versions.some((v) => v.version === version)
    )
      setVersion(
        catalog.data.versions.find((v) => v.support === "lts")?.version ||
          catalog.data.versions[0]?.version ||
          "",
      );
  }, [catalog.data, version]);
  const install = useMutation({
    mutationFn: async () => {
      const operation = await request<Operation>("runtime_install", {
        runtime,
        version,
      });
      await waitOperation(operation.id);
    },
    onSuccess: () => {
      setError("");
      cache.invalidateQueries({ queryKey: ["runtimes"] });
    },
    onError: (error) => setError(String(error)),
  });
  const system = useMutation({
    mutationFn: (candidate: Candidate) =>
      request("runtime_use_system", {
        runtime: candidate.runtime,
        executable: candidate.executable,
      }),
    onSuccess: () => {
      setError("");
      cache.invalidateQueries({ queryKey: ["runtimes"] });
    },
    onError: (error) => setError(String(error)),
  });
  const failure =
    error ||
    catalog.error?.message ||
    installed.error?.message ||
    candidates.error?.message;
  return (
    <>
      <section className="page-heading">
        <div>
          <div className="eyebrow">YOUR TOOLCHAIN</div>
          <h1>{t.title}</h1>
          <p>{t.subtitle}</p>
        </div>
        <Button
          disabled={!connected || candidates.isFetching}
          onClick={() => candidates.refetch()}
        >
          <Search size={15} />
          {t.discover}
        </Button>
      </section>
      {!connected && <div className="notice">{t.offline}</div>}
      {failure && (
        <div className="error" role="alert">
          {failure}
        </div>
      )}
      <section className="runtime-catalog">
        <div className="runtime-toolbar">
          <h2>
            <Box size={17} />
            {t.catalog}
          </h2>
          <Button
            disabled={!connected || catalog.isFetching}
            onClick={() => catalog.refetch()}
            aria-label={t.refresh}
          >
            <RefreshCw size={14} />
          </Button>
        </div>
        <div className="runtime-picker">
          <select
            aria-label="Runtime"
            value={runtime}
            onChange={(event) => {
              setRuntime(event.target.value);
              setVersion("");
            }}
          >
            {Object.entries(names).map(([id, name]) => (
              <option value={id} key={id}>
                {name}
              </option>
            ))}
          </select>
          <select
            aria-label={t.select}
            value={version}
            onChange={(event) => setVersion(event.target.value)}
            disabled={!catalog.data?.versions.length}
          >
            {!catalog.data?.versions.length && <option value="">—</option>}
            {catalog.data?.versions.map((value) => (
              <option key={value.version} value={value.version}>
                {value.version} ·{" "}
                {value.support === "lts"
                  ? "LTS"
                  : value.support === "eol"
                    ? lang === "tr"
                      ? "Destek sona erdi"
                      : "End of life"
                    : value.support === "current"
                      ? lang === "tr"
                        ? "Güncel"
                        : "Current"
                      : value.support === "publisher_release_support_unverified"
                        ? lang === "tr"
                          ? "Destek durumu doğrulanmadı"
                          : "Support not verified"
                        : lang === "tr"
                          ? "Yayıncı sürümü"
                          : "Publisher release"}
              </option>
            ))}
          </select>
          <Button
            variant="primary"
            disabled={
              !connected || !version || install.isPending || catalog.isFetching
            }
            onClick={() => install.mutate()}
          >
            <Download size={15} />
            {install.isPending ? t.installing : t.install}
          </Button>
        </div>
        {catalog.isFetching && (
          <p className="muted">
            {lang === "tr"
              ? "Yayıncı kataloğu okunuyor…"
              : "Reading publisher catalog…"}
          </p>
        )}
        {catalog.data?.note && <p className="muted">{catalog.data.note}</p>}
        {catalog.data && (
          <p className="catalog-meta">
            {catalog.data.platform} ·{" "}
            {catalog.data.cached ? t.cached : t.source} ·{" "}
            {new Date(catalog.data.fetched_at).toLocaleString(lang)}
          </p>
        )}
      </section>
      <div className="section-label">
        {t.installed}
        <span>{installed.data?.length ?? "—"}</span>
      </div>
      {installed.data?.some((entry) => entry.runtime === runtime) && (
        <section className="runtime-catalog">
          <Button
            variant="secondary"
            disabled={!connected || updates.isFetching}
            onClick={() => void updates.refetch()}
          >
            <RefreshCw size={15} />
            {lang === "tr" ? "Güncellemeleri kontrol et" : "Check for updates"}
          </Button>
          {updates.isError && (
            <p role="alert">{lang === "tr" ? "Güncelleme kontrolü başarısız." : "Update check failed."}</p>
          )}
          {updates.data?.cached && <p className="muted">{t.cached}</p>}
          {updates.data && !updates.data.updates.length && (
            <p className="muted">
              {lang === "tr" ? "Katalogda daha yeni kararlı bir sürüm bulunamadı." : "No newer stable version was found in the catalog."}
            </p>
          )}
          {updates.data?.updates.map((update) => (
            <p key={update.installation_id}>
              <strong>{update.current_version} → {update.version}</strong>{" · "}
              {update.same_major
                ? lang === "tr" ? "Aynı ana sürüm" : "Same major version"
                : lang === "tr" ? "Ana sürüm değişiyor" : "Major version change"}
              {update.already_installed && (lang === "tr" ? " · Kurulu" : " · Installed")}
            </p>
          ))}
        </section>
      )}
      <section className="runtime-list">
        {installed.data?.map((value) => (
          <article className="runtime-item" key={value.id}>
            <div className="project-icon">
              <Terminal size={20} />
            </div>
            <div className="runtime-info">
              <h2>
                {names[value.runtime] || value.runtime}{" "}
                <span>{value.version}</span>
              </h2>
              <p>
                <ShieldCheck size={12} />
                {value.archive_hash ? t.digest : t.explicit}
              </p>
              <details>
                <summary>{t.details}</summary>
                <code>
                  {value.source}
                  <br />
                  {value.executable}
                </code>
                {value.archive_hash && <small>{t.unsigned}</small>}
              </details>
            </div>
          </article>
        ))}
        {connected && installed.isSuccess && !installed.data?.length && (
          <div className="empty">
            <Box size={28} />
            <p>{t.empty}</p>
          </div>
        )}
      </section>
      <section className="runtime-catalog">
        <h2>
          {lang === "tr"
            ? "Dosyadan sistem aracı seç"
            : "Select a system executable"}
        </h2>
        <p className="muted">
          {lang === "tr"
            ? "Yukarıda seçili çalışma zamanı için tam dosya yolunu gir. Bu aracı kullan düğmesi sürüm kontrolünü çalıştırır; dosyaları taşımaz ve global PATH’i değiştirmez."
            : "Enter the executable path for the runtime selected above. Use this tool runs a version check; it does not move files or change global PATH."}
        </p>
        <form
          className="runtime-toolbar"
          onSubmit={(e) => {
            e.preventDefault();
            system.mutate({
              runtime,
              executable: systemPath,
              version_checked: false,
            });
          }}
        >
          <label htmlFor="system-executable">
            {lang === "tr"
              ? "Çalıştırılabilir dosyanın tam yolu"
              : "Absolute executable path"}
          </label>
          <input
            id="system-executable"
            value={systemPath}
            onChange={(e) => setSystemPath(e.target.value)}
            required
            disabled={system.isPending || !connected}
          />
          <Button disabled={!connected || system.isPending || !systemPath}>
            {t.optIn}
          </Button>
        </form>
      </section>
      {candidates.data && (
        <section className="system-tools">
          <h2>{t.system}</h2>
          <p className="muted">{t.systemNote}</p>
          {candidates.data.map((candidate) => (
            <div className="system-tool" key={candidate.executable}>
              <div>
                <strong>{names[candidate.runtime]}</strong>
                <code>{candidate.executable}</code>
              </div>
              <Button
                disabled={system.isPending || !connected}
                onClick={() => system.mutate(candidate)}
              >
                {t.optIn}
              </Button>
            </div>
          ))}
        </section>
      )}
    </>
  );
}
