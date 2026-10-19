import type { ReactNode } from "react";
import type { Coverage, TreeInfo, VolumeInfo } from "../api";
import { startScan } from "../lib/actions";
import { formatBytes, formatCount, formatDate, formatPct } from "../lib/format";
import { IS_MAC, OS } from "../lib/platform";
import { useStore } from "../store";
import { useVolumes, VolumeTile } from "./Volumes";

/** Skan starszy niż to (s) dostaje uwagę, że dane dysku są nowsze niż skan. */
const STALE_AFTER = 15 * 60;

/** Na macOS dane użytkownika leżą na wolumenie Data, ale cały dysk skanuje się od „/” (przez firmlinki). */
function scanTarget(vol: VolumeInfo): string {
  return IS_MAC && vol.mountPath === "/System/Volumes/Data" ? "/" : vol.mountPath;
}

function isWholeVolume(info: TreeInfo, vol: VolumeInfo): boolean {
  const root = info.rootPath.replace(/[\\/]+$/, "") || "/";
  const mount = vol.mountPath.replace(/[\\/]+$/, "") || "/";
  return root.toLowerCase() === mount.toLowerCase() || root === scanTarget(vol);
}

interface Segment {
  label: string;
  value: number;
  cls: string;
  note?: ReactNode;
}

/** Pasek złożony z segmentów + legenda z wartościami i udziałem w całości. */
function StackBar({ segments, total }: { segments: Segment[]; total: number }) {
  const shown = segments.filter((s) => s.value > 0);
  return (
    <div className="stack">
      <div className="stack-bar" role="img" aria-label={shown.map((s) => `${s.label}: ${formatBytes(s.value)}`).join(", ")}>
        {shown.map((s) => (
          <span key={s.cls} className={`seg ${s.cls}`} style={{ width: `${total ? (s.value / total) * 100 : 0}%` }} />
        ))}
      </div>
      <ul className="stack-legend">
        {segments.map((s) => (
          <li key={s.cls}>
            <span className={`swatch ${s.cls}`} />
            <span>{s.label}</span>
            <b>{formatBytes(s.value)}</b>
            <span className="muted">{formatPct(total ? (s.value / total) * 100 : 0)}</span>
            {/* Zawsze 5 komórek – wiersze legendy to `display: contents` w siatce 5-kolumnowej. */}
            <span className="muted legend-note">{s.note}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function CoverageSection({ info, vol, cov }: { info: TreeInfo; vol: VolumeInfo; cov: Coverage }) {
  const scanning = useStore((s) => s.scanning);
  const setTab = useStore((s) => s.setTab);
  const whole = isWholeVolume(info, vol);
  const found = Math.min(cov.scanned, cov.used);
  const stale = !scanning && Date.now() / 1000 - info.scannedAt > STALE_AFTER;
  const apfs = vol.fsType.toLowerCase() === "apfs";

  return (
    <>
      <h3>Skan a zajętość dysku</h3>
      {scanning ? <p className="muted">Trwa skan – zestawienie będzie pełne po jego zakończeniu.</p> : null}
      <StackBar
        total={cov.used}
        segments={[
          { label: "Znalezione w skanie", value: found, cls: "scanned" },
          { label: "Poza skanem", value: cov.unaccounted, cls: "unaccounted" },
        ]}
      />
      {cov.overcounted > 0 ? (
        <p className="muted">
          Skan pokazuje o {formatBytes(cov.overcounted)} więcej, niż zajmuje wolumen. Tak bywa przy klonach plików APFS (kopia
          nie zajmuje nowego miejsca, ale liczy się dwa razy) i przy skanie obejmującym inne wolumeny.
        </p>
      ) : null}
      {cov.unaccounted > 0 && !scanning ? (
        <>
          <p>Na „Poza skanem” składają się m.in.:</p>
          <ul className="disk-reasons">
            {!whole ? (
              <li>
                <span>
                  Pozostałe foldery na tym dysku – przeskanowano tylko <code>{info.rootPath}</code>.
                </span>
                <button className="btn small" onClick={() => startScan(scanTarget(vol), false)}>
                  Skanuj cały dysk
                </button>
              </li>
            ) : null}
            {info.skipped > 0 ? (
              <li>
                <span>{formatCount(info.skipped)} pominiętych ścieżek (brak uprawnień) – ich zawartość nie jest policzona.</span>
                <button className="btn small" onClick={() => setTab("skipped")}>
                  Pokaż
                </button>
              </li>
            ) : null}
            {apfs ? (
              <>
                <li>
                  Inne wolumeny tego samego kontenera APFS – system, VM (pamięć wymiany), Preboot, Recovery – dzielą to samo
                  miejsce.
                </li>
                <li>Migawki APFS i kopie lokalne Time Machine – nie są widoczne jako pliki.</li>
              </>
            ) : null}
            {OS === "windows" ? (
              <li>Plik stronicowania i hibernacji, punkty przywracania systemu i kopie w tle (VSS).</li>
            ) : null}
            {OS === "linux" ? <li>Bloki zarezerwowane dla administratora (np. 5% na ext4) i pliki innych użytkowników.</li> : null}
            {cov.purgeable > 0 ? (
              <li>{formatBytes(cov.purgeable)} do odzyskania – pamięć podręczna i kopie, które system zwolni sam, gdy zabraknie miejsca.</li>
            ) : null}
            <li>Metadane systemu plików (katalogi, dziennik, mapy wolnych bloków) – nie należą do żadnego pliku.</li>
          </ul>
        </>
      ) : null}
      {stale ? (
        <p className="muted">
          Skan z {formatDate(info.scannedAt)}, a dane dysku są bieżące – od tamtej pory zawartość mogła się zmienić.
        </p>
      ) : null}
    </>
  );
}

export function DiskPanel() {
  const info = useStore((s) => s.info);
  const volumes = useVolumes();
  if (!info) return null;
  const vol = info.volume;

  return (
    <div className="panel-scroll disk">
      {vol ? (
        <>
          <div className="disk-head">
            <div>
              <h2>{vol.name}</h2>
              <span className="muted">
                {vol.mountPath} · {vol.fsType.toUpperCase()}
              </span>
            </div>
            <div className="disk-free">
              <b>{formatBytes(vol.free)}</b> wolne z {formatBytes(vol.total)}
            </div>
          </div>
          <StackBar
            total={vol.total}
            segments={[
              { label: "Zajęte", value: vol.total - vol.free - (info.coverage?.purgeable ?? 0), cls: "used" },
              ...(info.coverage?.purgeable
                ? [{ label: "Do odzyskania", value: info.coverage.purgeable, cls: "purgeable", note: "zwalniane przez system" }]
                : []),
              { label: "Wolne", value: vol.free, cls: "free" },
            ]}
          />
          {vol.availableImportant !== null && vol.availableImportant > vol.free ? (
            <p className="muted">
              Finder pokazuje jako dostępne {formatBytes(vol.availableImportant)}: wolne miejsce plus to, które macOS zwolni sam.
            </p>
          ) : null}
          {info.coverage ? <CoverageSection info={info} vol={vol} cov={info.coverage} /> : null}
        </>
      ) : (
        <div className="callout">
          <p>
            Brak danych o wolumenie dla <code>{info.rootPath}</code> – ścieżka nie istnieje albo dysk jest odłączony (np. wczytany
            skan z innego komputera).
          </p>
        </div>
      )}

      {volumes.length > 0 ? (
        <>
          <h3>Wszystkie dyski</h3>
          <div className="volumes">
            {volumes.map((v) => (
              <VolumeTile
                key={v.path}
                volume={v}
                current={!!vol && (v.path === vol.mountPath || v.path === scanTarget(vol))}
                onClick={() => startScan(v.path, false)}
              />
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}

/** Wolne miejsce w stopce; kliknięcie otwiera zakładkę „Dysk”. */
export function DiskMini() {
  const vol = useStore((s) => s.info?.volume);
  const setTab = useStore((s) => s.setTab);
  if (!vol) return null;
  const used = vol.total - vol.free;
  return (
    <button
      className="disk-mini"
      onClick={() => setTab("disk")}
      title={`${vol.name}: ${formatBytes(used)} zajęte z ${formatBytes(vol.total)} – szczegóły w zakładce „Dysk”`}
    >
      <span>
        Wolne <b>{formatBytes(vol.free)}</b> z {formatBytes(vol.total)}
      </span>
      <span className="bar">
        <span className="fill" style={{ width: `${vol.total ? (used / vol.total) * 100 : 0}%` }} />
      </span>
    </button>
  );
}
