import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Download, RefreshCw } from "lucide-react";
import { Button } from "../../components/ui/button";
import { request } from "../../ipc/client";

type Report = {
  dockon_version: string;
  platform: { os: string; arch: string };
  project_counts: Record<string, number>;
  service_counts: Record<string, number>;
  runtimes: { installed: number };
  router: { running: boolean; has_error: boolean };
};

export function DiagnosticsPage({
  connected,
  lang,
}: {
  connected: boolean;
  lang: "tr" | "en";
}) {
  const tr = lang === "tr";
  const [ecosystem, setEcosystem] = useState("npm");
  const [packageName, setPackageName] = useState("");
  const [version, setVersion] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const guardian = useMutation({
    mutationFn: () => request<{ provider_base_url: string; context: Report }>("guardian_preview"),
  });
  const models = useMutation({
    mutationFn: () => request<string[]>("guardian_models", { api_key: apiKey.trim() }),
    onSuccess: (available) => setModel(available[0] || ""),
    onError: () => setApiKey(""),
  });
  const analysis = useMutation({
    mutationFn: () => request<string>("guardian_analyze", {
      api_key: apiKey.trim(), model, context: guardian.data?.context,
    }),
    onSettled: () => setApiKey(""),
  });
  const advisories = useMutation({
    mutationFn: () =>
      request<{ partial: boolean; advisories: { id: string; summary: string }[] }>(
        "advisory_lookup",
        { ecosystem, package: packageName.trim(), version: version.trim() },
      ),
  });
  const report = useQuery({
    queryKey: ["diagnostics"],
    queryFn: () => request<Report>("diagnostics"),
    enabled: connected,
  });

  function download() {
    if (!report.data) return;
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(report.data, null, 2)], { type: "application/json" }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = "dockon-diagnostics.json";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  return (
    <>
      <section className="page-heading">
        <div>
          <h1>{tr ? "Tanılama" : "Diagnostics"}</h1>
          <p>
            {tr
              ? "Paylaşılabilir durum raporu proje adlarını, dosya yollarını ve günlükleri içermez."
              : "The shareable status report excludes project names, file paths and logs."}
          </p>
        </div>
        <div className="heading-actions">
          <Button variant="secondary" disabled={!connected || report.isFetching} onClick={() => void report.refetch()}>
            <RefreshCw size={15} /> {tr ? "Yenile" : "Refresh"}
          </Button>
          <Button disabled={!connected || !report.data} onClick={download}>
            <Download size={15} /> {tr ? "JSON indir" : "Download JSON"}
          </Button>
        </div>
      </section>
      {!connected && <p>{tr ? "Daemon bağlantısı gerekli." : "Daemon connection required."}</p>}
      {report.isFetching && !report.data && <p>{tr ? "Durum alınıyor…" : "Reading status…"}</p>}
      {report.isError && <p role="alert">{tr ? "Durum raporu alınamadı." : "The status report could not be loaded."}</p>}
      {connected && report.data && (
        <section aria-label={tr ? "Durum raporu" : "Status report"}>
          <pre>{JSON.stringify(report.data, null, 2)}</pre>
        </section>
      )}
      <section className="runtime-catalog">
        <h2>{tr ? "Paket güvenlik duyuruları" : "Package security advisories"}</h2>
        <p className="muted">
          {tr
            ? "Sorgu, girdiğin ekosistem, paket adı ve sürümü OSV’ye gönderir. Yalnızca herkese açık paketleri sorgula."
            : "The lookup sends the ecosystem, package name and version you enter to OSV. Use public package names."}
        </p>
        <form className="runtime-toolbar" onSubmit={(event) => {
          event.preventDefault();
          advisories.mutate();
        }}>
          <label>
            {tr ? "Ekosistem" : "Ecosystem"}
            <select value={ecosystem} disabled={advisories.isPending} onChange={(event) => {
              setEcosystem(event.target.value);
              advisories.reset();
            }}>
              {["npm", "PyPI", "crates.io", "Go", "Packagist", "NuGet"].map((value) => (
                <option key={value}>{value}</option>
              ))}
            </select>
          </label>
          <label>
            {tr ? "Paket adı" : "Package name"}
            <input required maxLength={200} value={packageName} disabled={advisories.isPending} onChange={(event) => {
              setPackageName(event.target.value);
              advisories.reset();
            }} />
          </label>
          <label>
            {tr ? "Tam sürüm" : "Exact version"}
            <input required maxLength={96} value={version} disabled={advisories.isPending} onChange={(event) => {
              setVersion(event.target.value);
              advisories.reset();
            }} />
          </label>
          <Button disabled={!connected || advisories.isPending}>
            {advisories.isPending ? (tr ? "Sorgulanıyor…" : "Looking up…") : (tr ? "OSV’de sorgula" : "Look up in OSV")}
          </Button>
        </form>
        {advisories.isError && <p role="alert">{tr ? "OSV sorgusu başarısız." : "The OSV lookup failed."}</p>}
        {advisories.data && (
          <>
            <p role="status">
              {tr ? "OSV eşleşmeleri" : "OSV matches"} · {advisories.data.advisories.length}
              {advisories.data.partial && (tr ? " · Kısmi sonuç" : " · Partial results")}
            </p>
            <ul>
              {advisories.data.advisories.map((item) => (
                <li key={item.id}><strong>{item.id}</strong> {item.summary}</li>
              ))}
            </ul>
          </>
        )}
      </section>
      <section className="runtime-catalog">
        <h2>AI Guardian</h2>
        <p className="muted">
          {tr
            ? "Anahtar bu sayfa açıkken bellekte tutulur. Model listesi sorgusu proje verisi göndermez."
            : "The key stays in memory while this page is open. Model lookup sends no project data."}
        </p>
        <Button variant="secondary" disabled={!connected || guardian.isPending || analysis.isPending} onClick={() => guardian.mutate()}>
          {tr ? "Veriyi önizle" : "Preview context"}
        </Button>
        {guardian.isError && <p role="alert">{tr ? "Önizleme alınamadı." : "Preview failed."}</p>}
        {guardian.data && (
          <>
            <p><code>{guardian.data.provider_base_url}</code></p>
            <pre>{JSON.stringify(guardian.data.context, null, 2)}</pre>
            <form className="runtime-toolbar" onSubmit={(event) => {
              event.preventDefault();
              if (connected && apiKey.trim() && !models.isPending && !analysis.isPending) models.mutate();
            }}>
              <label>
                {tr ? "Sağlayıcı API anahtarı" : "Provider API key"}
                <input type="password" autoComplete="off" required maxLength={1024} value={apiKey}
                  disabled={models.isPending || analysis.isPending}
                  onChange={(event) => { setApiKey(event.target.value); setModel(""); models.reset(); }} />
              </label>
              <Button disabled={!connected || !apiKey.trim() || models.isPending || analysis.isPending}>
                {tr ? "Modelleri getir" : "List models"}
              </Button>
            </form>
            {models.isError && <p role="alert">{models.error.message}</p>}
            {models.data?.length === 0 && <p>{tr ? "Sağlayıcı model döndürmedi." : "The provider returned no models."}</p>}
            {!!models.data?.length && (
              <form className="runtime-toolbar" onSubmit={(event) => {
                event.preventDefault();
                if (connected && apiKey.trim() && model && !analysis.isPending) analysis.mutate();
              }}>
                <label>
                  Model
                  <select value={model} onChange={(event) => setModel(event.target.value)} disabled={analysis.isPending}>
                    {models.data.map((id) => <option key={id}>{id}</option>)}
                  </select>
                </label>
                <Button disabled={!connected || !apiKey.trim() || !model || analysis.isPending || guardian.isPending}>
                  {tr ? "Önizlenen veriyi gönder ve analiz et" : "Send previewed context and analyze"}
                </Button>
              </form>
            )}
            {analysis.isError && <p role="alert">{analysis.error.message}</p>}
            {analysis.data && <p style={{ whiteSpace: "pre-wrap" }}>{analysis.data}</p>}
          </>
        )}
      </section>
    </>
  );
}
