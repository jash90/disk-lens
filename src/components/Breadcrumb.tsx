import { useEffect, useState } from "react";
import { api, type Crumb } from "../api";
import { useStore } from "../store";
import { shortcut } from "../lib/platform";

export function Breadcrumb() {
  const root = useStore((s) => s.root);
  const version = useStore((s) => s.version);
  const back = useStore((s) => s.back);
  const goBack = useStore((s) => s.goBack);
  const navigate = useStore((s) => s.navigate);
  const [crumbs, setCrumbs] = useState<Crumb[]>([]);

  useEffect(() => {
    let alive = true;
    api
      .getAncestors(root)
      .then((c) => alive && setCrumbs(c))
      .catch(() => alive && setCrumbs([]));
    return () => {
      alive = false;
    };
  }, [root, version]);

  const parent = crumbs.length > 1 ? crumbs[crumbs.length - 2].id : null;

  return (
    <nav className="crumbs" aria-label="Ścieżka">
      <button className="btn icon" disabled={back.length === 0} onClick={goBack} title={`Wstecz (Esc, ${shortcut("[")})`}>
        ←
      </button>
      <button className="btn icon" disabled={parent === null} onClick={() => parent !== null && navigate(parent)} title="Folder nadrzędny">
        ↑
      </button>
      <ol>
        {crumbs.map((c, i) => (
          <li key={c.id}>
            <button className={i === crumbs.length - 1 ? "current" : ""} onClick={() => navigate(c.id)} title={c.name}>
              {c.name}
            </button>
          </li>
        ))}
      </ol>
    </nav>
  );
}
