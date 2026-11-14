import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";
import { SystemPermissions } from "../routing/SystemPermissions";

type RouterStatus = {
  running: boolean;
  root_certificate: string | null;
  certificate_trusted: boolean;
  settings: {
    http_port: number;
    https_port: number;
    routes: { project_id: string; domain: string; https: boolean }[];
  } | null;
};

export function ProjectSettings({ id, name, root, lang, onClose, serverId, phpId, stopped, runtimes }: {
  id: string;
  name: string;
  root: string;
  lang: "tr" | "en";
  onClose: () => void;
  serverId?: string;
  phpId?: string;
  stopped: boolean;
  runtimes: { id: string; runtime: string; version: string }[];
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const router = useQuery({ queryKey: ["router"], queryFn: () => request<RouterStatus>("router_status") });
  const route = router.data?.settings?.routes.find((item) => item.project_id === id);
  const settings = router.data?.settings;
  const hostname = route?.domain || `${root.split(/[\\/]/).at(-1)?.toLowerCase()}.test`;
  const port = route?.https ? settings?.https_port : settings?.http_port;
  const standardPort = route?.https ? 443 : 80;
  const save = useMutation({
    mutationFn: (https: boolean) => request<RouterStatus>("workspace_route", { id, https }),
    onSuccess: (value) => cache.setQueryData(["router"], value),
  });
  const component = useMutation({
    mutationFn: async ({ kind, runtime }: { kind: "server" | "php"; runtime: string }) => {
      const params = { id, runtime_installation: runtime };
      if (kind === "php") await request("project_php_version", params);
      else await waitOperation((await request<Operation>("project_switch_static_server", params)).id);
    },
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const servers = runtimes.filter((entry) => ["apache", "nginx"].includes(entry.runtime));
  const php = runtimes.filter((entry) => entry.runtime === "php-cgi");

  return <div className="modal-backdrop">
    <section className="modal project-wizard" role="dialog" aria-modal="true" aria-label={tr ? "Proje ayarları" : "Project settings"}>
      <h2>{name}</h2>
      {servers.some((entry) => entry.id === serverId) && (
        <label>{tr ? "Web sunucusu" : "Web server"}
          <select value={serverId} disabled={!stopped || component.isPending}
            onChange={(event) => component.mutate({ kind: "server", runtime: event.target.value })}>
            {servers.map((entry) => <option key={entry.id} value={entry.id}>{entry.runtime === "apache" ? "Apache" : "Nginx"} {entry.version}</option>)}
          </select>
        </label>
      )}
      {phpId && (
        <label>PHP (FastCGI)
          <select value={phpId} disabled={!stopped || component.isPending}
            onChange={(event) => component.mutate({ kind: "php", runtime: event.target.value })}>
            {php.map((entry) => <option key={entry.id} value={entry.id}>PHP {entry.version}</option>)}
          </select>
        </label>
      )}
      {component.error && <p role="alert">{component.error.message}</p>}
      <div className="project-location">
        <span>{tr ? "Yerel adres" : "Local address"}</span>
        <code>{route && port ? `${route.https ? "https" : "http"}://${hostname}${port === standardPort ? "" : `:${port}`}` : hostname}</code>
      </div>
      <label className="project-ssl">
        <input type="checkbox" checked={route?.https ?? false} disabled={save.isPending || router.isPending}
          onChange={(event) => save.mutate(event.target.checked)} />
        {tr ? "SSL kullan (HTTPS)" : "Use SSL (HTTPS)"}
      </label>
      {route?.https && (
        <p role="status">{router.data?.certificate_trusted
          ? (tr ? "Dockon sertifikası Windows’ta güvenilir." : "Windows trusts the Dockon certificate.")
          : (tr ? "Tarayıcı uyarısını kaldırmak için aşağıdan Dockon sertifikasına güven." : "Trust the Dockon certificate below to clear the browser warning.")}</p>
      )}
      {!router.data?.running && (
        <Button variant="secondary" disabled={save.isPending || router.isPending}
          onClick={() => save.mutate(route?.https ?? false)}>
          {tr ? "Yerel erişimi başlat" : "Start local access"}
        </Button>
      )}
      {save.error && <p role="alert">{save.error.message}</p>}
      {router.error && <p role="alert">{router.error.message}</p>}
      {route && <SystemPermissions lang={lang} hasCertificate={Boolean(router.data?.root_certificate)}
        onApplied={() => void cache.invalidateQueries({ queryKey: ["router"] })} />}
      <div className="heading-actions"><Button type="button" onClick={onClose}>{tr ? "Kapat" : "Close"}</Button></div>
    </section>
  </div>;
}
