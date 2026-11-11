import { api, type Breakdown } from "../../api";
import { CATEGORY_COLORS } from "../../lib/colors";
import { filesLabel, formatBytes, formatCount, formatPct } from "../../lib/format";
import { useAsync } from "../../lib/useAsync";
import { useStore, useViewArgs } from "../../store";

export function TypeBreakdownPanel() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const view = useViewArgs();
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const { data, error } = useAsync<Breakdown>(() => api.typeBreakdown(root, view), [root, version, view]);

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
