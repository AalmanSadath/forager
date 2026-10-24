import type { ReactNode } from 'react';

/** The small "are you sure" box, for destructive actions, as in BifroSSH. */
export default function ConfirmModal({
  question,
  hint,
  children,
  confirmLabel = 'Remove',
  onConfirm,
  onCancel,
}: {
  question: ReactNode;
  hint?: ReactNode;
  /** Extra controls, such as an "also uninstall" checkbox. */
  children?: ReactNode;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <>
      <div className="modal-overlay confirm-overlay" onClick={onCancel} />
      <div className="confirm-modal" role="dialog">
        <p>{question}</p>
        {hint && <p className="form-hint" style={{ margin: 0 }}>{hint}</p>}
        {children}
        <div className="confirm-actions">
          <button className="btn-secondary btn-sm" onClick={onCancel}>Cancel</button>
          <button className="btn-danger btn-sm" onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </div>
    </>
  );
}
