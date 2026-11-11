import { api, type Skipped } from "../../api";
import { formatCount } from "../../lib/format";
import { IS_MAC } from "../../lib/platform";
import { useAsync } from "../../lib/useAsync";
import { useStore } from "../../store";

export function SkippedPanel() {
  const version = useStore((s) => s.version);
  const { data, error } = useAsync<{ total: number; items: Skipped[] }>(() => api.skippedPaths(2000), [version]);
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
