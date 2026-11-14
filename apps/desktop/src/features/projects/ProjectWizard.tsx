import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request } from "../../ipc/client";

type Installation = {
  id: string;
  runtime: string;
  version: string;
  platform: string;
};
type Detection = {
  root: string;
  profiles: { id: string; runtime: string; evidence: string }[];
};
export function ProjectWizard({
  lang,
  onClose,
}: {
  lang: "tr" | "en";
  onClose: () => void;
}) {
  const tr = lang === "tr";
  const [mode, setMode] = useState<"create" | "import">("create");
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [installation, setInstallation] = useState("");
  const [profile, setProfile] = useState("");
  const cache = useQueryClient();
  const installed = useQuery({
    queryKey: ["runtimes"],
    queryFn: () => request<Installation[]>("runtime_installed"),
  });
  const detect = useMutation({
    mutationFn: () => request<Detection>("project_detect", { root }),
    onSuccess: () => {
      setProfile("");
      setInstallation("");
    },
  });
  const save = useMutation({
    mutationFn: () =>
      request(mode === "create" ? "project_create" : "project_import", {
        id,
        name,
        root,
        runtime_installation: installation,
        ...(mode === "import" ? { profile } : {}),
      }),
    onSuccess: () => {
      cache.invalidateQueries({ queryKey: ["projects"] });
      onClose();
    },
  });
  const runtime = detect.data?.profiles.find((p) => p.id === profile)?.runtime;
  const choices = (installed.data || []).filter(
    (i) =>
      i.runtime !== "caddy" &&
      (i.runtime !== "apache" || i.platform.startsWith("windows-")) &&
      (mode === "create" || i.runtime === runtime),
  );
  const title = tr ? "Proje hazırla" : "Set up a project";
  return (
    <div className="modal-backdrop">
      <form
        className="modal project-wizard"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <h2>{title}</h2>
        <div className="wizard-modes">
          <Button
            type="button"
            variant={mode === "create" ? "primary" : undefined}
            disabled={save.isPending}
            onClick={() => {
              setMode("create");
              detect.reset();
            }}
          >
            {tr ? "Yeni proje" : "New project"}
          </Button>
          <Button
            type="button"
            variant={mode === "import" ? "primary" : undefined}
            disabled={save.isPending}
            onClick={() => {
              setMode("import");
              detect.reset();
            }}
          >
            {tr ? "Klasörden içe aktar" : "Import folder"}
          </Button>
        </div>
        <label htmlFor="project-name">
          {tr ? "Proje adı" : "Project name"}
        </label>
        <input
          id="project-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          maxLength={200}
          required
          disabled={save.isPending}
        />
        <label htmlFor="project-id">
          {tr
            ? "Proje kimliği (harf, rakam, - ve _)"
            : "Project ID (letters, numbers, - and _)"}
        </label>
        <input
          id="project-id"
          value={id}
          onChange={(e) => setId(e.target.value)}
          pattern="[A-Za-z0-9_-]{1,64}"
          required
          disabled={save.isPending}
        />
        <label htmlFor="project-root">
          {tr ? "Proje klasörünün tam yolu" : "Absolute project folder path"}
        </label>
        <input
          id="project-root"
          value={root}
          onChange={(e) => {
            setRoot(e.target.value);
            detect.reset();
            setProfile("");
          }}
          required
          disabled={save.isPending}
        />
        <p>
          {mode === "create"
            ? tr
              ? "Yeni klasör oluşturulur. Üst klasör mevcut olmalı; hedef klasör mevcut olmamalı."
              : "Creates a new folder. The parent must exist; the target folder must not exist."
            : tr
              ? "Kaynak dosyalar değiştirilmez. Bağımlılık kurulumu ayrıca yapılır."
              : "Source files stay unchanged. Dependencies must be installed separately."}
        </p>
        {mode === "import" && (
          <>
            <Button
              type="button"
              disabled={!root || detect.isPending || save.isPending}
              onClick={() => detect.mutate()}
            >
              {tr ? "Proje türünü algıla" : "Detect project type"}
            </Button>
            {detect.data && (
              <>
                <label htmlFor="launch-profile">
                  {tr ? "Başlatma profili" : "Launch profile"}
                </label>
                <select
                  id="launch-profile"
                  value={profile}
                  onChange={(e) => {
                    setProfile(e.target.value);
                    setInstallation("");
                  }}
                  required
                >
                  <option value="">
                    {tr ? "Profil seç" : "Choose a profile"}
                  </option>
                  {detect.data.profiles.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.runtime} · {p.evidence}
                    </option>
                  ))}
                </select>
                {!detect.data.profiles.length && (
                  <p>
                    {tr
                      ? "Desteklenen giriş dosyası bulunamadı. Özel başlatma için manifest içe aktarabilirsin."
                      : "No supported entry point found. Import a manifest for a custom launch command."}
                  </p>
                )}
              </>
            )}
          </>
        )}
        <label htmlFor="project-runtime">
          {tr ? "Çalışma zamanı ve sürüm" : "Runtime and version"}
        </label>
        <select
          id="project-runtime"
          value={installation}
          onChange={(e) => setInstallation(e.target.value)}
          required
          disabled={save.isPending}
        >
          <option value="">
            {tr ? "Kurulu sürüm seç" : "Choose an installed version"}
          </option>
          {choices.map((i) => (
            <option key={i.id} value={i.id}>
              {i.runtime} {i.version} ·{" "}
              {i.id.startsWith("system:")
                ? tr
                  ? "sistem"
                  : "system"
                : tr
                  ? "Dockon yönetimi"
                  : "Dockon managed"}
            </option>
          ))}
        </select>
        {installed.isSuccess && !installed.data.length && (
          <p>
            {tr
              ? "Önce Çalışma ortamları sayfasından bir sürüm kur veya sistem aracını seç."
              : "Install a runtime or select a system tool on the Runtimes page first."}
          </p>
        )}
        <p>
          {tr
            ? "Proje kayıt sonrası durdurulmuş kalır. Başlatma işlemi yalnızca sen istediğinde yapılır."
            : "The registered project stays stopped until you start it."}
        </p>
        {[installed.error, detect.error, save.error]
          .filter(Boolean)
          .map((e, i) => (
            <p role="alert" className="error" key={i}>
              {String(e)}
            </p>
          ))}
        <div>
          <Button type="button" onClick={onClose} disabled={save.isPending}>
            {tr ? "Vazgeç" : "Cancel"}
          </Button>
          <Button
            variant="primary"
            disabled={
              save.isPending || !installation || (mode === "import" && !profile)
            }
          >
            {save.isPending
              ? tr
                ? "Hazırlanıyor…"
                : "Preparing…"
              : mode === "create"
                ? tr
                  ? "Projeyi oluştur"
                  : "Create project"
                : tr
                  ? "Projeyi kaydet"
                  : "Register project"}
          </Button>
        </div>
      </form>
    </div>
  );
}
