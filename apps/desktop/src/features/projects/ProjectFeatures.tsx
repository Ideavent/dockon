import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";

export type Task =
  | "dependencies"
  | "laravel_queue"
  | "node_dev"
  | "tests"
  | "flutter_device"
  | "flutter_emulator";
export type ToolVersion = { runtime: string; version: string };
export type Preset = {
  schema_version: 1;
  name: string;
  runtime: ToolVersion | null;
  php_cgi: ToolVersion | null;
  composer: ToolVersion | null;
  java: ToolVersion | null;
  android_tools: ToolVersion | null;
  database: string | null;
  mail: boolean;
  https: boolean;
  tasks: Task[];
};
export type SavedPreset = { id: string; preset: Preset };

type ActiveTask = {
  task: Task;
  running: boolean;
  exit_code: number | null;
  port: number | null;
};
type FlutterTarget = { id: string; name: string; emulator: boolean };
type Check = {
  code: string;
  ok: boolean;
  detail: string;
  action: "versions" | "tasks" | "source";
};
export type SnapshotProject = {
  manifest: { id: string; name: string; services?: { id: string }[] };
  services?: { id: string; state: string }[];
};

export async function waitForTask(id: string, task: Task) {
  for (let attempt = 0; attempt < 1200; attempt++) {
    const current = (await request<ActiveTask[]>("project_tasks", { id })).find(
      (entry) => entry.task === task,
    );
    if (current && !current.running) {
      if (current.exit_code !== 0)
        throw new Error("Task failed. Check the project logs.");
      return current;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error("Task is still running. Check the project logs.");
}

const taskNames: Record<Task, [string, string]> = {
  dependencies: [
    "Bağ\u0131ml\u0131l\u0131klar\u0131 kur",
    "Install dependencies",
  ],
  laravel_queue: ["Laravel kuyruk i\u015fçisi", "Laravel queue worker"],
  node_dev: ["Vite geli\u015ftirme sunucusu", "Vite development server"],
  tests: ["Testleri çal\u0131\u015ft\u0131r", "Run tests"],
  flutter_device: ["Flutter cihaz oturumu", "Flutter device session"],
  flutter_emulator: ["Android emülatörü", "Android emulator"],
};

export function ProjectTasksPanel({
  id,
  tr,
  runtime,
  php,
  running,
  favorites,
  openLogs,
  startWeb,
  openWeb,
}: {
  id: string;
  tr: boolean;
  runtime?: string;
  php: boolean;
  running: boolean;
  favorites: Task[];
  openLogs: () => void;
  startWeb: () => void;
  openWeb: () => void;
}) {
  const cache = useQueryClient();
  const [target, setTarget] = useState("web");
  const [avd, setAvd] = useState("");
  const state = useQuery({
    queryKey: ["project-tasks", id],
    queryFn: () => request<ActiveTask[]>("project_tasks", { id }),
    refetchInterval: 900,
  });
  const targets = useMutation({
    mutationFn: () => request<FlutterTarget[]>("flutter_targets", { id }),
  });
  const emulators = useMutation({
    mutationFn: () => request<string[]>("flutter_emulators", { id }),
  });
  const taskControl = useMutation({
    mutationFn: async ({ task, stop }: { task: Task; stop: boolean }) => {
      if (stop) return request("project_task_stop", { id, task });
      const operation = await request<Operation>("project_task_run", {
        id,
        task,
        ...(task === "flutter_device" ? { device: target } : {}),
        ...(task === "flutter_emulator" ? { device: avd } : {}),
      });
      return waitOperation(operation.id);
    },
    onSettled: () =>
      cache.invalidateQueries({ queryKey: ["project-tasks", id] }),
  });
  const pin = useMutation({
    mutationFn: (task: Task) =>
      request("project_tasks_set", {
        id,
        tasks: favorites.includes(task)
          ? favorites.filter((item) => item !== task)
          : [...favorites, task],
      }),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["projects"] }),
  });
  const hasDeps = php || ["node", "python", "flutter"].includes(runtime ?? "");
  const offered: Task[] = [
    ...(hasDeps ? (["dependencies"] as const) : []),
    ...(php ? (["laravel_queue"] as const) : []),
    ...(runtime === "node" ? (["node_dev"] as const) : []),
    ...(runtime ? (["tests"] as const) : []),
  ];
  offered.sort(
    (a, b) => Number(favorites.includes(b)) - Number(favorites.includes(a)),
  );
  const deviceSession = state.data?.find(
    (entry) => entry.task === "flutter_device",
  );
  const emulatorSession = state.data?.find(
    (entry) => entry.task === "flutter_emulator",
  );
  return (
    <section className="router-plan">
      {runtime === "flutter" && (
        <div className="project-location">
          <label>
            {tr ? "Flutter hedefi" : "Flutter target"}
            <select
              value={target}
              onChange={(e) => setTarget(e.target.value)}
              disabled={deviceSession?.running}
            >
              <option value="web">Web</option>
              {targets.data?.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}
                  {device.emulator ? " · Emulator" : ""}
                </option>
              ))}
            </select>
          </label>
          <div className="heading-actions">
            <Button
              type="button"
              disabled={targets.isPending}
              onClick={() => targets.mutate()}
            >
              {targets.isPending
                ? tr
                  ? "Aran\u0131yor…"
                  : "Discovering…"
                : tr
                  ? "Android cihazlar\u0131 bul"
                  : "Find Android devices"}
            </Button>
            {deviceSession?.running ? (
              <Button
                type="button"
                onClick={() =>
                  taskControl.mutate({ task: "flutter_device", stop: true })
                }
              >
                {tr ? "Cihaz oturumunu durdur" : "Stop device session"}
              </Button>
            ) : (
              <Button
                type="button"
                disabled={
                  taskControl.isPending ||
                  (target !== "web" &&
                    !targets.data?.some((item) => item.id === target))
                }
                onClick={() =>
                  target === "web"
                    ? running
                      ? openWeb()
                      : startWeb()
                    : taskControl.mutate({
                        task: "flutter_device",
                        stop: false,
                      })
                }
              >
                {target === "web" && running
                  ? tr
                    ? "Web önizlemesini aç"
                    : "Open web preview"
                  : tr
                    ? "Hedefi ba\u015flat"
                    : "Start target"}
              </Button>
            )}
          </div>
          {targets.isSuccess && targets.data.length === 0 && (
            <small>
              {tr
                ? "Bağl\u0131 Android cihaz veya çal\u0131\u015fan emülatör bulunamad\u0131."
                : "Connect an Android device or start an emulator."}
            </small>
          )}
          <div className="heading-actions">
            <Button
              type="button"
              variant="secondary"
              disabled={emulators.isPending}
              onClick={() => emulators.mutate()}
            >
              {tr ? "Kurulu emülatörleri bul" : "Find installed emulators"}
            </Button>
            {emulators.data && (
              <select
                aria-label={tr ? "Android emülatörü" : "Android emulator"}
                value={avd}
                disabled={emulatorSession?.running}
                onChange={(event) => setAvd(event.target.value)}
              >
                <option value="">
                  {tr ? "Emülatör seç" : "Choose emulator"}
                </option>
                {emulators.data.map((id) => (
                  <option key={id}>{id}</option>
                ))}
              </select>
            )}
            {(avd || emulatorSession?.running) && (
              <Button
                type="button"
                disabled={taskControl.isPending}
                onClick={() =>
                  taskControl.mutate({
                    task: "flutter_emulator",
                    stop: Boolean(emulatorSession?.running),
                  })
                }
              >
                {emulatorSession?.running
                  ? tr
                    ? "Emülatörü durdur"
                    : "Stop emulator"
                  : tr
                    ? "Emülatörü aç"
                    : "Boot emulator"}
              </Button>
            )}
          </div>
          {emulatorSession?.running && (
            <small>
              {tr
                ? "Emülatör hazır olduğunda Android cihazları yenile."
                : "Refresh Android devices when the emulator finishes booting."}
            </small>
          )}
        </div>
      )}
      {offered.map((task) => {
        const current = state.data?.find((entry) => entry.task === task);
        const unavailable =
          !running && (task === "laravel_queue" || task === "node_dev");
        return (
          <div className="project-location" key={task}>
            <strong>{taskNames[task][tr ? 0 : 1]}</strong>
            {current?.running && (
              <small>
                {tr ? "Çal\u0131\u015f\u0131yor" : "Running"}{" "}
                {current.port ? <code>127.0.0.1:{current.port}</code> : null}
              </small>
            )}
            {!current?.running && current?.exit_code === 0 && (
              <small>{tr ? "Tamamland\u0131" : "Completed"}</small>
            )}
            {!current?.running &&
              current?.exit_code != null &&
              current.exit_code > 0 && (
                <small>
                  {tr ? "Günlükleri kontrol et" : "Check the logs"} (
                  {current.exit_code})
                </small>
              )}
            <div className="heading-actions">
              <Button
                type="button"
                disabled={unavailable || taskControl.isPending}
                onClick={() =>
                  taskControl.mutate({ task, stop: !!current?.running })
                }
              >
                {current?.running
                  ? tr
                    ? "Durdur"
                    : "Stop"
                  : tr
                    ? "Çal\u0131\u015ft\u0131r"
                    : "Run"}
              </Button>
              <Button
                type="button"
                variant="secondary"
                disabled={pin.isPending}
                aria-pressed={favorites.includes(task)}
                onClick={() => pin.mutate(task)}
              >
                {favorites.includes(task)
                  ? tr
                    ? "Sabitlemeyi kald\u0131r"
                    : "Unpin"
                  : tr
                    ? "Sabitle"
                    : "Pin"}
              </Button>
            </div>
          </div>
        );
      })}
      <Button type="button" variant="secondary" onClick={openLogs}>
        {tr ? "Proje günlükleri" : "Project logs"}
      </Button>
      {(taskControl.error ||
        targets.error ||
        emulators.error ||
        pin.error ||
        state.error) && (
        <p role="alert">
          {
            (
              taskControl.error ||
              targets.error ||
              emulators.error ||
              pin.error ||
              state.error
            )?.message
          }
        </p>
      )}
    </section>
  );
}

const checkNames: Record<string, [string, string]> = {
  source: ["Proje klasörü", "Project directory"],
  runtime: ["Çal\u0131\u015fma zaman\u0131", "Runtime"],
  tools: ["Atanan araçlar", "Assigned tools"],
  composer: ["Composer sürümü", "Composer version"],
  dependencies: ["PHP bağ\u0131ml\u0131l\u0131klar\u0131", "PHP dependencies"],
  node_dependencies: [
    "Node.js bağ\u0131ml\u0131l\u0131klar\u0131",
    "Node.js dependencies",
  ],
  jdk: ["Android için Java JDK", "Java JDK for Android"],
  android: ["Android SDK platformu", "Android SDK platform"],
  php_cgi: ["PHP FastCGI", "PHP FastCGI"],
  php_extensions: ["PHP eklentileri", "PHP extensions"],
};

export function ProjectCheckPanel({
  id,
  tr,
  onAction,
}: {
  id: string;
  tr: boolean;
  onAction: (action: Check["action"]) => void;
}) {
  const checks = useQuery({
    queryKey: ["project-check", id],
    queryFn: () => request<Check[]>("project_check", { id }),
    retry: false,
  });
  return (
    <section className="router-plan">
      <Button
        type="button"
        disabled={checks.isFetching}
        onClick={() => checks.refetch()}
      >
        {checks.isFetching
          ? tr
            ? "Kontrol ediliyor…"
            : "Checking…"
          : tr
            ? "Yeniden kontrol et"
            : "Check again"}
      </Button>
      {checks.data?.map((check) => (
        <div className="project-location" key={check.code}>
          <strong>{checkNames[check.code]?.[tr ? 0 : 1] ?? check.code}</strong>
          <small>
            {check.ok
              ? tr
                ? "Haz\u0131r"
                : "Ready"
              : tr
                ? "İ\u015flem gerekli"
                : "Action needed"}
          </small>
          {check.detail && <code>{check.detail}</code>}
          {!check.ok && check.action !== "source" && (
            <Button
              type="button"
              variant="secondary"
              onClick={() => onAction(check.action)}
            >
              {check.action === "tasks"
                ? tr
                  ? "Görevleri aç"
                  : "Open tasks"
                : tr
                  ? "Sürüm ve araç ayarlar\u0131"
                  : "Runtime and tools settings"}
            </Button>
          )}
        </div>
      ))}
      {checks.error && <p role="alert">{checks.error.message}</p>}
    </section>
  );
}

export function ProjectSharePanel({
  id,
  name,
  tr,
}: {
  id: string;
  name: string;
  tr: boolean;
}) {
  const [title, setTitle] = useState(name);
  const cache = useQueryClient();
  const presets = useQuery({
    queryKey: ["project-presets"],
    queryFn: () => request<SavedPreset[]>("project_presets"),
  });
  const share = useMutation({
    mutationFn: async (kind: "save" | "export") => {
      if (kind === "export")
        return invoke<string>("export_project_preset", {
          id,
          name: title.trim(),
        });
      const preset = await request<Preset>("project_preset_capture", {
        id,
        name: title.trim(),
      });
      await request("project_preset_save", { preset });
      return null;
    },
    onSuccess: () => cache.invalidateQueries({ queryKey: ["project-presets"] }),
  });
  const remove = useMutation({
    mutationFn: (id: string) => request("project_preset_remove", { id }),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["project-presets"] }),
  });
  return (
    <section className="router-plan">
      <label>
        {tr ? "Ön ayar ad\u0131" : "Preset name"}{" "}
        <input
          value={title}
          maxLength={80}
          onChange={(e) => setTitle(e.target.value)}
        />
      </label>
      <div className="heading-actions">
        <Button
          type="button"
          disabled={!title.trim() || share.isPending}
          onClick={() => share.mutate("save")}
        >
          {tr ? "Ön ayar olarak kaydet" : "Save as preset"}
        </Button>
        <Button
          type="button"
          disabled={!title.trim() || share.isPending}
          onClick={() => share.mutate("export")}
        >
          {tr ? "dockon.json d\u0131\u015fa aktar" : "Export dockon.json"}
        </Button>
      </div>
      {share.isSuccess && (
        <p role="status">
          {share.data ?? (tr ? "Ön ayar kaydedildi." : "Preset saved.")}
        </p>
      )}
      {presets.data?.map(({ id, preset }) => (
        <div className="heading-actions" key={id}>
          <span>{preset.name}</span>
          <Button
            type="button"
            variant="secondary"
            disabled={remove.isPending}
            onClick={() => remove.mutate(id)}
          >
            {tr ? "Ön ayar\u0131 sil" : "Remove preset"}
          </Button>
        </div>
      ))}
      {(share.error || remove.error || presets.error) && (
        <p role="alert">
          {(share.error || remove.error || presets.error)?.message}
        </p>
      )}
    </section>
  );
}

export function DatabaseSnapshots({
  id,
  engine,
  tr,
  running,
  projects,
}: {
  id: string;
  engine: string;
  tr: boolean;
  running: boolean;
  projects: SnapshotProject[];
}) {
  const cache = useQueryClient();
  const [backup, setBackup] = useState("");
  const [target, setTarget] = useState(id);
  const snapshots = useQuery({
    queryKey: ["database-snapshots", id, engine],
    queryFn: () => request<string[]>("database_snapshots", { id, engine }),
  });
  const eligible = projects.filter((project) =>
    project.services?.some(
      (service) => service.id === engine && service.state === "RUNNING",
    ),
  );
  const mutation = useMutation({
    mutationFn: async (copy: boolean) => {
      const result = await request<Operation>(
        copy ? "database_snapshot_copy" : "database_snapshot",
        { id, engine, ...(copy ? { backup, target } : {}) },
      );
      await waitOperation(result.id);
      return copy ? "dockon_restore_" + result.id.replaceAll("-", "") : null;
    },
    onSuccess: () =>
      cache.invalidateQueries({ queryKey: ["database-snapshots", id, engine] }),
  });
  return (
    <div className="project-location">
      <Button
        type="button"
        disabled={!running || mutation.isPending}
        onClick={() => mutation.mutate(false)}
      >
        {tr ? "Anl\u0131k görüntü al" : "Take snapshot"}
      </Button>
      {!!snapshots.data?.length && (
        <>
          <label>
            {tr ? "Anl\u0131k görüntü" : "Snapshot"}
            <select value={backup} onChange={(e) => setBackup(e.target.value)}>
              <option value="">{tr ? "Yedek seç" : "Select snapshot"}</option>
              {snapshots.data.map((file) => (
                <option key={file}>{file}</option>
              ))}
            </select>
          </label>
          <label>
            {tr ? "Hedef proje" : "Target project"}
            <select value={target} onChange={(e) => setTarget(e.target.value)}>
              {!eligible.some((project) => project.manifest.id === id) && (
                <option value={id} disabled>
                  {tr
                    ? "Bu projenin veritaban\u0131 duruyor"
                    : "This project's database is stopped"}
                </option>
              )}
              {eligible.map((project) => (
                <option key={project.manifest.id} value={project.manifest.id}>
                  {project.manifest.name}
                </option>
              ))}
            </select>
          </label>
          <Button
            type="button"
            disabled={
              !backup ||
              !eligible.some((project) => project.manifest.id === target) ||
              mutation.isPending
            }
            onClick={() => mutation.mutate(true)}
          >
            {tr
              ? "Yeni veritaban\u0131na geri yükle"
              : "Restore to a new database"}
          </Button>
        </>
      )}
      {mutation.isPending && (
        <p role="status">{tr ? "İ\u015fleniyor…" : "Working…"}</p>
      )}
      {mutation.isSuccess && (
        <p role="status">
          {mutation.data ??
            (tr ? "Anl\u0131k görüntü kaydedildi." : "Snapshot saved.")}
        </p>
      )}
      {(mutation.error || snapshots.error) && (
        <p role="alert">{(mutation.error || snapshots.error)?.message}</p>
      )}
    </div>
  );
}
