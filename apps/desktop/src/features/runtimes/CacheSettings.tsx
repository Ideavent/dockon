import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download } from "lucide-react";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";

export const cacheNames = { redis: "Redis Cache", memcached: "Memcached" };
export type CacheService = keyof typeof cacheNames;

type CacheProject = {
  manifest: {
    id: string;
    name: string;
    provider?: "native" | "container";
    services?: {
      id: string;
      container?: { image: string };
      runtime?: { installation_id: string };
    }[];
  };
  state: string;
  services?: { id: string; state: string; port: number }[];
};
type Release = { version: string; image?: string; installation_id?: string };

export function CacheSettings({
  service,
  lang,
  connected,
  project,
}: {
  service: CacheService;
  lang: "tr" | "en";
  connected: boolean;
  project?: CacheProject;
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const [projectId, setProjectId] = useState("");
  const [version, setVersion] = useState("");
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => request<CacheProject[]>("list"),
    enabled: connected && !project,
  });
  const target =
    project ?? projects.data?.find((entry) => entry.manifest.id === projectId);
  const container = target?.manifest.provider === "container";
  const releases = useQuery({
    queryKey: ["cache-catalog", service, container],
    queryFn: () => request<Release[]>("cache_catalog", { service, container }),
    enabled: connected,
  });
  const current = target?.manifest.services?.find(
    (entry) => entry.id === service,
  );
  const status = target?.services?.find((entry) => entry.id === service);
  const assigned = releases.data?.find((entry) =>
    entry.installation_id
      ? entry.installation_id === current?.runtime?.installation_id
      : entry.image === current?.container?.image,
  );
  const chosen =
    releases.data?.find((entry) => entry.version === version)?.version ??
    assigned?.version ??
    releases.data?.[0]?.version ??
    "";
  const configure = useMutation({
    mutationFn: async ({
      id,
      version,
    }: {
      id: string;
      version: string | null;
    }) => {
      await waitOperation(
        (
          await request<Operation>("project_cache_set", {
            id,
            service,
            version,
          })
        ).id,
      );
    },
    onSettled: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const editable =
    connected && target?.state === "STOPPED" && !configure.isPending;
  const failure = configure.error || releases.error || projects.error;
  return (
    <section
      className="runtime-catalog cache-settings"
      aria-label={cacheNames[service]}
    >
      <p>
        {releases.data?.some((entry) => entry.installation_id)
          ? tr
            ? "Önbellek paketi Dockon tarafından indirilir ve projeyle Windows üzerinde çalışır."
            : "Dockon downloads the cache package and runs it with the project on Windows."
          : tr
            ? "Yerel önbellek paketleri Windows x64 projelerinde kullanılabilir."
            : "Local cache packages are available for Windows x64 projects."}
      </p>
      {!project && (
        <label>
          {tr ? "Proje" : "Project"}
          <select
            value={projectId}
            disabled={configure.isPending}
            onChange={(event) => {
              setProjectId(event.target.value);
              setVersion("");
              configure.reset();
            }}
          >
            <option value="">{tr ? "Proje seç" : "Choose a project"}</option>
            {projects.data?.map((entry) => (
              <option key={entry.manifest.id} value={entry.manifest.id}>
                {entry.manifest.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {target && (
        <p>
          {tr ? "Atanan sürüm" : "Assigned version"} ·{" "}
          {assigned?.version ??
            (current
              ? tr
                ? "Mevcut özel imaj"
                : "Existing custom image"
              : "—")}
          {status?.state === "RUNNING" && (
            <>
              {" "}
              · <code>127.0.0.1:{status.port}</code>
            </>
          )}
        </p>
      )}
      {target && target.state !== "STOPPED" && (
        <p className="muted">
          {tr
            ? "Sürümü değiştirmek için projeyi durdur."
            : "Stop the project to change its version."}
        </p>
      )}
      <label>
        {tr ? "Sürüm" : "Version"}
        <select
          value={chosen}
          disabled={!releases.data?.length || configure.isPending}
          onChange={(event) => setVersion(event.target.value)}
        >
          {!releases.data?.length && <option value="">—</option>}
          {releases.data?.map((entry) => (
            <option key={entry.version} value={entry.version}>
              {entry.version}
            </option>
          ))}
        </select>
      </label>
      <div className="heading-actions">
        <Button
          variant="primary"
          disabled={!editable || !chosen || chosen === assigned?.version}
          onClick={() =>
            target &&
            configure.mutate({ id: target.manifest.id, version: chosen })
          }
        >
          <Download size={15} />{" "}
          {configure.isPending
            ? tr
              ? "Uygulanıyor…"
              : "Applying…"
            : tr
              ? "İndir ve projeye ata"
              : "Download and assign"}
        </Button>
        {current && (
          <Button
            disabled={!editable}
            onClick={() =>
              target &&
              configure.mutate({ id: target.manifest.id, version: null })
            }
          >
            {tr ? "Projeden kaldır" : "Remove from project"}
          </Button>
        )}
      </div>
      {failure && <p role="alert">{failure.message}</p>}
    </section>
  );
}
