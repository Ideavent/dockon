import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request } from "../../ipc/client";
import { SystemPermissions } from "../routing/SystemPermissions";

type RouterStatus = {
  running: boolean;
  root_certificate: string | null;
  settings: {
    http_port: number;
    https_port: number;
    routes: { project_id: string; domain: string; https: boolean }[];
  } | null;
};

export function ProjectSettings({ id, name, root, lang, onClose }: {
  id: string;
  name: string;
  root: string;
  lang: "tr" | "en";
  onClose: () => void;
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const router = useQuery({ queryKey: ["router"], queryFn: () => request<RouterStatus>("router_status") });
  const route = router.data?.settings?.routes.find((item) => item.project_id === id);
  const settings = router.data?.settings;
  const hostname = route?.domain || `${root.split(/[\\/]/).at(-1)?.toLowerCase()}.test`;
  const port = route?.https ? settings?.https_port : settings?.http_port;
  const save = useMutation({
    mutationFn: (https: boolean) => request<RouterStatus>("workspace_route", { id, https }),
    onSuccess: (value) => cache.setQueryData(["router"], value),
  });

  return <div className="modal-backdrop">
    <section className="modal project-wizard" role="dialog" aria-modal="true" aria-label={tr ? "Proje ayarları" : "Project settings"}>
      <h2>{name}</h2>
      <div className="project-location">
        <span>{tr ? "Yerel adres" : "Local address"}</span>
        <code>{route && port ? `${route.https ? "https" : "http"}://${hostname}:${port}` : hostname}</code>
      </div>
      <label className="project-ssl">
        <input type="checkbox" checked={route?.https ?? false} disabled={save.isPending || router.isPending}
          onChange={(event) => save.mutate(event.target.checked)} />
        {tr ? "SSL kullan (HTTPS)" : "Use SSL (HTTPS)"}
      </label>
      {!router.data?.running && (
        <Button variant="secondary" disabled={save.isPending || router.isPending}
          onClick={() => save.mutate(route?.https ?? false)}>
          {tr ? "Yerel erişimi başlat" : "Start local access"}
        </Button>
      )}
      {save.error && <p role="alert">{save.error.message}</p>}
      {router.error && <p role="alert">{router.error.message}</p>}
      {route && <SystemPermissions lang={lang} hasCertificate={Boolean(router.data?.root_certificate)} />}
      <div className="heading-actions"><Button type="button" onClick={onClose}>{tr ? "Kapat" : "Close"}</Button></div>
    </section>
  </div>;
}
