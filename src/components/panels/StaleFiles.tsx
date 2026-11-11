import { useEffect, useMemo, useState } from "react";
import { api, errorMessage, type AgeHistogram, type StaleArgs, type StaleListKind, type StaleRow, type StaleSort } from "../../api";
import { exportView, openNodeMenu, reveal } from "../../lib/actions";
import { AGE_COLORS, AGE_NAMES, kindIcon } from "../../lib/colors";
import { formatAge, formatBytes, formatCount, formatDate, formatPct, itemsLabel } from "../../lib/format";
import { useAsync } from "../../lib/useAsync";
import { useStore, useViewArgs } from "../../store";

const PAGE = 200;
/** The Trash command takes at most this many items at once (`trash_batch::MAX_ITEMS`). */
const MAX_SELECTED = 5000;

const LISTS: { id: StaleListKind; label: string; hint: string }[] = [
  { id: "files", label: "Pliki", hint: "Pliki, których od dawna nikt nie zmieniał ani nie dodał na ten dysk." },
  { id: "folders", label: "Foldery", hint: "Foldery, w których od dawna nic się nie zmieniło. Pokazany jest najwyższy taki folder." },
  {
    id: "caches",
    label: "Cache projektów",
    hint: "Foldery zależności i kompilacji (node_modules, target, DerivedData…) w projektach, których od dawna nikt nie ruszał. Narzędzia odtworzą je przy następnej kompilacji.",
  },
];

const THRESHOLDS = [
  { days: 183, label: "6 mies." },
  { days: 365, label: "1 rok" },
  { days: 730, label: "2 lata" },
  { days: 1095, label: "3 lata" },
];

const SORTS: { id: StaleSort; label: string }[] = [
  { id: "size", label: "największe" },
  { id: "age", label: "najstarsze" },
  { id: "score", label: "duże i stare" },
  { id: "path", label: "ścieżka" },
];

const INSTALLER_EXTS = ["dmg", "pkg", "zip", "iso", "exe", "msi"];

const ACCESS_CAVEAT =
  "Data ostatniego dostępu bywa niedokładna: część systemów jej nie aktualizuje, a kopie zapasowe i indeksowanie ją zmieniają.";

const UNKNOWN_AGE_COLOR = "var(--map-rest)";

function AgeStrip({ data }: { data: AgeHistogram }) {
  const parts = [
    ...data.buckets.map((b, i) => ({ ...b, name: AGE_NAMES[i], color: AGE_COLORS[i] })),
    { ...data.unknown, name: "data nieznana", color: UNKNOWN_AGE_COLOR },
  ].filter((p) => p.size > 0);
  const total = parts.reduce((sum, p) => sum + p.size, 0);
  if (total === 0) return null;
  return (
    <div className="age-strip">
      <div className="stacked" role="img" aria-label="Zajęte miejsce według daty modyfikacji">
        {parts.map((p) => (
          <span
            key={p.name}
            style={{ width: `${(p.size / total) * 100}%`, background: p.color }}
            title={`${p.name}: ${formatBytes(p.size)} (${formatPct((p.size / total) * 100)})`}
          />
        ))}
      </div>
      <div className="age-legend">
        {parts.map((p) => (
          <span key={p.name} className="legend-item">
            <span className="swatch" style={{ background: p.color }} />
            {p.name} <b>{formatBytes(p.size)}</b>
          </span>
        ))}
      </div>
    </div>
  );
}

export function StaleFiles() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const scanning = useStore((s) => s.scanning);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const setTrashTarget = useStore((s) => s.setTrashTarget);
  const view = useViewArgs();

  const [list, setList] = useState<StaleListKind>("files");
  const [days, setDays] = useState(365);
  const [custom, setCustom] = useState(false);
  const [useAccess, setUseAccess] = useState(false);
  const [showApps, setShowApps] = useState(false);
  const [sort, setSort] = useState<StaleSort>("size");

  const args: StaleArgs = useMemo(
    () => ({ list, olderThanDays: days, useAccess, showApps, sort, desc: sort !== "path" }),
    [list, days, useAccess, showApps, sort],
  );

  const histogram = useAsync(() => api.ageHistogram(root, view), [root, version, view]);

  const [rows, setRows] = useState<StaleRow[]>([]);
  const [total, setTotal] = useState(0);
  const [totalSize, setTotalSize] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** Checked rows: id → size. */
  const [checked, setChecked] = useState<Map<number, number>>(new Map());

  // A new list (other arguments, view or data) starts from its first page with nothing checked.
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setChecked(new Map());
    api
      .staleList(root, view, args, 0, PAGE)
      .then((page) => {
        if (!alive) return;
        setRows(page.rows);
        setTotal(page.total);
        setTotalSize(page.totalSize);
        setError(null);
      })
      .catch((e) => {
        if (!alive) return;
        setRows([]);
        setTotal(0);
        setTotalSize(0);
        setError(errorMessage(e));
      })
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [root, version, view, args]);

  const loadMore = async () => {
    const offset = rows.length;
    try {
      const page = await api.staleList(root, view, args, offset, PAGE);
      // Ignore the answer if the list was replaced while it was on its way.
      setRows((cur) => (cur.length === offset ? [...cur, ...page.rows] : cur));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const toggle = (row: StaleRow) =>
    setChecked((cur) => {
      const next = new Map(cur);
      if (!next.delete(row.id) && next.size < MAX_SELECTED) next.set(row.id, row.size);
      return next;
    });

  const allChecked = rows.length > 0 && rows.every((r) => checked.has(r.id));
  const toggleAll = () => setChecked(allChecked ? new Map() : new Map(rows.slice(0, MAX_SELECTED).map((r) => [r.id, r.size])));

  const checkedSize = [...checked.values()].reduce((sum, size) => sum + size, 0);
  const installersOn = filter.extensions.join() === INSTALLER_EXTS.join();
  const now = Date.now() / 1000;
  const current = LISTS.find((l) => l.id === list)!;

  return (
    <div className="stale">
      <div className="stale-head">
        {histogram.data ? <AgeStrip data={histogram.data} /> : null}
        <div className="stale-controls">
          <div className="seg" role="group" aria-label="Rodzaj listy">
            {LISTS.map((l) => (
              <button key={l.id} className={list === l.id ? "on" : ""} onClick={() => setList(l.id)}>
                {l.label}
              </button>
            ))}
          </div>
          <span className="muted">starsze niż</span>
          <div className="seg" role="group" aria-label="Próg wieku">
            {THRESHOLDS.map((t) => (
              <button
                key={t.days}
                className={!custom && days === t.days ? "on" : ""}
                onClick={() => {
                  setCustom(false);
                  setDays(t.days);
                }}
              >
                {t.label}
              </button>
            ))}
            <button className={custom ? "on" : ""} onClick={() => setCustom(true)}>
              własny
            </button>
          </div>
          {custom ? (
            <label className="stale-days">
              <input
                type="number"
                min={1}
                max={36500}
                value={days}
                onChange={(e) => setDays(Math.min(36500, Math.max(1, Math.round(Number(e.target.value)) || 1)))}
                aria-label="Liczba dni"
              />
              dni
            </label>
          ) : null}
          <select value={sort} onChange={(e) => setSort(e.target.value as StaleSort)} aria-label="Sortowanie" title="Sortowanie">
            {SORTS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
        </div>
        <div className="stale-controls">
          {list === "files" ? (
            <>
              <label className="check" title={ACCESS_CAVEAT}>
                <input type="checkbox" checked={useAccess} onChange={(e) => setUseAccess(e.target.checked)} />
                uwzględnij ostatni dostęp
              </label>
              <label className="check" title="Aplikacje wyglądają na stare, bo liczy się data ich wydania, a nie użycia.">
                <input type="checkbox" checked={showApps} onChange={(e) => setShowApps(e.target.checked)} />
                pokaż aplikacje
              </label>
              <button
                className={installersOn ? "btn small primary" : "btn small"}
                onClick={() => setFilter({ ...filter, extensions: installersOn ? [] : INSTALLER_EXTS })}
                title={`Filtr rozszerzeń: ${INSTALLER_EXTS.join(", ")}`}
              >
                Instalatory i archiwa
              </button>
            </>
          ) : null}
          <span className="stale-spacer" />
          <button className="btn small" disabled={total === 0} onClick={() => exportView("csv", args)}>
            CSV…
          </button>
          <button className="btn small" disabled={total === 0} onClick={() => exportView("json", args)}>
            JSON…
          </button>
        </div>
        <p className="muted stale-hint">
          {current.hint}
          {list === "files" && useAccess ? ` ${ACCESS_CAVEAT}` : ""}
        </p>
      </div>

      <div className="panel-scroll">
        {error ? (
          <div className="panel-msg error">{error}</div>
        ) : loading && rows.length === 0 ? (
          <div className="panel-msg">Liczę…</div>
        ) : rows.length === 0 ? (
          <div className="panel-msg">Nic starszego niż wybrany próg w tym widoku.</div>
        ) : (
          <>
            <table className="list">
              <thead>
                <tr>
                  <th className="check-col">
                    <input type="checkbox" checked={allChecked} onChange={toggleAll} aria-label="Zaznacz wszystkie pokazane" />
                  </th>
                  <th>Element</th>
                  <th className="num">Rozmiar</th>
                  <th>Zmodyfikowano</th>
                  <th>Ostatni dostęp</th>
                  <th className="num">Bez zmian od</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr
                    key={r.id}
                    className={selected === r.id ? "selected" : ""}
                    onClick={() => select(r.id, "other")}
                    onDoubleClick={() => reveal(r.id)}
                    onContextMenu={(e) => openNodeMenu(e, r.id)}
                  >
                    <td className="check-col" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={checked.has(r.id)} onChange={() => toggle(r)} aria-label={`Zaznacz ${r.name}`} />
                    </td>
                    <td>
                      <div className="file-cell">
                        <span className="icon">{kindIcon(r.kind, r.cat)}</span>
                        <div>
                          <div className="name">{r.name}</div>
                          <div className="muted path">{r.path}</div>
                        </div>
                      </div>
                    </td>
                    <td className="num">{formatBytes(r.size)}</td>
                    <td className="muted">{formatDate(r.mtime, true)}</td>
                    <td className="muted">{formatDate(r.atime, true)}</td>
                    <td className="num">{formatAge(now - r.lastActivity)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length < total ? (
              <button className="btn small stale-more" onClick={loadMore}>
                Pokaż więcej ({formatCount(total - rows.length)})
              </button>
            ) : null}
          </>
        )}
      </div>

      <div className="stale-foot">
        <span className="muted">
          Razem: {itemsLabel(total)}, {formatBytes(totalSize)}
        </span>
        <span className="stale-spacer" />
        <span>
          Zaznaczono {formatCount(checked.size)}, {formatBytes(checkedSize)}
        </span>
        <button
          className="btn small danger"
          disabled={checked.size === 0 || scanning}
          onClick={() => setTrashTarget({ ids: [...checked.keys()], verify: true })}
        >
          Przenieś do Kosza…
        </button>
      </div>
    </div>
  );
}
