import { volumeLabel } from "../i18n/volume";
import { t, useLocale } from "../i18n";
import { useEffect, useState } from "react";
import { api, type Volume } from "../api";
import { formatBytes } from "../lib/format";
import { useStore } from "../store";

/** Lista wolumenów; odświeżana przy każdej zmianie danych (skan, Kosz, ponowny skan). */
export function useVolumes(): Volume[] {
  const version = useStore((s) => s.version);
  const [volumes, setVolumes] = useState<Volume[]>([]);
  useEffect(() => {
    let alive = true;
    api
      .listVolumes()
      .then((v) => alive && setVolumes(v))
      .catch(() => alive && setVolumes([]));
    return () => {
      alive = false;
    };
  }, [version]);
  return volumes;
}

/** Kafelek wolumenu z paskiem zajętości. */
export function VolumeTile({ volume, current, onClick }: { volume: Volume; current?: boolean; onClick: () => void }) {
  useLocale();
  const used = volume.total - volume.free;
  return (
    <button className={current ? "volume current" : "volume"} onClick={onClick}>
      <span className="v-name">
        {volumeLabel(volume.name, volume.path)}
        {current ? <span className="v-tag">{t("bieżący skan")}</span> : null}
      </span>
      <span className="bar">
        <span className="fill" style={{ width: `${volume.total ? (used / volume.total) * 100 : 0}%` }} />
      </span>
      <span className="muted">
        {t("{used} zajęte z {total} · wolne {free}", { used: formatBytes(used), total: formatBytes(volume.total), free: formatBytes(volume.free) })}
      </span>
    </button>
  );
}
