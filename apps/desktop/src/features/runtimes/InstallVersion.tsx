import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";

export function AndroidLicense({
  tr,
  checked,
  onChange,
}: {
  tr: boolean;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="project-ssl">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        {tr ? "Şu koşulları kabul ediyorum" : "I accept the"}{" "}
        <a
          href="https://developer.android.com/studio#command-tools"
          target="_blank"
          rel="noreferrer"
        >
          {tr ? "Android SDK lisansı" : "Android SDK license"}
        </a>
      </span>
    </label>
  );
}

export function InstallVersion({
  runtime,
  label,
  tr,
}: {
  runtime: string;
  label: string;
  tr: boolean;
}) {
  const cache = useQueryClient();
  const [open, setOpen] = useState(false);
  const [choice, setChoice] = useState("");
  const [accepted, setAccepted] = useState(false);
  const catalog = useQuery({
    queryKey: ["runtime-catalog", runtime],
    queryFn: () =>
      request<{ versions: { version: string; support: string }[] }>(
        "runtime_catalog",
        { runtime },
      ),
    enabled: open,
  });
  const versions =
    catalog.data?.versions.filter((row) => row.support !== "eol") ?? [];
  const version = choice || versions[0]?.version;
  const install = useMutation({
    mutationFn: async () => {
      const operation = await request<Operation>("runtime_install", {
        runtime,
        version,
      });
      await waitOperation(operation.id);
      await cache.invalidateQueries({ queryKey: ["runtimes"] });
    },
    onSuccess: () => {
      setOpen(false);
    },
  });
  return (
    <div className="project-location">
      <Button
        type="button"
        variant="secondary"
        onClick={() => setOpen(!open)}
        disabled={install.isPending}
      >
        {tr ? `${label} indir` : `Download ${label}`}
      </Button>
      {open && (
        <>
          <label>
            {tr ? "İndirilecek sürüm" : "Version to download"}
            <select
              value={version ?? ""}
              onChange={(event) => setChoice(event.target.value)}
              disabled={install.isPending}
            >
              {!versions.length && (
                <option value="">
                  {catalog.isPending ? (tr ? "Yükleniyor…" : "Loading…") : "—"}
                </option>
              )}
              {versions.map((row) => (
                <option key={row.version} value={row.version}>
                  {row.version}
                </option>
              ))}
            </select>
          </label>
          {runtime === "android-tools" && (
            <AndroidLicense tr={tr} checked={accepted} onChange={setAccepted} />
          )}
          <Button
            type="button"
            disabled={
              !version ||
              install.isPending ||
              (runtime === "android-tools" && !accepted)
            }
            onClick={() => install.mutate()}
          >
            {install.isPending
              ? tr
                ? "Kuruluyor…"
                : "Installing…"
              : tr
                ? "Sürümü kur"
                : "Install version"}
          </Button>
          {(catalog.error || install.error) && (
            <p role="alert">{(catalog.error || install.error)?.message}</p>
          )}
        </>
      )}
    </div>
  );
}
