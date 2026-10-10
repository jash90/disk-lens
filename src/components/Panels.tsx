import { useEffect, useState } from "react";
import { api, errorMessage, type Breakdown, type DiffResult, type FileRow, type Skipped } from "../api";
import { pickScanFile, reveal } from "../lib/actions";
import { CATEGORY_COLORS, kindIcon } from "../lib/colors";
import { filesLabel, formatBytes, formatCount, formatDate, formatPct } from "../lib/format";
import { useStore, useViewArgs } from "../store";
import { IS_MAC } from "../lib/platform";

/** Wspólne pobieranie danych panelu przy zmianie korzenia/danych/widoku. */
function usePanelData<T>(load: () => Promise<T>, deps: unknown[]): { data: T | null; error: string | null } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    load()
      .then((d) => {
        if (alive) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return { data, error };
}

export function TopFiles() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const openMenu = useStore((s) => s.openMenu);
  const view = useViewArgs();
  const { data, error } = usePanelData<FileRow[]>(() => api.topFiles(root, view, 100), [root, version, view]);
  const max = data?.[0]?.size ?? 1;

  if (error) return <div className="panel-msg error">{error}</div>;
  if (!data) return <div className="panel-msg">Liczę…</div>;
  if (data.length === 0) return <div className="panel-msg">Brak plików w tym widoku.</div>;
  return (
    <div className="panel-scroll">
      <table className="list">
        <thead>
          <tr>
            <th className="num">#</th>
            <th>Plik</th>
            <th className="num">Rozmiar</th>
            <th>Zmodyfikowano</th>
          </tr>
        </thead>
        <tbody>
          {data.map((f, i) => (
            <tr
              key={f.id}
              className={selected === f.id ? "selected" : ""}
              onClick={() => select(f.id, "other")}
              onDoubleClick={() => reveal(f.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                select(f.id, "other");
                openMenu({ x: e.clientX, y: e.clientY, id: f.id });
              }}
            >
              <td className="num muted">{i + 1}</td>
              <td>
                <div className="file-cell">
                  <span className="icon">{kindIcon(f.kind, f.cat)}</span>
                  <div>
                    <div className="name">{f.name}</div>
                    <div className="muted path">{f.path}</div>
                  </div>
                </div>
                <div className="mini-bar">
                  <span style={{ width: `${(f.size / max) * 100}%`, background: CATEGORY_COLORS[f.cat] }} />
                </div>
              </td>
              <td className="num">{formatBytes(f.size)}</td>
              <td className="muted">{formatDate(f.mtime, true)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function TypeBreakdownPanel() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const view = useViewArgs();
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const { data, error } = usePanelData<Breakdown>(() => api.typeBreakdown(root, view), [root, version, view]);

  if (error) return <div className="panel-msg error">{error}</div>;
  if (!data) return <div className="panel-msg">Liczę…</div>;
  const total = Math.max(1, data.categories.reduce((a, c) => a + c.size, 0));
  return (
    <div className="panel-scroll breakdown">
      <div className="stacked" aria-hidden>
        {data.categories.map((c) => (
          <span key={c.cat} style={{ width: `${(c.size / total) * 100}%`, background: CATEGORY_COLORS[c.cat] }} title={c.name} />
        ))}
      </div>
      <h3>Kategorie</h3>
      <table className="list">
        <tbody>
          {data.categories.map((c) => (
            <tr key={c.cat}>
              <td>
                <span className="swatch" style={{ background: CATEGORY_COLORS[c.cat] }} /> {c.name}
              </td>
              <td className="num">{formatBytes(c.size)}</td>
              <td className="num muted">{formatPct((c.size / total) * 100)}</td>
              <td className="num muted">{filesLabel(c.files)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Rozszerzenia</h3>
      <table className="list">
        <tbody>
          {data.extensions.map((e) => (
            <tr key={e.ext}>
              <td>
                <span className="swatch" style={{ background: CATEGORY_COLORS[e.cat] }} />{" "}
                {e.ext.startsWith("(") ? e.ext : `.${e.ext}`}
              </td>
              <td className="num">{formatBytes(e.size)}</td>
              <td className="num muted">{formatPct((e.size / total) * 100)}</td>
              <td className="num muted">{formatCount(e.files)}</td>
              <td className="num">
                {e.ext.startsWith("(") ? null : (
                  <button className="link" onClick={() => setFilter({ ...filter, extensions: [e.ext] })}>
                    filtruj
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function SkippedPanel() {
  const version = useStore((s) => s.version);
  const { data, error } = usePanelData<{ total: number; items: Skipped[] }>(() => api.skippedPaths(2000), [version]);
  if (error) return <div className="panel-msg error">{error}</div>;
  if (!data) return <div className="panel-msg">Wczytuję…</div>;
  return (
    <div className="panel-scroll">
      <div className="callout">
        {data.total === 0 ? (
          <p>Wszystkie foldery zostały odczytane.</p>
        ) : (
          <>
            <p>
              Pominięto <b>{formatCount(data.total)}</b> ścieżek (brak uprawnień lub błąd odczytu). Ich zawartość nie jest
              wliczona do sum.
            </p>
            {IS_MAC ? (
              <>
                <p className="muted">
                  Chronione foldery (Poczta, Wiadomości, Safari, kontenery aplikacji) wymagają „Pełnego dostępu do dysku” dla
                  Disk Lens. Po nadaniu uprawnienia uruchom aplikację ponownie i przeskanuj.
                </p>
                <button className="btn" onClick={() => api.openPrivacySettings().catch(() => {})}>
                  Otwórz ustawienia prywatności
                </button>
              </>
            ) : (
              <p className="muted">
                Część folderów systemowych i innych użytkowników jest dostępna tylko dla administratora. Uruchom Disk Lens z
                uprawnieniami administratora, jeśli chcesz je policzyć.
              </p>
            )}
          </>
        )}
      </div>
      {data.items.length > 0 ? (
        <table className="list">
          <tbody>
            {data.items.map((s, i) => (
              <tr key={`${s.path}-${i}`}>
                <td className="path-cell">{s.path}</td>
                <td className="muted">{s.error}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {data.total > data.items.length ? (
        <div className="panel-msg">… i {formatCount(data.total - data.items.length)} więcej</div>
      ) : null}
    </div>
  );
}

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
    // Przeliczenie po zmianie trybu rozmiaru.
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
              {delta >= 0 ? "+" : "−"}
              {formatBytes(Math.abs(delta))}
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
                    {e.delta >= 0 ? "+" : "−"}
                    {formatBytes(Math.abs(e.delta))}
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
