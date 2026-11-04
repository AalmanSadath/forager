import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * The app's own dropdown, as in BifroSSH, rather than a `<select>`: the
 * toolkit draws a `<select>`'s list in its own colours, whatever the theme.
 */

interface AnchorRect {
  /** Where a menu hanging below the anchor starts. */
  top: number;
  /** The anchor's own top edge, for a menu that opens upward instead. */
  anchorTop: number;
  left: number;
  width: number;
}

function anchorBelow(el: Element): AnchorRect {
  const r = el.getBoundingClientRect();
  return { top: r.bottom + 2, anchorTop: r.top, left: r.left, width: r.width };
}

/** Space kept between the menu and the window's edge. */
const BOTTOM_MARGIN = 12;

/** Below this much room under the anchor the menu opens upward, if there is more above. */
const MIN_BELOW = 160;

/** Whether `el` can be seen inside every scrolling box it sits in. */
function inView(el: Element): boolean {
  const r = el.getBoundingClientRect();
  if (r.bottom < 0 || r.top > window.innerHeight) return false;
  for (let p = el.parentElement; p; p = p.parentElement) {
    const oy = getComputedStyle(p).overflowY;
    if (oy !== 'auto' && oy !== 'scroll' && oy !== 'hidden') continue;
    const c = p.getBoundingClientRect();
    if (r.bottom <= c.top || r.top >= c.bottom) return false;
  }
  return true;
}

/**
 * A menu portalled to `document.body` at a fixed position under `anchor`,
 * so panels that scroll and clip do not cut it off. It follows the anchor
 * as the page scrolls or the window resizes, hides while the anchor is
 * scrolled out of sight, and opens upward near the bottom of the window.
 * While more of the list lies below what is shown, a "⋯" row scrolls it.
 */
function PortalMenu({
  anchor,
  maxHeight,
  menuRef,
  children,
}: {
  anchor: Element;
  maxHeight?: number;
  menuRef?: React.Ref<HTMLDivElement>;
  children: ReactNode;
}) {
  const [rect, setRect] = useState<AnchorRect>(() => anchorBelow(anchor));
  const [visible, setVisible] = useState(true);
  const [more, setMore] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const checkMore = useCallback(() => {
    const el = scrollRef.current;
    if (el) setMore(el.scrollTop + el.clientHeight < el.scrollHeight - 1);
  }, []);

  useEffect(() => {
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        setRect(anchorBelow(anchor));
        setVisible(inView(anchor));
        checkMore();
      });
    };
    measure();
    // Capture, so a scroll in any panel is heard, not just the window's own.
    window.addEventListener('scroll', measure, true);
    window.addEventListener('resize', measure);
    const observer = new ResizeObserver(measure);
    observer.observe(anchor);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener('scroll', measure, true);
      window.removeEventListener('resize', measure);
      observer.disconnect();
    };
  }, [anchor, checkMore]);

  useLayoutEffect(checkMore);

  if (!visible) return null;
  const below = Math.max(0, window.innerHeight - rect.top - BOTTOM_MARGIN);
  const above = Math.max(0, rect.anchorTop - 2 - BOTTOM_MARGIN);
  const wanted = maxHeight ?? MIN_BELOW;
  const upward = below < Math.min(wanted, MIN_BELOW) && above > below;
  const available = upward ? above : below;

  const scrollPage = () => {
    const el = scrollRef.current;
    if (el) el.scrollBy({ top: Math.max(el.clientHeight - 30, 30), behavior: 'smooth' });
  };

  return createPortal(
    <div
      ref={menuRef}
      className="picker-menu picker-menu-portal"
      role="listbox"
      style={{
        position: 'fixed',
        ...(upward ? { top: 'auto', bottom: window.innerHeight - rect.anchorTop + 2 } : { top: rect.top }),
        left: rect.left,
        width: rect.width,
        maxHeight: maxHeight ? Math.min(maxHeight, available) : available,
        zIndex: 9999,
      }}
    >
      <div ref={scrollRef} className="picker-menu-scroll" onScroll={checkMore}>
        {children}
      </div>
      {more && (
        <button
          type="button"
          className="picker-more"
          aria-label="More"
          tabIndex={-1}
          onMouseDown={(e) => {
            e.preventDefault();
            scrollPage();
          }}
          onWheel={(e) => scrollRef.current?.scrollBy({ top: e.deltaY })}
        >
          ⋯
        </button>
      )}
    </div>,
    document.body,
  );
}

const CHEVRON = (
  <svg width="10" height="6" viewBox="0 0 10 6" fill="currentColor" aria-hidden="true">
    <path d="M0 0l5 6 5-6z" />
  </svg>
);

export interface DropdownOption<T extends string> {
  value: T;
  label: string;
}

/**
 * A `<select>`-shaped button with its menu portalled out of the layout.
 * Nothing covers the page while it is open: a press outside the button and
 * the menu closes it and still reaches what it landed on, and Escape
 * closes it too.
 */
export default function Dropdown<T extends string>({
  id,
  value,
  options,
  onChange,
  title,
  ariaLabel,
  className,
  maxHeight,
}: {
  id?: string;
  value: T;
  options: DropdownOption<T>[];
  onChange: (value: T) => void;
  title?: string;
  ariaLabel?: string;
  className?: string;
  maxHeight?: number;
}) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  const current = options.find((o) => o.value === value);

  const choose = (next: T) => {
    onChange(next);
    close();
    btnRef.current?.focus();
  };

  // Up and Down step through the options while the menu is closed, as on a
  // <select>.
  const step = (e: React.KeyboardEvent) => {
    if (open || (e.key !== 'ArrowDown' && e.key !== 'ArrowUp')) return;
    e.preventDefault();
    const at = options.findIndex((o) => o.value === value);
    const next = options[Math.min(options.length - 1, Math.max(0, at + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next && next.value !== value) onChange(next.value);
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (btnRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Only the menu: a dialog or page behind it keeps its own Escape.
        e.stopPropagation();
        close();
        btnRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [open, close]);

  return (
    <>
      <button
        ref={btnRef}
        id={id}
        type="button"
        className={`picker-btn${className ? ` ${className}` : ''}`}
        title={title}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={step}
      >
        <span className="picker-label">{current?.label ?? ''}</span>
        {CHEVRON}
      </button>
      {open && btnRef.current && (
        <PortalMenu anchor={btnRef.current} maxHeight={maxHeight} menuRef={menuRef}>
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              role="option"
              aria-selected={option.value === value}
              className={`picker-item${option.value === value ? ' selected' : ''}`}
              // Keeps focus on the button while pressed; the click chooses,
              // which Enter and Space on a focused row do too.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => choose(option.value)}
            >
              {option.label}
            </button>
          ))}
        </PortalMenu>
      )}
    </>
  );
}
