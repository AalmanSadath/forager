import type { FormEvent, ReactNode } from 'react';

/**
 * The centred dialog shell, as in BifroSSH: backdrop, panel, header. Leaving
 * out `onClose` makes it blocking, for work that has to finish or be answered.
 */
export default function Modal({
  title,
  subtitle,
  className,
  onClose,
  onSubmit,
  children,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  className?: string;
  onClose?: () => void;
  onSubmit?: (e: FormEvent) => void;
  children: ReactNode;
}) {
  const body = (
    <>
      <div className="modal-header">
        <div>
          <h2>{title}</h2>
          {subtitle && <div className="modal-subtitle">{subtitle}</div>}
        </div>
      </div>
      {children}
    </>
  );
  // Clicks inside reach the backdrop by bubbling and would close the dialog.
  const stop = onClose ? (e: React.MouseEvent) => e.stopPropagation() : undefined;
  const cls = `modal${className ? ` ${className}` : ''}`;
  return (
    <div className="modal-overlay" onClick={onClose}>
      {onSubmit ? (
        <form className={cls} onClick={stop} onSubmit={onSubmit}>
          {body}
        </form>
      ) : (
        <div className={cls} onClick={stop}>
          {body}
        </div>
      )}
    </div>
  );
}
