import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { Breadcrumb } from "./components/Breadcrumb";
import { FilterBar } from "./components/FilterBar";
import { FolderTree } from "./components/FolderTree";
import { ContextMenu, DetailsBar, Toasts, Welcome } from "./components/Overlays";
import { ConfirmTrashDialog } from "./components/TrashDialog";
import { Tabs } from "./components/Tabs";
import { Toolbar } from "./components/Toolbar";
import { UpdateDialog, useUpdateEvents } from "./components/Update";
import { chooseAndScan, refreshInfo, rescan, startScan } from "./lib/actions";
import { useStore } from "./store";

function isTyping(e: KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "SELECT" || el.tagName === "TEXTAREA");
}

function useProgressEvents() {
  useEffect(() => {
    const un = api.onProgress((p) => {
      const s = useStore.getState();
      if (s.info && p.scanId !== s.info.scanId) return;
      s.setProgress(p);
      // Footer totals and counters grow along with the scan.
      refreshInfo().then(() => useStore.getState().bump());
    });
    refreshInfo().then(async (info) => {
      const path = info ? null : await api.launchPath();
      if (path) startScan(path, false);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.key.toLowerCase() === "o") {
        e.preventDefault();
        chooseAndScan(false);
      } else if (meta && e.key.toLowerCase() === "r") {
        e.preventDefault();
        if (s.info && !s.scanning) rescan(s.selected ?? s.root);
      } else if (meta && e.key.toLowerCase() === "f") {
        e.preventDefault();
        s.focusSearch();
      } else if (meta && e.key === "Backspace") {
        if (isTyping(e)) return;
        e.preventDefault();
        if (s.selected !== null && !s.scanning) s.setTrashTarget({ ids: [s.selected], verify: false });
      } else if ((meta && e.key === "[") || (e.key === "Escape" && !isTyping(e) && !s.menu && s.trashTarget === null)) {
        e.preventDefault();
        s.goBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Tree/panel split remembered as a ratio, so that it fits any window width. */
function Split({ left, right }: { left: React.ReactNode; right: React.ReactNode }) {
  const [ratio, setRatio] = useState(() => {
    try {
      const v = Number(localStorage.getItem("split-ratio"));
      return v >= 0.2 && v <= 0.8 ? v : 0.5;
    } catch {
      return 0.5;
    }
  });
  const dragging = useRef(false);
  useEffect(() => {
    const move = (e: MouseEvent) => {
      if (!dragging.current) return;
      setRatio(Math.max(0.25, Math.min(0.75, e.clientX / window.innerWidth)));
    };
    const up = () => {
      if (!dragging.current) return;
      dragging.current = false;
      document.body.classList.remove("resizing");
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);
  useEffect(() => {
    try {
      localStorage.setItem("split-ratio", ratio.toFixed(3));
    } catch {
      /* no storage: not a problem */
    }
  }, [ratio]);
  return (
    <div className="split" style={{ gridTemplateColumns: `minmax(360px, ${ratio}fr) 6px minmax(360px, ${1 - ratio}fr)` }}>
      <section className="pane">{left}</section>
      <div
        className="gutter"
        role="separator"
        aria-orientation="vertical"
        onMouseDown={() => {
          dragging.current = true;
          document.body.classList.add("resizing");
        }}
      />
      <section className="pane">{right}</section>
    </div>
  );
}

export function App() {
  useProgressEvents();
  useShortcuts();
  useUpdateEvents();
  const info = useStore((s) => s.info);

  return (
    <div className="app">
      <Toolbar />
      {info ? (
        <>
          <div className="subbar">
            <Breadcrumb />
            <FilterBar />
          </div>
          <Split
            left={<FolderTree />}
            right={
              <Tabs />
            }
          />
          <DetailsBar />
        </>
      ) : (
        <Welcome />
      )}
      <ContextMenu />
      <ConfirmTrashDialog />
      <UpdateDialog />
      <Toasts />
    </div>
  );
}
