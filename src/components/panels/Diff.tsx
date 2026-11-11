import { useEffect, useState } from "react";
import { api, errorMessage, type DiffResult } from "../../api";
import { pickScanFile } from "../../lib/actions";
import { formatBytes, formatDate, formatSignedBytes } from "../../lib/format";
import { useStore } from "../../store";

const STATUS_LABEL: Record<string, string> = { grown: "urosło", shrunk: "zmalało", added: "nowe", removed: "usunięte" };

export function DiffPanel() {
  const mode = useStore((s) => s.mode);
  const scanning = useStore((s) => s.scanning);
  const [result, setResult] = useState<DiffResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [onlyDirs, setOnlyDirs] = useState(false);
  const [file, setFile] = useState<string | null>(null);

  const run = async (path: string) => {
    setBusy(true);
    setError(null);
    try {
      setResult(await api.diffScans(path, mode));
      setFile(path);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (file) run(file);
    // Recompute when the size mode changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode]);

  const choose = async () => {
    const p = await pickScanFile("Wybierz wcześniejszy skan do porównania");
    if (p) run(p);
  };

  const entries = result?.entries.filter((e) => !onlyDirs || e.isDir) ?? [];
  const delta = result ? result.totalAfter - result.totalBefore : 0;

  return (
    <div className="panel-scroll">
      <div className="callout">
        <p>
          Porównaj bieżący skan z wcześniej zapisanym (Plik → Zapisz skan…) tego samego folderu, żeby zobaczyć, co urosło, a co
          zmalało.
        </p>
        <button className="btn" onClick={choose} disabled={busy || scanning}>
          {busy ? "Porównuję…" : "Wybierz zapisany skan…"}
        </button>
        {error ? <p className="error">{error}</p> : null}
      </div>
      {result ? (
        <>
          {result.beforeRoot !== result.afterRoot ? (
            <p className="warn-text">
              Uwaga: porównujesz różne foldery ({result.beforeRoot} → {result.afterRoot}).
            </p>
          ) : null}
          <div className="diff-summary">
            <span>
              {formatDate(result.beforeScannedAt)}: <b>{formatBytes(result.totalBefore)}</b>
            </span>
            <span>→</span>
            <span>
              teraz: <b>{formatBytes(result.totalAfter)}</b>
            </span>
            <span className={delta >= 0 ? "up" : "down"}>
              {formatSignedBytes(delta)}
            </span>
            <label className="check">
              <input type="checkbox" checked={onlyDirs} onChange={(e) => setOnlyDirs(e.target.checked)} /> tylko foldery
            </label>
          </div>
          <table className="list">
            <thead>
              <tr>
                <th>Ścieżka</th>
                <th>Zmiana</th>
                <th className="num">Przed</th>
                <th className="num">Po</th>
                <th className="num">Różnica</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr key={e.path}>
                  <td className="path-cell" style={{ paddingLeft: 8 + Math.min(e.depth, 8) * 12 }}>
                    {e.isDir ? "📁 " : ""}
                    {e.path}
                  </td>
                  <td>
                    <span className={`status ${e.status}`}>{STATUS_LABEL[e.status]}</span>
                  </td>
                  <td className="num muted">{formatBytes(e.before)}</td>
                  <td className="num muted">{formatBytes(e.after)}</td>
                  <td className={`num ${e.delta >= 0 ? "up" : "down"}`}>
                    {formatSignedBytes(e.delta)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {entries.length === 0 ? <div className="panel-msg">Brak różnic.</div> : null}
        </>
      ) : null}
    </div>
  );
}
