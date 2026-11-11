import { api, type Volume } from "../api";
import { formatBytes } from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { useStore } from "../store";

/** List of volumes, refreshed on every data change (scan, Trash, rescan). */
export function useVolumes(): Volume[] {
  const version = useStore((s) => s.version);
  return useAsync<Volume[]>(() => api.listVolumes(), [version]).data ?? [];
}

/** Volume tile with a usage bar. */
export function VolumeTile({ volume, current, onClick }: { volume: Volume; current?: boolean; onClick: () => void }) {
  const used = volume.total - volume.free;
  return (
    <button className={current ? "volume current" : "volume"} onClick={onClick}>
      <span className="v-name">
        {volume.name}
        {current ? <span className="v-tag">bieżący skan</span> : null}
      </span>
      <span className="bar">
        <span className="fill" style={{ width: `${volume.total ? (used / volume.total) * 100 : 0}%` }} />
      </span>
      <span className="muted">
        {formatBytes(used)} zajęte z {formatBytes(volume.total)} · wolne {formatBytes(volume.free)}
      </span>
    </button>
  );
}
