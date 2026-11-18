import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AppWindow, AudioWaveform, Power } from "lucide-react";

type CloseBehavior = "ask" | "minimize" | "quit";
type DesktopPreferences = {
  close_behavior: CloseBehavior;
  auto_start: boolean | null;
  auto_start_error: string | null;
};
type Change =
  | { setting: "auto_start"; value: boolean }
  | { setting: "close_behavior"; value: CloseBehavior };
const preferencesKey = ["desktop-preferences"];

export function SystemSettings({
  desktop,
  lang,
  reduceMotion,
  onReduceMotion,
}: {
  desktop: boolean;
  lang: "tr" | "en";
  reduceMotion: boolean;
  onReduceMotion: (reduce: boolean) => void;
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const preferences = useQuery({
    queryKey: preferencesKey,
    queryFn: () => invoke<DesktopPreferences>("desktop_preferences"),
    enabled: desktop,
    retry: false,
  });
  const save = useMutation({
    mutationFn: async (
      change: Change,
    ): Promise<Partial<DesktopPreferences>> => {
      if (change.setting === "auto_start") {
        const auto_start = await invoke<boolean>("set_auto_start", {
          enabled: change.value,
        });
        return { auto_start };
      }
      const close_behavior = await invoke<CloseBehavior>("set_close_behavior", {
        behavior: change.value,
      });
      return { close_behavior };
    },
    onSuccess: (saved) => {
      cache.setQueryData<DesktopPreferences>(
        preferencesKey,
        (current) => current && { ...current, ...saved },
      );
    },
  });
  const current = preferences.data;
  const error = save.error || preferences.error || current?.auto_start_error;

  return (
    <section aria-labelledby="system-heading">
      <div className="page-heading">
        <h1 id="system-heading">
          {tr ? "Sistem ayarları" : "System settings"}
        </h1>
      </div>
      {!desktop && (
        <p className="notice">
          {tr
            ? "Başlangıç ve pencere ayarları masaüstü uygulamasında kullanılabilir."
            : "Startup and window settings are available in the desktop app."}
        </p>
      )}
      {desktop && preferences.isPending && (
        <p className="settings-message" role="status">
          {tr ? "Ayarlar yükleniyor…" : "Loading settings…"}
        </p>
      )}
      {error && (
        <p className="error" role="alert">
          {tr ? "Ayar işlemi başarısız oldu." : "Could not access the setting."}{" "}
          {String(error)}
        </p>
      )}
      <div className="settings-panel">
        <label className="settings-row">
          <span className="settings-icon">
            <Power size={19} aria-hidden="true" />
          </span>
          <span className="settings-copy">
            <strong>{tr ? "Oturum açınca başlat" : "Launch at sign-in"}</strong>
            <span id="auto-start-description">
              {current?.auto_start === null && !current.auto_start_error
                ? tr
                  ? "Bu seçenek Windows ve macOS’te kullanılabilir."
                  : "Available on Windows and macOS."
                : tr
                  ? "Bilgisayarda oturum açtığında Dockon’u başlatır."
                  : "Opens Dockon when you sign in."}
            </span>
          </span>
          <input
            className="settings-switch"
            type="checkbox"
            role="switch"
            aria-label={tr ? "Oturum açınca başlat" : "Launch at sign-in"}
            aria-describedby="auto-start-description"
            checked={current?.auto_start === true}
            disabled={current?.auto_start == null || save.isPending}
            onChange={(event) =>
              save.mutate({
                setting: "auto_start",
                value: event.target.checked,
              })
            }
          />
        </label>
        <div className="settings-row settings-row-select">
          <span className="settings-icon">
            <AppWindow size={19} aria-hidden="true" />
          </span>
          <div className="settings-copy">
            <label htmlFor="close-behavior">
              {tr ? "Kapatma düğmesi" : "Close button"}
            </label>
            <span id="close-behavior-description">
              {tr
                ? "Penceredeki × düğmesine basıldığında."
                : "When you press × on the window."}
            </span>
          </div>
          <select
            id="close-behavior"
            aria-describedby="close-behavior-description"
            value={current?.close_behavior ?? "ask"}
            disabled={!current || save.isPending}
            onChange={(event) =>
              save.mutate({
                setting: "close_behavior",
                value: event.target.value as CloseBehavior,
              })
            }
          >
            <option value="ask">
              {tr ? "Her seferinde sor" : "Ask every time"}
            </option>
            <option value="minimize">
              {tr ? "Bildirim alanına küçült" : "Minimize to tray"}
            </option>
            <option value="quit">{tr ? "Tamamen kapat" : "Quit Dockon"}</option>
          </select>
        </div>
        <label className="settings-row">
          <span className="settings-icon">
            <AudioWaveform size={19} aria-hidden="true" />
          </span>
          <span className="settings-copy">
            <strong>{tr ? "Animasyonları azalt" : "Reduce motion"}</strong>
            <span id="motion-description">
              {tr
                ? "Arayüzdeki hareketli geçişleri kapatır."
                : "Disables animated transitions in the interface."}
            </span>
          </span>
          <input
            className="settings-switch"
            type="checkbox"
            role="switch"
            aria-label={tr ? "Animasyonları azalt" : "Reduce motion"}
            aria-describedby="motion-description"
            checked={reduceMotion}
            onChange={(event) => onReduceMotion(event.target.checked)}
          />
        </label>
      </div>
      <p className="settings-message" role="status">
        {save.isPending
          ? tr
            ? "Kaydediliyor…"
            : "Saving…"
          : save.isSuccess && (tr ? "Ayar kaydedildi." : "Setting saved.")}
      </p>
    </section>
  );
}
