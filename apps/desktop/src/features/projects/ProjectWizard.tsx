import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";
import { InstallVersion } from "../runtimes/InstallVersion";
import { runtimeNames } from "../runtimes/RuntimePage";
import {
  waitForTask,
  type Preset,
  type SavedPreset,
  type ToolVersion,
} from "./ProjectFeatures";
import type { RouterStatus } from "../routing/RouterPage";

const templates: Record<string, [string, string][]> = {
  php: [
    ["laravel", "Laravel"],
    ["codeigniter", "CodeIgniter"],
    ["cakephp", "CakePHP"],
    ["symfony", "Symfony"],
  ],
  node: [
    ["vite-react", "React / Vite"],
    ["vite-vue", "Vue / Vite"],
    ["vite-svelte", "Svelte / Vite"],
  ],
  python: [
    ["django", "Django"],
    ["flask", "Flask"],
  ],
  java: [["spring-boot", "Spring Boot"]],
  flutter: [["flutter", "Flutter"]],
};
const projectKinds = [
  "php",
  "node",
  "python",
  "java",
  "flutter",
  "dotnet",
  "go",
  "rust",
];

type Installation = {
  id: string;
  runtime: string;
  version: string;
  platform: string;
};
type Detection = {
  root: string;
  profiles: { id: string; runtime: string; evidence: string }[];
  versions?: Record<string, string>;
  dependency_files?: string[];
  preset?: Preset | null;
};
function recommend(choices: Installation[], family: string, hint?: string) {
  const sorted = choices
    .filter((item) => item.runtime === family)
    .sort((a, b) =>
      b.version.localeCompare(a.version, undefined, { numeric: true }),
    );
  if (!hint) return sorted[0]?.id ?? "";
  const simple = /^(v|~|\^|>=|==)?\s*(\d+(?:\.\d+){0,2})(?:\.x)?$/.exec(
    hint.trim(),
  );
  if (!simple) return "";
  const [, rule, version] = simple;
  if (rule === ">=")
    return (
      sorted.find(
        (item) =>
          item.version.localeCompare(version, undefined, { numeric: true }) >=
          0,
      )?.id ?? ""
    );
  const prefix =
    rule === "^"
      ? version.split(".")[0]
      : rule === "~"
        ? version.split(".").slice(0, 2).join(".")
        : version;
  return (
    sorted.find(
      (item) =>
        item.version === prefix || item.version.startsWith(prefix + "."),
    )?.id ?? ""
  );
}

export function ProjectWizard({
  lang,
  onClose,
  folder,
  workspaceRoot,
}: {
  lang: "tr" | "en";
  onClose: () => void;
  folder?: { name: string; path: string };
  workspaceRoot?: string;
}) {
  const tr = lang === "tr";
  const [mode, setMode] = useState<"create" | "import">(
    folder ? "import" : "create",
  );
  const [gitMode, setGitMode] = useState(false);
  const [gitUrl, setGitUrl] = useState("");
  const [installDependencies, setInstallDependencies] = useState(false);
  const [startAfterImport, setStartAfterImport] = useState(false);
  const [chosenPreset, setChosenPreset] = useState<SavedPreset>();
  const [created, setCreated] = useState(false);
  const [id, setId] = useState(
    (folder?.name || "")
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .slice(0, 63),
  );
  const [name, setName] = useState(folder?.name || "");
  const [root, setRoot] = useState(folder?.path || "");
  const [installation, setInstallation] = useState("");
  const [profile, setProfile] = useState("");
  const [phpCgi, setPhpCgi] = useState("");
  const [kind, setKind] = useState("application");
  const [webServer, setWebServer] = useState("apache");
  const [database, setDatabase] = useState("");
  const [mail, setMail] = useState(false);
  const [template, setTemplate] = useState("");
  const [composer, setComposer] = useState("");
  const [java, setJava] = useState("");
  const [androidSdk, setAndroidSdk] = useState("");
  const [androidTools, setAndroidTools] = useState("");
  const creatingPhp = mode === "create" && kind === "php";
  const usesFastCgi = creatingPhp
    ? webServer !== "php"
    : mode === "import" && ["apache:php", "nginx:php"].includes(profile);
  const cache = useQueryClient();
  const router = useQuery({
    queryKey: ["router"],
    queryFn: () => request<RouterStatus>("router_status"),
  });
  const suffix = router.data?.settings?.domain_suffix ?? "test";
  const installed = useQuery({
    queryKey: ["runtimes"],
    queryFn: () => request<Installation[]>("runtime_installed"),
  });
  const presets = useQuery({
    queryKey: ["project-presets"],
    queryFn: () => request<SavedPreset[]>("project_presets"),
  });
  const resolveVersion = (ref?: ToolVersion | null) =>
    installed.data?.find(
      (item) => item.runtime === ref?.runtime && item.version === ref.version,
    )?.id || "";
  const applyPreset = (entry?: SavedPreset) => {
    setChosenPreset(entry);
    if (!entry) return;
    const preset = entry.preset;
    setDatabase(preset.database ?? "");
    setMail(preset.mail);
    setComposer(resolveVersion(preset.composer));
    setJava(resolveVersion(preset.java));
    setAndroidTools(resolveVersion(preset.android_tools));
    setPhpCgi(resolveVersion(preset.php_cgi));
    const runtime = preset.runtime?.runtime;
    if (mode === "create" && runtime) {
      const target =
        preset.php_cgi || runtime === "php"
          ? "php"
          : projectKinds.includes(runtime)
            ? runtime
            : "application";
      setKind(target);
      setTemplate(target === "flutter" ? "flutter" : "");
      if (target === "php") setWebServer(runtime);
    }
    if (mode === "import" && runtime) {
      const match = detect.data?.profiles.find(
        (item) => item.runtime === runtime,
      );
      if (match) setProfile(match.id);
    }
    setInstallation(resolveVersion(preset.runtime));
  };
  const loadPreset = useMutation({
    mutationFn: async (file: File) => {
      if (file.size > 32768) throw new Error("The Dockon preset is too large.");
      const preset: Preset = JSON.parse(await file.text());
      return request<SavedPreset>("project_preset_save", { preset });
    },
    onSuccess: (entry) => {
      void cache.invalidateQueries({ queryKey: ["project-presets"] });
      applyPreset(entry);
    },
  });
  const detect = useMutation({
    mutationFn: (path: string) =>
      request<Detection>("project_detect", { root: path }),
    onSuccess: (result) => {
      const preferredFamily = chosenPreset?.preset.runtime?.runtime;
      const next =
        result.profiles.find((item) => item.runtime === preferredFamily) ??
        result.profiles[0];
      setProfile(next?.id ?? "");
      setInstallation(
        next
          ? resolveVersion(
              chosenPreset?.preset.runtime?.runtime === next.runtime
                ? chosenPreset?.preset.runtime
                : null,
            ) ||
              recommend(
                installed.data ?? [],
                next.runtime,
                result.versions?.[next.runtime],
              )
          : "",
      );
    },
  });
  const clone = useMutation({
    mutationFn: async () => {
      if (!workspaceRoot)
        throw new Error("The workspace directory is unavailable.");
      const url = new URL(gitUrl);
      const base =
        name.trim() ||
        url.pathname
          .split("/")
          .filter(Boolean)
          .at(-1)
          ?.replace(/\.git$/, "") ||
        "";
      const slug = base
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .slice(0, 63)
        .replace(/^-+|-+$/g, "");
      if (!slug) throw new Error("Enter a valid project name.");
      const operation = await request<Operation>("workspace_git_clone", {
        slug,
        url: gitUrl.trim(),
      });
      await waitOperation(operation.id);
      return { slug, name: base, root: workspaceRoot + "/" + slug };
    },
    onSuccess: (result) => {
      setId(result.slug);
      setName(result.name);
      setRoot(result.root);
      setGitMode(false);
      detect.mutate(result.root);
      void cache.invalidateQueries({ queryKey: ["workspace-folders"] });
    },
  });
  useEffect(() => {
    if (folder) detect.mutate(folder.path);
  }, [folder]);
  const save = useMutation({
    mutationFn: async () => {
      const components = {
        database: database || undefined,
        mail,
        php_cgi_installation: usesFastCgi ? phpCgi : undefined,
      };
      const setup = {
        ...(mode === "create" ? { template: template || undefined } : {}),
        tools: {
          composer: phpContext ? composer || undefined : undefined,
          java: flutterContext ? java || undefined : undefined,
          android_tools: flutterContext ? androidTools || undefined : undefined,
          android_sdk: flutterContext
            ? androidSdk.trim() || undefined
            : undefined,
        },
      };
      const method =
        mode === "create"
          ? workspaceRoot
            ? "workspace_create"
            : "project_create"
          : "project_import";
      const params =
        method === "workspace_create"
          ? {
              slug,
              name: name.trim(),
              runtime_installation: installation,
              ...components,
              ...setup,
            }
          : {
              id,
              name,
              root,
              runtime_installation: installation,
              ...components,
              ...setup,
              ...(mode === "import" ? { profile } : {}),
            };
      if (mode === "create" && template) {
        await waitOperation((await request<Operation>(method, params)).id);
      } else {
        await request(method, params);
      }
      setCreated(true);
      const projectId = method === "workspace_create" ? slug : id;
      await cache.invalidateQueries({ queryKey: ["projects"] });
      if (chosenPreset) {
        await request("project_tasks_set", {
          id: projectId,
          tasks: chosenPreset.preset.tasks,
        });
        await request("workspace_route", {
          id: projectId,
          https: chosenPreset.preset.https,
        });
      }
      if (mode === "import" && installDependencies) {
        const task = await request<Operation>("project_task_run", {
          id: projectId,
          task: "dependencies",
        });
        await waitOperation(task.id);
        await waitForTask(projectId, "dependencies");
      }
      if (mode === "import" && startAfterImport) {
        await waitOperation(
          (await request<Operation>("start", { id: projectId })).id,
        );
        await invoke("open_project", { id: projectId });
      }
    },
    onSuccess: () => {
      cache.invalidateQueries({ queryKey: ["projects"] });
      onClose();
    },
  });
  const managedCreate = mode === "create" && Boolean(workspaceRoot);
  const slug = name
    .trim()
    .toLocaleLowerCase("tr")
    .replace(/ı/g, "i")
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/[^a-z0-9]+/g, "-")
    .slice(0, 63)
    .replace(/^-+|-+$/g, "");
  const runtime = detect.data?.profiles.find((p) => p.id === profile)?.runtime;
  const phpContext =
    creatingPhp || (mode === "import" && (runtime === "php" || usesFastCgi));
  const flutterContext =
    mode === "create" ? kind === "flutter" : runtime === "flutter";
  const missing = chosenPreset
    ? (["runtime", "php_cgi", "composer", "java", "android_tools"] as const)
        .map((key) => chosenPreset.preset[key])
        .filter((version) => version && !resolveVersion(version))
    : [];
  const choices = (installed.data || []).filter(
    (i) =>
      ![
        "caddy",
        "php-cgi",
        "redis",
        "memcached",
        "composer",
        "android-tools",
      ].includes(i.runtime) &&
      (i.runtime !== "apache" || i.platform.startsWith("windows-")) &&
      (mode === "import"
        ? i.runtime === runtime
        : creatingPhp
          ? i.runtime === webServer
          : kind === "application"
            ? !["php", "flutter"].includes(i.runtime)
            : i.runtime === kind),
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
            aria-pressed={mode === "create"}
            disabled={save.isPending}
            onClick={() => {
              setMode("create");
              setGitMode(false);
              detect.reset();
            }}
          >
            {tr ? "Yeni proje" : "New project"}
          </Button>
          <Button
            type="button"
            aria-pressed={mode === "import" && !gitMode}
            disabled={save.isPending}
            onClick={() => {
              setMode("import");
              setGitMode(false);
              detect.reset();
            }}
          >
            {tr ? "Klasörden içe aktar" : "Import folder"}
          </Button>
          <Button
            type="button"
            aria-pressed={gitMode}
            disabled={save.isPending || clone.isPending}
            onClick={() => {
              setMode("import");
              setGitMode(true);
              setRoot("");
              detect.reset();
            }}
          >
            {tr ? "Git’ten al" : "Import from Git"}
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
        {gitMode && (
          <div className="project-location">
            <label>
              {tr ? "HTTPS Git deposu" : "HTTPS Git repository"}
              <input
                type="url"
                value={gitUrl}
                onChange={(event) => setGitUrl(event.target.value)}
                disabled={clone.isPending}
                placeholder="https://github.com/team/project.git"
              />
            </label>
            <Button
              type="button"
              disabled={!gitUrl.trim() || clone.isPending || !workspaceRoot}
              onClick={() => clone.mutate()}
            >
              {clone.isPending
                ? tr
                  ? "İndiriliyor…"
                  : "Cloning…"
                : tr
                  ? "Depoyu indir"
                  : "Clone repository"}
            </Button>
          </div>
        )}
        {!gitMode && (
          <>
            <label>
              {tr ? "Ön ayar" : "Preset"}
              <select
                value={chosenPreset?.id ?? ""}
                onChange={(event) =>
                  applyPreset(
                    presets.data?.find(
                      (entry) => entry.id === event.target.value,
                    ),
                  )
                }
              >
                <option value="">
                  {tr ? "Ön ayar kullanma" : "No preset"}
                </option>
                {presets.data?.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.preset.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {tr ? "dockon.json içe aktar" : "Import dockon.json"}
              <input
                type="file"
                accept=".json,application/json"
                disabled={loadPreset.isPending || save.isPending}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (file) loadPreset.mutate(file);
                  event.target.value = "";
                }}
              />
            </label>
            {detect.data?.preset && (
              <Button
                type="button"
                disabled={save.isPending}
                onClick={() => {
                  if (detect.data?.preset)
                    applyPreset({
                      id: "repository",
                      preset: detect.data.preset,
                    });
                }}
              >
                {tr
                  ? "Depodaki dockon.json ayarlarını kullan"
                  : "Use dockon.json from repository"}
              </Button>
            )}
            {!!missing.length && (
              <p role="status">
                {tr
                  ? "Kurulu olmayan ön ayar sürümleri"
                  : "Preset versions to install"}{" "}
                {missing
                  .map((item) => `${item!.runtime} ${item!.version}`)
                  .join(", ")}
              </p>
            )}
          </>
        )}
        {gitMode ? null : managedCreate ? (
          <div className="project-location">
            <span>{tr ? "Proje klasörü" : "Project folder"}</span>
            <code>
              {workspaceRoot}\{slug || "…"}
            </code>
            <span>{tr ? "Yerel adres" : "Local address"}</span>
            <code>{slug ? `${slug}.${suffix}` : "…"}</code>
          </div>
        ) : (
          <>
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
              {tr
                ? "Proje klasörünün tam yolu"
                : "Absolute project folder path"}
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
          </>
        )}
        {mode === "import" && (
          <>
            <Button
              type="button"
              disabled={!root || detect.isPending || save.isPending}
              onClick={() => detect.mutate(root)}
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
                      {p.runtime} · {p.id.endsWith(":php") ? "FastCGI · " : ""}
                      {p.evidence}
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
        {mode === "create" && (
          <>
            <label htmlFor="project-kind">
              {tr ? "Proje türü" : "Project type"}
            </label>
            <select
              id="project-kind"
              value={kind}
              disabled={save.isPending}
              onChange={(event) => {
                setKind(event.target.value);
                setTemplate(event.target.value === "flutter" ? "flutter" : "");
                setInstallation("");
              }}
            >
              <option value="application">
                {tr
                  ? "Uygulama veya statik site"
                  : "Application or static site"}
              </option>
              {projectKinds.map((kind) => (
                <option key={kind} value={kind}>
                  {runtimeNames[kind]}
                </option>
              ))}
            </select>
          </>
        )}
        {mode === "create" && templates[kind] && (
          <label>
            {tr ? "Proje şablonu" : "Project template"}
            <select
              value={template}
              onChange={(event) => setTemplate(event.target.value)}
              disabled={save.isPending}
            >
              {kind !== "flutter" && (
                <option value="">{tr ? "Boş proje" : "Blank project"}</option>
              )}
              {templates[kind].map(([id, label]) => (
                <option value={id} key={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
        )}
        {creatingPhp && (
          <>
            <label htmlFor="project-web-server">
              {tr ? "Web sunucusu" : "Web server"}
            </label>
            <select
              id="project-web-server"
              value={webServer}
              disabled={save.isPending}
              onChange={(event) => {
                setWebServer(event.target.value);
                setInstallation("");
              }}
            >
              <option value="apache">Apache</option>
              <option value="nginx">Nginx</option>
              <option value="php">
                {tr ? "PHP geliştirme sunucusu" : "PHP development server"}
              </option>
            </select>
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
        {mode === "create" && kind !== "application" && kind !== "rust" && (
          <InstallVersion
            key={creatingPhp ? webServer : kind}
            runtime={creatingPhp ? webServer : kind}
            label={runtimeNames[creatingPhp ? webServer : kind]}
            tr={tr}
          />
        )}
        {mode === "import" && !gitMode && runtime && runtime !== "rust" && (
          <InstallVersion
            key={runtime}
            runtime={runtime}
            label={runtimeNames[runtime] ?? runtime}
            tr={tr}
          />
        )}
        {phpContext && (
          <>
            <label>
              Composer
              <select
                value={composer}
                onChange={(event) => setComposer(event.target.value)}
                disabled={save.isPending}
                required={Boolean(template)}
              >
                <option value="">
                  {tr ? "Composer sürümü seç" : "Select a Composer version"}
                </option>
                {installed.data
                  ?.filter((row) => row.runtime === "composer")
                  .map((row) => (
                    <option key={row.id} value={row.id}>
                      {row.version}
                    </option>
                  ))}
              </select>
            </label>
            <InstallVersion runtime="composer" label="Composer" tr={tr} />
          </>
        )}
        {flutterContext && (
          <>
            <label>
              Java JDK
              <select
                value={java}
                onChange={(event) => setJava(event.target.value)}
                disabled={save.isPending}
              >
                <option value="">
                  {tr ? "JDK seç (Android için)" : "Choose a JDK (for Android)"}
                </option>
                {installed.data
                  ?.filter((row) => row.runtime === "java")
                  .map((row) => (
                    <option key={row.id} value={row.id}>
                      {row.version}
                    </option>
                  ))}
              </select>
            </label>
            <label>
              Android SDK Tools
              <select
                value={androidTools}
                onChange={(event) => setAndroidTools(event.target.value)}
                disabled={save.isPending}
              >
                <option value="">
                  {tr ? "Mevcut SDK veya web" : "Existing SDK or web only"}
                </option>
                {installed.data
                  ?.filter((row) => row.runtime === "android-tools")
                  .map((row) => (
                    <option key={row.id} value={row.id}>
                      {row.version}
                    </option>
                  ))}
              </select>
            </label>
            <InstallVersion
              runtime="android-tools"
              label="Android SDK Tools"
              tr={tr}
            />
            <label>
              {tr
                ? "Android SDK klasörü (isteğe bağlı)"
                : "Android SDK directory (optional)"}
              <input
                value={androidSdk}
                onChange={(event) => setAndroidSdk(event.target.value)}
                disabled={save.isPending}
              />
            </label>
            <InstallVersion runtime="java" label="Java JDK" tr={tr} />
          </>
        )}
        {usesFastCgi && (
          <>
            <InstallVersion runtime="php-cgi" label="PHP FastCGI" tr={tr} />
            <label htmlFor="project-php-cgi">PHP CGI</label>
            <select
              id="project-php-cgi"
              value={phpCgi}
              disabled={save.isPending}
              onChange={(event) => setPhpCgi(event.target.value)}
            >
              <option value="">
                {tr ? "Kurulu PHP CGI seç" : "Choose installed PHP CGI"}
              </option>
              {installed.data
                ?.filter((item) => item.runtime === "php-cgi")
                .map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.version}
                  </option>
                ))}
            </select>
            <p>
              {tr
                ? "Bu profil public/index.php giriş noktasını çalıştırır. .htaccess kuralları ayrı yapılandırma gerektirir."
                : "This profile runs public/index.php. .htaccess rules require separate configuration."}
            </p>
          </>
        )}
        <label htmlFor="project-database">
          {tr ? "Veritabanı" : "Database"}
        </label>
        <select
          id="project-database"
          value={database}
          disabled={save.isPending}
          onChange={(event) => setDatabase(event.target.value)}
        >
          <option value="">{tr ? "Veritabanı yok" : "No database"}</option>
          <option value="mysql">MySQL</option>
          <option value="postgres">PostgreSQL</option>
        </select>
        <label className="project-ssl">
          <input
            type="checkbox"
            checked={mail}
            disabled={save.isPending}
            onChange={(event) => setMail(event.target.checked)}
          />
          {tr ? "Proje postasını etkinleştir" : "Enable project mail"}
        </label>
        {database && (
          <p className="muted">
            {tr
              ? "Bu veritabanı proje ile birlikte yerel olarak açılır."
              : "This database runs locally with the project."}
          </p>
        )}
        {installed.isSuccess && !choices.length && (
          <p>
            {tr
              ? "Bu profil için Ayarlar’dan bir sürüm kur veya sistem aracını seç."
              : "Install a runtime or select a system tool on the Runtimes page first."}
          </p>
        )}
        <p>
          {tr
            ? "Proje kayıt sonrası durdurulmuş kalır. Başlatma işlemi yalnızca sen istediğinde yapılır."
            : "The registered project stays stopped until you start it."}
        </p>
        {mode === "import" &&
          !gitMode &&
          !!detect.data?.dependency_files?.length && (
            <div className="project-location">
              {Object.entries(detect.data.versions ?? {}).map(
                ([tool, version]) => (
                  <code key={tool}>
                    {tool} {version}
                  </code>
                ),
              )}
              <code>{detect.data.dependency_files.join(", ")}</code>
              <label className="project-ssl">
                <input
                  type="checkbox"
                  checked={installDependencies}
                  onChange={(event) =>
                    setInstallDependencies(event.target.checked)
                  }
                  disabled={save.isPending}
                />
                {tr
                  ? "Kayıttan sonra bağımlılıkları kur ve depo betiklerine izin ver"
                  : "Install dependencies after import and allow repository scripts"}
              </label>
            </div>
          )}
        {mode === "import" && !gitMode && !!detect.data?.profiles.length && (
          <label className="project-ssl">
            <input
              type="checkbox"
              checked={startAfterImport}
              onChange={(event) => setStartAfterImport(event.target.checked)}
              disabled={save.isPending}
            />
            {tr
              ? "Kurulum bitince projeyi başlat ve tarayıcıda aç"
              : "Start the project and open it in the browser after setup"}
          </label>
        )}
        {created && save.error && (
          <p role="status">
            {tr
              ? "Proje kaydedildi. Kalan işlemleri proje ayarlarından sürdürebilirsin."
              : "Project saved. Continue setup from project settings."}
          </p>
        )}
        {[
          installed.error,
          detect.error,
          save.error,
          clone.error,
          loadPreset.error,
        ]
          .filter(Boolean)
          .map((e, i) => (
            <p role="alert" className="error" key={i}>
              {String(e)}
            </p>
          ))}
        <div className="dialog-actions">
          <Button type="button" onClick={onClose} disabled={save.isPending}>
            {tr ? "Vazgeç" : "Cancel"}
          </Button>
          <Button
            variant="primary"
            disabled={
              save.isPending ||
              clone.isPending ||
              gitMode ||
              created ||
              !installation ||
              (managedCreate && !slug) ||
              (mode === "import" && !profile) ||
              (usesFastCgi && !phpCgi) ||
              (phpContext &&
                ((mode === "create" && Boolean(template)) ||
                  (mode === "import" &&
                    installDependencies &&
                    detect.data?.dependency_files?.includes(
                      "composer.json",
                    ))) &&
                !composer)
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
