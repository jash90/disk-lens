import { useEffect, useRef, useState } from "react";
import { api, errorMessage, type TrashCheck, type TrashOutcome, type TrashProblem } from "../api";
import { moveToTrash } from "../lib/actions";
import { filesLabel, formatBytes, formatCount, itemsLabel } from "../lib/format";
import { useStore } from "../store";
import { Modal } from "./Modal";

/** At most this many paths are listed in the dialog; the rest is summed up in one line. */
const LISTED = 8;

function ProblemList({ title, items }: { title: string; items: TrashProblem[] }) {
  if (items.length === 0) return null;
  return (
    <div className="trash-problems">
      <p className="error">{title}</p>
      <ul>
        {items.slice(0, LISTED).map((p) => (
          <li key={p.path}>
            <span className="path">{p.path}</span>
            <span className="muted"> – {p.reason}</span>
          </li>
        ))}
      </ul>
      {items.length > LISTED ? <p className="muted">…i {formatCount(items.length - LISTED)} więcej</p> : null}
    </div>
  );
}

/** Confirmation of a move to the Trash, for one item or a whole selection, and its result. */
export function ConfirmTrashDialog() {
  const request = useStore((s) => s.trashTarget);
  const setTrashTarget = useStore((s) => s.setTrashTarget);
  const toast = useStore((s) => s.toast);
  const [check, setCheck] = useState<TrashCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<TrashOutcome | null>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    setCheck(null);
    setError(null);
    setOutcome(null);
    setRunning(false);
    if (!request) return;
    let alive = true;
    api
      .trashCheck(request.ids)
      .then((c) => alive && setCheck(c))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [request]);

  useEffect(() => {
    if (check && check.count > 0) confirmRef.current?.focus();
  }, [check]);

  if (!request) return null;
  const close = () => {
    if (!running) setTrashTarget(null);
  };

  const confirm = async () => {
    setRunning(true);
    const result = await moveToTrash(request.ids, request.verify);
    setRunning(false);
    if (!result) return;
    if (result.skipped.length === 0 && result.failed.length === 0) {
      setTrashTarget(null);
      toast(`Przeniesiono do Kosza (${formatBytes(result.freed)}).`, "success");
    } else {
      setOutcome(result);
    }
  };

  if (outcome) {
    return (
      <Modal titleId="trash-title" role="alertdialog" onClose={close}>
        <h2 id="trash-title">Nie wszystko trafiło do Kosza</h2>
        <p>
          Przeniesiono: <b>{itemsLabel(outcome.trashed)}</b> ({formatBytes(outcome.freed)}).
        </p>
        <ProblemList title={`Pominięto: ${itemsLabel(outcome.skipped.length)}`} items={outcome.skipped} />
        <ProblemList title={`Nie udało się przenieść: ${itemsLabel(outcome.failed.length)}`} items={outcome.failed} />
        <div className="modal-actions">
          <button className="btn" onClick={close}>
            Zamknij
          </button>
        </div>
      </Modal>
    );
  }

  const many = check !== null && check.count > 1;
  return (
    <Modal titleId="trash-title" role="alertdialog" onClose={close}>
      <h2 id="trash-title">Przenieść do Kosza?</h2>
      {error ? <p className="error">{error}</p> : null}
      {check ? (
        <>
          {check.count > 0 ? (
            <>
              <div className="path-box">
                {check.sample.map((p) => (
                  <div key={p}>{p}</div>
                ))}
                {check.count > check.sample.length ? (
                  <div className="muted">…i {formatCount(check.count - check.sample.length)} więcej</div>
                ) : null}
              </div>
              <p>
                Zwolni ok. <b>{formatBytes(check.alloc)}</b>
                {many ? ` (${itemsLabel(check.count)}, ${filesLabel(check.files)})` : check.files > 1 ? ` (${filesLabel(check.files)})` : ""}.{" "}
                {many ? "Elementy trafią" : "Element trafi"} do Kosza – możesz {many ? "je" : "go"} stamtąd przywrócić.
              </p>
              {request.verify ? (
                <p className="muted">Elementy zmienione po skanie zostaną pominięte.</p>
              ) : null}
            </>
          ) : null}
          <ProblemList title={check.count > 0 ? "Te elementy zostaną pominięte:" : "Zablokowane:"} items={check.blocked} />
        </>
      ) : !error ? (
        <p>Sprawdzam…</p>
      ) : null}
      <div className="modal-actions">
        <button className="btn" disabled={running} onClick={close}>
          Anuluj
        </button>
        <button ref={confirmRef} className="btn danger" disabled={!check || check.count === 0 || running} onClick={confirm}>
          {running ? "Przenoszę…" : "Przenieś do Kosza"}
        </button>
      </div>
    </Modal>
  );
}
