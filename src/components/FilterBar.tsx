import { useEffect, useRef, useState } from "react";
import { EMPTY_FILTER, filterActive } from "../api";
import { useStore } from "../store";
import { shortcut } from "../lib/platform";

const MIN_SIZES = [
  { label: "dowolny", value: 0 },
  { label: "≥ 1 MB", value: 1e6 },
  { label: "≥ 10 MB", value: 1e7 },
  { label: "≥ 100 MB", value: 1e8 },
  { label: "≥ 1 GB", value: 1e9 },
];

const SUGGESTED_EXCLUDES = ["node_modules", ".git", "target", "Library", ".cache"];

function splitList(s: string): string[] {
  return s
    .split(/[,\s]+/)
    .map((x) => x.trim().replace(/^\./, ""))
    .filter(Boolean);
}

export function FilterBar() {
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const searchFocus = useStore((s) => s.searchFocus);
  const [query, setQuery] = useState(filter.query);
  const [exts, setExts] = useState(filter.extensions.join(", "));
  const [excludeInput, setExcludeInput] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (searchFocus > 0) {
      searchRef.current?.focus();
      searchRef.current?.select();
    }
  }, [searchFocus]);

  // Pola tekstowe stosujemy z opóźnieniem, żeby nie przeliczać przy każdym znaku.
  useEffect(() => {
    const t = setTimeout(() => {
      const extensions = splitList(exts).map((e) => e.toLowerCase());
      const cur = useStore.getState().filter;
      if (cur.query !== query || cur.extensions.join() !== extensions.join()) {
        setFilter({ ...cur, query, extensions });
      }
    }, 300);
    return () => clearTimeout(t);
  }, [query, exts, setFilter]);

  const addExclude = (name: string) => {
    const n = name.trim();
    if (!n || filter.excludes.includes(n)) return;
    setFilter({ ...filter, excludes: [...filter.excludes, n] });
  };

  const active = filterActive(filter);

  return (
    <div className={`filterbar ${active ? "active" : ""}`}>
      <input
        ref={searchRef}
        className="search"
        type="search"
        placeholder={`Szukaj nazwy pliku (${shortcut("F")})`}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && (setQuery(""), (e.target as HTMLInputElement).blur())}
      />
      <label className="field">
        Rozmiar
        <select value={filter.minSize} onChange={(e) => setFilter({ ...filter, minSize: Number(e.target.value) })}>
          {MIN_SIZES.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      <input
        className="exts"
        placeholder="Rozszerzenia, np. mp4, mov"
        value={exts}
        onChange={(e) => setExts(e.target.value)}
        title="Pokaż tylko pliki o tych rozszerzeniach"
      />
      <div className="excludes" title="Pomijane w całości (foldery lub pliki o tej nazwie)">
        <span className="muted">Pomiń:</span>
        {filter.excludes.map((x) => (
          <span key={x} className="chip">
            {x}
            <button
              aria-label={`Usuń ${x}`}
              onClick={() => setFilter({ ...filter, excludes: filter.excludes.filter((e) => e !== x) })}
            >
              ×
            </button>
          </span>
        ))}
        <input
          className="exclude-input"
          list="exclude-suggestions"
          placeholder="nazwa…"
          value={excludeInput}
          onChange={(e) => setExcludeInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              addExclude(excludeInput);
              setExcludeInput("");
            }
          }}
        />
        <datalist id="exclude-suggestions">
          {SUGGESTED_EXCLUDES.filter((s) => !filter.excludes.includes(s)).map((s) => (
            <option key={s} value={s} />
          ))}
        </datalist>
      </div>
      {active ? (
        <button
          className="btn small"
          onClick={() => {
            setQuery("");
            setExts("");
            setFilter(EMPTY_FILTER);
          }}
        >
          Wyczyść filtry
        </button>
      ) : null}
    </div>
  );
}
