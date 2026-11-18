import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, LoaderCircle, Minus, Power } from "lucide-react";
import { Button } from "../../components/ui/button";

export function CloseDialog({
  lang,
  quitting,
  onDismiss,
}: {
  lang: "tr" | "en";
  quitting: boolean;
  onDismiss: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const tr = lang === "tr";
  const pending = busy || quitting;

  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);

  async function choose(command: "minimize_to_tray" | "quit_desktop") {
    setBusy(true);
    setError("");
    try {
      await invoke(command);
      if (command === "minimize_to_tray") onDismiss();
    } catch (error) {
      setError(String(error));
      setBusy(false);
    }
  }

  return (
    <dialog
      ref={dialog}
      className="modal close-dialog"
      aria-labelledby="close-dialog-heading"
      onCancel={(event) => {
        event.preventDefault();
        if (!pending) onDismiss();
      }}
    >
      <h2 id="close-dialog-heading">
        {tr ? "Dockon kapatılsın mı?" : "Close Dockon?"}
      </h2>
      <div className="close-options">
        <button
          className="close-option"
          disabled={pending}
          onClick={() => void choose("minimize_to_tray")}
        >
          <span className="settings-icon">
            <Minus size={19} aria-hidden="true" />
          </span>
          <span className="settings-copy">
            <strong>
              {tr ? "Bildirim alanına küçült" : "Minimize to tray"}
            </strong>
            <span>
              {tr
                ? "Servisler arka planda çalışmaya devam eder."
                : "Services keep running in the background."}
            </span>
          </span>
          <ChevronRight size={17} aria-hidden="true" />
        </button>
        <button
          className="close-option close-option-quit"
          disabled={pending}
          onClick={() => void choose("quit_desktop")}
        >
          <span className="settings-icon">
            <Power size={19} aria-hidden="true" />
          </span>
          <span className="settings-copy">
            <strong>{tr ? "Tamamen kapat" : "Quit Dockon"}</strong>
            <span>
              {tr
                ? "Çalışan projeleri ve servisleri durdurur."
                : "Stops running projects and services."}
            </span>
          </span>
          <ChevronRight size={17} aria-hidden="true" />
        </button>
      </div>
      {quitting && (
        <p className="close-progress" role="status">
          <LoaderCircle className="dock-spin" size={16} aria-hidden="true" />
          {tr ? "Servisler durduruluyor…" : "Stopping services…"}
        </p>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <div className="dialog-actions">
        <Button onClick={onDismiss} disabled={pending} autoFocus>
          {tr ? "Vazgeç" : "Cancel"}
        </Button>
      </div>
    </dialog>
  );
}
