import { formatCount } from "../lib/format";
import { useStore, type Tab } from "../store";
import { DiskPanel } from "./DiskPanel";
import { DiffPanel } from "./panels/Diff";
import { SkippedPanel } from "./panels/Skipped";
import { TopFiles } from "./panels/TopFiles";
import { TypeBreakdownPanel } from "./panels/TypeBreakdown";
import { Treemap } from "./Treemap";

const TABS: { id: Tab; label: string }[] = [
  { id: "treemap", label: "Treemapa" },
  { id: "top", label: "Największe pliki" },
  { id: "types", label: "Wg typu" },
  { id: "skipped", label: "Pominięte" },
  { id: "diff", label: "Porównanie" },
  { id: "disk", label: "Dysk" },
];

/** Right-hand pane: the tab strip and the body of the active tab. */
export function Tabs() {
  const skipped = useStore((s) => s.info?.skipped ?? 0);
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  return (
    <div className="right">
      <div className="tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t.id} role="tab" aria-selected={tab === t.id} className={tab === t.id ? "on" : ""} onClick={() => setTab(t.id)}>
            {t.label}
            {t.id === "skipped" && skipped > 0 ? <span className="count">{formatCount(skipped)}</span> : null}
          </button>
        ))}
      </div>
      <div className="tab-body">
        {tab === "treemap" ? <Treemap /> : null}
        {tab === "top" ? <TopFiles /> : null}
        {tab === "types" ? <TypeBreakdownPanel /> : null}
        {tab === "skipped" ? <SkippedPanel /> : null}
        {tab === "diff" ? <DiffPanel /> : null}
        {tab === "disk" ? <DiskPanel /> : null}
      </div>
    </div>
  );
}
