import { useEffect, useRef, useState } from "react";
import { api, type Volume } from "../api";
import {
  cancelScan,
  chooseAndScan,
  exportView,
  loadScan,
  saveScan,
  startScan,
} from "../lib/actions";
import { filesLabel, foldersLabel, formatBytes, formatCount, formatDuration, middleEllipsis } from "../lib/format";
import { useStore } from "../store";
import { shortcut } from "../lib/platform";

function Menu({ label, children }: { label: string; children: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);
  return (
    <div className="dropdown" ref={ref}>
      <button className="btn" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {label} ▾
      </button>
      {open ? <div className="dropdown-menu">{children(() => setOpen(false))}</div> : null}
    </div>
  );
}

export function Toolbar() {
  const scanning = useStore((s) => s.scanning);
  const progress = useStore((s) => s.progress);
  const info = useStore((s) => s.info);
  const mode = useStore((s) => s.mode);
  const setMode = useStore((s) => s.setMode);
  const [cross, setCross] = useState(false);
  const [volumes, setVolumes] = useState<Volume[]>([]);

  useEffect(() => {
    api.listVolumes().then(setVolumes).catch(() => setVolumes([]));
  }, []);

  return (
    <header className="toolbar">
      <div className="tb-group">
        <button className="btn primary" onClick={() => chooseAndScan(cross)} title={`Skanuj folder (${shortcut("O")})`}>
          Skanuj folder…
        </button>
        <Menu label="Wolumen">
          {(close) => (
            <>
              {volumes.map((v) => (
                <button
                  key={v.path}
                  className="menu-item"
                  onClick={() => {
                    close();
                    startScan(v.path, cross);
                  }}
                >
                  <span>{v.name}</span>
                  <span className="muted">
                    {formatBytes(v.total - v.free)} / {formatBytes(v.total)}
                  </span>
                </button>
              ))}
              {volumes.length === 0 ? <div className="menu-note">Brak wolumenów</div> : null}
            </>
          )}
        </Menu>
        <label className="check" title="Wchodź do punktów montowania innych dysków i obrazów">
          <input type="checkbox" checked={cross} onChange={(e) => setCross(e.target.checked)} />
          Inne wolumeny
        </label>
      </div>

      <div className="tb-progress">
        {scanning && progress ? (
          <>
            <span className="spinner" aria-hidden />
            <span className="stat">
              <b>{formatCount(progress.files)}</b> plików · <b>{formatBytes(progress.bytes)}</b> ·{" "}
              {formatDuration(progress.elapsedMs)}
            </span>
            <span className="current" title={progress.current}>
              {middleEllipsis(progress.current, 70)}
            </span>
            <button className="btn danger" onClick={cancelScan}>
              Anuluj
            </button>
          </>
        ) : scanning ? (
          <>
            <span className="spinner" aria-hidden />
            <span className="stat">Rozpoczynam skan…</span>
            <button className="btn danger" onClick={cancelScan}>
              Anuluj
            </button>
          </>
        ) : progress?.cancelled ? (
          <span className="stat warn-text">Skan anulowany – wyniki są częściowe.</span>
        ) : progress?.done ? (
          <span className="stat">
            Gotowe: {filesLabel(progress.files)}, {foldersLabel(progress.dirs)} w{" "}
            {formatDuration(progress.elapsedMs)}
          </span>
        ) : info ? (
          <span className="stat">{info.rootPath}</span>
        ) : null}
      </div>

      <div className="tb-group">
        <div className="seg" role="group" aria-label="Tryb rozmiaru">
          <button
            className={mode === "alloc" ? "on" : ""}
            onClick={() => setMode("alloc")}
            title="Miejsce faktycznie zajęte na dysku (bloki). Tak liczy „du”."
          >
            Zajęte
          </button>
          <button
            className={mode === "logical" ? "on" : ""}
            onClick={() => setMode("logical")}
            title="Rozmiar logiczny plików. Pliki rzadkie, skompresowane i klony (APFS, Btrfs) mogą mieć go większy niż zajęte miejsce."
          >
            Logiczny
          </button>
        </div>
        <span
          className="info-dot"
          title={
            "Suma może różnić się od zajętości pokazywanej przez system: klony plików (APFS, Btrfs) dzielą bloki " +
            "i są tu liczone przy każdej kopii, pliki rzadkie zajmują mniej niż ich rozmiar logiczny, a migawki " +
            "i pliki systemowe nie są widoczne w folderach."
          }
        >
          ⓘ
        </span>
        <Menu label="Plik">
          {(close) => (
            <>
              <button className="menu-item" disabled={!info || scanning} onClick={() => (close(), saveScan())}>
                Zapisz skan…
              </button>
              <button className="menu-item" onClick={() => (close(), loadScan())}>
                Wczytaj skan…
              </button>
              <div className="menu-sep" />
              <button className="menu-item" disabled={!info} onClick={() => (close(), exportView("csv"))}>
                Eksportuj widok do CSV…
              </button>
              <button className="menu-item" disabled={!info} onClick={() => (close(), exportView("json"))}>
                Eksportuj widok do JSON…
              </button>
            </>
          )}
        </Menu>
      </div>
    </header>
  );
}
