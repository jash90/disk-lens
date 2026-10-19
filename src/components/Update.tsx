import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorMessage, type UpdateInfo } from "../api";
import { formatBytes } from "../lib/format";
import { useStore } from "../store";

const RELEASES = "https://github.com/jash90/disk-lens/releases/latest";

type Status =
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "installing"; info: UpdateInfo; done: number; total: number }
  | { kind: "error"; message: string; info?: UpdateInfo };

/** Wynik cichego sprawdzenia po starcie (i każdego kolejnego) trafia do store'a. */
export function useUpdateEvents() {
  useEffect(() => {
    const st = useStore.getState;
    api
      .knownUpdate()
      .then((u) => u && st().setUpdate(u))
      .catch(() => {});
    const un = api.onUpdateStatus((u) => st().setUpdate(u));
    return () => {
      un.then((f) => f());
    };
  }, []);
}

/** Przycisk na pasku narzędzi, widoczny tylko gdy jest nowa wersja. */
export function UpdateButton() {
  const update = useStore((s) => s.update);
  const setUpdateDialog = useStore((s) => s.setUpdateDialog);
  if (!update) return null;
  return (
    <button className="btn update-btn" onClick={() => setUpdateDialog("show")} title="Pobierz i zainstaluj nową wersję">
      Aktualizacja {update.version}
    </button>
  );
}

export function UpdateDialog() {
  const mode = useStore((s) => s.updateDialog);
  const setUpdateDialog = useStore((s) => s.setUpdateDialog);
  const known = useStore((s) => s.update);
  const scanning = useStore((s) => s.scanning);
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState<Status>({ kind: "checking" });
  /** Rośnie przy „Spróbuj ponownie” – wymusza nowe sprawdzenie serwera. */
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (mode === null) return;
    const current = useStore.getState().update;
    if (mode === "show" && current && attempt === 0) {
      setStatus({ kind: "available", info: current });
      return;
    }
    let alive = true;
    setStatus({ kind: "checking" });
    api
      .checkUpdate()
      .then((info) => alive && setStatus(info ? { kind: "available", info } : { kind: "latest" }))
      .catch((e) => alive && setStatus({ kind: "error", message: errorMessage(e) }));
    return () => {
      alive = false;
    };
  }, [mode, attempt]);

  useEffect(() => {
    const un = api.onUpdateProgress((p) => {
      setStatus((s) => (s.kind === "installing" && !p.finished ? { ...s, done: p.done, total: p.total } : s));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (mode === null) return null;
  const installing = status.kind === "installing";
  const close = () => !installing && setUpdateDialog(null);
  const install = (info: UpdateInfo) => {
    setStatus({ kind: "installing", info, done: 0, total: 0 });
    // Po udanej instalacji aplikacja startuje od nowa – tu wracamy tylko z błędem.
    api.installUpdate().catch((e) => setStatus({ kind: "error", message: errorMessage(e), info }));
  };
  const info = status.kind === "available" || status.kind === "installing" || status.kind === "error" ? status.info : undefined;
  const fraction = installing && status.total > 0 ? Math.min(1, status.done / status.total) : 0;

  return (
    <div className="modal-backdrop" onMouseDown={close}>
      <div
        className="modal update-modal"
        role="dialog"
        aria-labelledby="update-title"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && close()}
      >
        <h2 id="update-title">{info ? `Disk Lens ${info.version}` : "Aktualizacje"}</h2>
        <p className="muted">
          Zainstalowana wersja: {version || known?.currentVersion || "?"}
          {info?.date ? ` · nowa wersja z ${info.date}` : ""}
        </p>

        {status.kind === "checking" ? <p>Sprawdzam…</p> : null}
        {status.kind === "latest" ? <p>Masz najnowszą wersję.</p> : null}
        {status.kind === "error" ? <p className="error">{status.message}</p> : null}
        {info?.notes && !installing ? <div className="update-notes">{info.notes}</div> : null}
        {installing ? (
          <>
            <div className="update-progress" role="progressbar" aria-valuenow={Math.round(fraction * 100)}>
              <span style={{ width: `${Math.round(fraction * 100)}%` }} />
            </div>
            <p className="muted">
              {status.total > 0
                ? `Pobrano ${formatBytes(status.done)} z ${formatBytes(status.total)}`
                : `Pobrano ${formatBytes(status.done)}`}
              {" "}· po instalacji aplikacja uruchomi się ponownie.
            </p>
          </>
        ) : null}
        {status.kind === "available" && scanning ? (
          <p className="muted">Instalacja uruchomi aplikację ponownie – poczekaj na koniec skanu albo go anuluj.</p>
        ) : null}

        <div className="modal-actions">
          {status.kind === "error" ? (
            <button className="btn" onClick={() => openUrl(RELEASES).catch(() => {})}>
              Otwórz stronę wydania
            </button>
          ) : null}
          {status.kind === "error" && !status.info ? (
            <button className="btn" onClick={() => setAttempt((a) => a + 1)}>
              Spróbuj ponownie
            </button>
          ) : null}
          {status.kind === "latest" || (status.kind === "error" && !status.info) ? (
            <button className="btn" onClick={close}>
              Zamknij
            </button>
          ) : (
            <button className="btn" disabled={installing} onClick={close}>
              Później
            </button>
          )}
          {status.kind === "available" || (status.kind === "error" && status.info) ? (
            <button className="btn primary" disabled={scanning} onClick={() => info && install(info)}>
              Pobierz i zainstaluj
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
