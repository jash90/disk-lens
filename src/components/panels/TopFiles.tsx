import { api, type FileRow } from "../../api";
import { openNodeMenu, reveal } from "../../lib/actions";
import { CATEGORY_COLORS, kindIcon } from "../../lib/colors";
import { formatBytes, formatDate } from "../../lib/format";
import { useAsync } from "../../lib/useAsync";
import { useStore, useViewArgs } from "../../store";

export function TopFiles() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const view = useViewArgs();
  const { data, error } = useAsync<FileRow[]>(() => api.topFiles(root, view, 100), [root, version, view]);
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
              onContextMenu={(e) => openNodeMenu(e, f.id)}
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
