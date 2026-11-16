import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request } from "../../ipc/client";
import { SystemPermissions } from "./SystemPermissions";

type Route = { project_id: string; domain: string; https: boolean };
export type RouterStatus = {
  settings: {
    http_port: number;
    https_port: number;
    domain_suffix: string;
    default_https: boolean;
    routes: Route[];
  } | null;
  running: boolean;
  error: string | null;
  hosts_pending: boolean;
  root_certificate: string | null;
  certificate_trusted: boolean;
};

export function RouterPage({
  connected,
  lang,
}: {
  connected: boolean;
  lang: "tr" | "en";
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const router = useQuery({
    queryKey: ["router"],
    queryFn: () => request<RouterStatus>("router_status"),
    enabled: connected,
    refetchInterval: 2000,
  });
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () =>
      request<
        {
          manifest: {
            id: string;
            name: string;
            health?: { type: string };
            services?: { id: string }[];
          };
        }[]
      >("list"),
    enabled: connected,
  });
  const [draft, setDraft] = useState<{ suffix: string; https: boolean } | null>(
    null,
  );
  const settings = router.data?.settings;
  const saved = {
    suffix: settings?.domain_suffix ?? "test",
    https: settings?.default_https ?? false,
  };
  const values = draft ?? saved;
  const changed =
    values.suffix.trim().replace(/^\.+/, "").toLowerCase() !== saved.suffix ||
    values.https !== saved.https;
  const save = useMutation({
    mutationFn: () =>
      request("router_preferences", {
        domain_suffix: values.suffix,
        default_https: values.https,
      }),
    onSuccess: async () => {
      await cache.invalidateQueries({ queryKey: ["router"] });
      setDraft(null);
    },
  });
  return (
    <>
      <section className="page-heading">
        <h1>{tr ? "Yerel adresler" : "Local domains"}</h1>
      </section>
      <form
        className="router-form"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate();
        }}
      >
        <label>
          {tr ? "Alan adı uzantısı" : "Domain suffix"}
          <input
            required
            spellCheck={false}
            disabled={!connected || router.isPending || save.isPending}
            value={
              values.suffix.startsWith(".")
                ? values.suffix
                : "." + values.suffix
            }
            onChange={(event) =>
              setDraft({
                ...values,
                suffix: event.target.value.replace(/^\.+/, ""),
              })
            }
          />
        </label>
        <label className="project-ssl">
          <input
            type="checkbox"
            checked={values.https}
            disabled={!connected || router.isPending || save.isPending}
            onChange={(event) =>
              setDraft({ ...values, https: event.target.checked })
            }
          />
          {tr
            ? "Yeni projelerde varsayılan SSL"
            : "Default SSL for new projects"}
        </label>
        <Button
          type="submit"
          disabled={!connected || !changed || save.isPending}
        >
          {tr ? "Kaydet" : "Save"}
        </Button>
      </form>
      {(save.error || router.error || router.data?.error) && (
        <p role="alert">
          {save.error?.message || router.error?.message || router.data?.error}
        </p>
      )}
      <section className="router-plan">
        <h2>{tr ? "Mevcut adresler" : "Current addresses"}</h2>
        {settings?.routes.length ? (
          settings.routes.map((route) => {
            const project = projects.data?.find(
              ({ manifest }) => manifest.id === route.project_id,
            );
            const hasMail = project?.manifest.services?.some(
              ({ id }) => id === "mailpit",
            );
            const windows = navigator.userAgent.includes("Windows");
            const standard = windows && !route.domain.endsWith(".localhost");
            const scheme = route.https ? "https" : "http";
            const defaultPort = route.https ? 443 : 80;
            const port = route.https ? settings.https_port : settings.http_port;
            const suffix = standard || port === defaultPort ? "" : ":" + port;
            const mailSuffix =
              standard || settings.http_port === 80
                ? ""
                : ":" + settings.http_port;
            return (
              <div className="project-location" key={route.project_id}>
                <strong>{project?.manifest.name ?? route.project_id}</strong>
                {project?.manifest.health?.type !== "tcp" && (
                  <code>
                    {scheme}://{route.domain}
                    {suffix}/
                  </code>
                )}
                {hasMail && (
                  <code>
                    http://webmail.{route.domain}
                    {mailSuffix}/
                  </code>
                )}
                {hasMail && standard && (
                  <code>http://mail.{route.domain}/api/v1/</code>
                )}
              </div>
            );
          })
        ) : (
          <p>{tr ? "Henüz yerel adres yok." : "No local domains yet."}</p>
        )}
        <SystemPermissions
          lang={lang}
          hostsPending={router.data?.hosts_pending ?? false}
        />
      </section>
    </>
  );
}
