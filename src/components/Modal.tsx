import type { ReactNode } from "react";

/** Dialog over a backdrop. Clicking the backdrop or pressing Escape inside calls `onClose`. */
export function Modal({
  titleId,
  role = "dialog",
  className,
  onClose,
  children,
}: {
  titleId: string;
  role?: "dialog" | "alertdialog";
  className?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className={className ? `modal ${className}` : "modal"}
        role={role}
        aria-labelledby={titleId}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        {children}
      </div>
    </div>
  );
}
