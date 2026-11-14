import { useQuery } from "@tanstack/react-query";
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
    </>
  );
}
