import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type NodeInfo, type TrashCheck, type Volume } from "../api";
import { chooseAndScan, confirmTrash, copyPath, openTerminal, rescan, reveal, startScan } from "../lib/actions";
import { filesLabel, foldersLabel, formatBytes, formatCount, formatDate, middleEllipsis } from "../lib/format";
import { useStore, useViewArgs } from "../store";
import { REVEAL_LABEL, shortcut } from "../lib/platform";

export function ContextMenu() {
  const menu = useStore((s) => s.menu);
  const openMenu = useStore((s) => s.openMenu);
  const navigate = useStore((s) => s.navigate);
  const setTrashTarget = useStore((s) => s.setTrashTarget);
  const scanning = useStore((s) => s.scanning);
  const view = useViewArgs();
  const [node, setNode] = useState<NodeInfo | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    let alive = true;
    api.getNode(menu.id, view).then((n) => alive && setNode(n)).catch(() => alive && setNode(null));
    const close = (e: Event) => {
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      openMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      alive = false;
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu, openMenu, view]);

  if (!menu) return null;
  const isDir = node?.kind === "dir" || node?.kind === "package";
  const run = (fn: () => void) => () => {
    openMenu(null);
    fn();
  };
  const left = Math.min(menu.x, window.innerWidth - 240);
  const top = Math.min(menu.y, window.innerHeight - 260);

  return (
    <div className="context-menu" ref={ref} style={{ left, top }} role="menu">
      {node ? <div className="menu-title">{node.name}</div> : null}
      {isDir ? (
        <button className="menu-item" onClick={run(() => navigate(menu.id))}>
          {node?.kind === "package" ? "Wejdź do środka pakietu" : "Otwórz jako korzeń widoku"}
        </button>
      ) : null}
      <button className="menu-item" onClick={run(() => reveal(menu.id))}>
        {REVEAL_LABEL}
      </button>
      <button className="menu-item" onClick={run(() => copyPath(menu.id))}>
        Kopiuj ścieżkę
      </button>
      <button className="menu-item" onClick={run(() => openTerminal(menu.id))}>
        Otwórz w Terminalu
      </button>
      {isDir ? (
        <button className="menu-item" disabled={scanning} onClick={run(() => rescan(menu.id))}>
          Przeskanuj ponownie
        </button>
      ) : null}
      <div className="menu-sep" />
      <button className="menu-item danger" disabled={scanning} onClick={run(() => setTrashTarget(menu.id))}>
        Przenieś do Kosza…
      </button>
    </div>
  );
}

export function ConfirmTrashDialog() {
  const id = useStore((s) => s.trashTarget);
  const setTrashTarget = useStore((s) => s.setTrashTarget);
  const [check, setCheck] = useState<TrashCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    setCheck(null);
    setError(null);
    if (id === null) return;
    api.trashCheck(id).then(setCheck).catch((e) => setError(errorMessage(e)));
  }, [id]);

  useEffect(() => {
    if (check && !check.blocked) confirmRef.current?.focus();
  }, [check]);

  if (id === null) return null;
  return (
    <div className="modal-backdrop" onMouseDown={() => setTrashTarget(null)}>
      <div
        className="modal"
        role="alertdialog"
        aria-labelledby="trash-title"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && setTrashTarget(null)}
      >
        <h2 id="trash-title">Przenieść do Kosza?</h2>
        {error ? <p className="error">{error}</p> : null}
        {check ? (
          <>
            <p className="path-box">{check.path}</p>
            <p>
              Zwolni ok. <b>{formatBytes(check.alloc)}</b>
              {check.files > 1 ? ` (${filesLabel(check.files)})` : ""}. Element trafi do Kosza – możesz go stamtąd
              przywrócić.
            </p>
            {check.blocked ? <p className="error">Zablokowane: {check.blocked}</p> : null}
          </>
        ) : !error ? (
          <p>Sprawdzam…</p>
        ) : null}
        <div className="modal-actions">
          <button className="btn" onClick={() => setTrashTarget(null)}>
            Anuluj
          </button>
          <button
            ref={confirmRef}
            className="btn danger"
            disabled={!check || !!check.blocked}
            onClick={() => confirmTrash(id)}
          >
            Przenieś do Kosza
          </button>
        </div>
      </div>
    </div>
  );
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  const busy = useStore((s) => s.busy);
  return (
    <div className="toasts" aria-live="polite">
      {busy ? (
        <div className="toast info">
          <span className="spinner" /> {busy}
        </div>
      ) : null}
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`} onClick={() => dismiss(t.id)}>
          {t.text}
        </div>
      ))}
    </div>
  );
}

export function Welcome() {
  const [volumes, setVolumes] = useState<Volume[]>([]);
  useEffect(() => {
    api.listVolumes().then(setVolumes).catch(() => setVolumes([]));
  }, []);
  return (
    <div className="welcome">
      <h1>Co zajmuje miejsce na dysku?</h1>
      <p className="muted">
        Wybierz folder albo cały wolumen. Zobaczysz treemapę (każdy plik to prostokąt o polu proporcjonalnym do rozmiaru) i drzewo
        folderów z sumami.
      </p>
      <button className="btn primary big" onClick={() => chooseAndScan(false)}>
        Wybierz folder… <kbd>{shortcut("O")}</kbd>
      </button>
      <div className="volumes">
        {volumes.map((v) => {
          const used = v.total - v.free;
          return (
            <button key={v.path} className="volume" onClick={() => startScan(v.path, false)}>
              <span className="v-name">{v.name}</span>
              <span className="bar">
                <span className="fill" style={{ width: `${v.total ? (used / v.total) * 100 : 0}%` }} />
              </span>
              <span className="muted">
                {formatBytes(used)} zajęte z {formatBytes(v.total)} · wolne {formatBytes(v.free)}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function DetailsBar() {
  const selected = useStore((s) => s.selected);
  const version = useStore((s) => s.version);
  const scanning = useStore((s) => s.scanning);
  const info = useStore((s) => s.info);
  const setTrashTarget = useStore((s) => s.setTrashTarget);
  const view = useViewArgs();
  const [node, setNode] = useState<NodeInfo | null>(null);

  useEffect(() => {
    if (selected === null) {
      setNode(null);
      return;
    }
    let alive = true;
    api.getNode(selected, view).then((n) => alive && setNode(n)).catch(() => alive && setNode(null));
    return () => {
      alive = false;
    };
  }, [selected, version, view]);

  if (!node) {
    return (
      <footer className="details">
        {info ? (
          <span className="muted">
            {info.rootPath} · {formatBytes(view.mode === "alloc" ? info.alloc : info.logical)} · {filesLabel(info.files)} ·{" "}
            {foldersLabel(info.dirs)}
            {info.skipped ? ` · ${formatCount(info.skipped)} pominiętych` : ""}
          </span>
        ) : null}
      </footer>
    );
  }
  const isDir = node.kind === "dir" || node.kind === "package";
  return (
    <footer className="details">
      <div className="d-main">
        <strong title={node.path}>{middleEllipsis(node.path, 120)}</strong>
        <span className="muted">
          {formatBytes(node.size)} ({view.mode === "alloc" ? `logicznie ${formatBytes(node.logical)}` : `zajęte ${formatBytes(node.alloc)}`})
          {isDir ? ` · ${filesLabel(node.files)} · ${foldersLabel(node.dirs)}` : ""} · zmieniono{" "}
          {formatDate(node.mtime)}
        </span>
      </div>
      <div className="d-actions">
        <button className="btn small" onClick={() => reveal(node.id)}>
          {REVEAL_LABEL}
        </button>
        <button className="btn small" onClick={() => copyPath(node.id)}>
          Kopiuj ścieżkę
        </button>
        <button className="btn small" onClick={() => openTerminal(node.id)}>
          Terminal
        </button>
        {isDir ? (
          <button className="btn small" disabled={scanning} onClick={() => rescan(node.id)} title={shortcut("R")}>
            Przeskanuj
          </button>
        ) : null}
        <button className="btn small danger" disabled={scanning} onClick={() => setTrashTarget(node.id)} title={shortcut("⌫")}>
          Do Kosza…
        </button>
      </div>
    </footer>
  );
}
