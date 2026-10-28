// A choice from a list in the look of the window: a button and its own list, grouped if the
// options say so, instead of a native select that draws the system's control. Keys work as in a
// select: arrows, Home/End and Enter open and walk the list, a letter goes to the next option
// starting with it (by key position, so a Russian layout works too), Escape closes the list only.
import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface PickerOption {
  value: string;
  /** On the button, and in the list unless `item` says otherwise. */
  label: string;
  /** In the list, when the group heading already says part of the label. */
  item?: string;
  group?: string;
  /** Shown quieter: "no environment" and the like. */
  faint?: boolean;
}

export function Picker({
  options,
  value,
  onChange,
  label,
  title,
  className = "",
}: {
  options: PickerOption[];
  value: string;
  onChange: (value: string) => void;
  /** For screen readers: what is chosen. */
  label: string;
  title?: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(0);
  const [place, setPlace] = useState<React.CSSProperties | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const at = Math.max(0, options.findIndex((o) => o.value === value));
  const current = options[at];

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !button.current?.contains(e.target as Node) && !menu.current?.contains(e.target as Node) && setOpen(false);
    const close = () => setOpen(false);
    window.addEventListener("mousedown", away);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [open]);

  // Placed by the window, not inside the parent: a dialog scrolls and would cut the list. Near
  // the right edge it hangs from the button's right side.
  useLayoutEffect(() => {
    if (!open || !button.current) return;
    const r = button.current.getBoundingClientRect();
    const side = r.left > window.innerWidth / 2 ? { right: window.innerWidth - r.right } : { left: r.left };
    setPlace({ top: r.bottom + 4, maxHeight: window.innerHeight - r.bottom - 16, minWidth: r.width, ...side });
  }, [open]);
  useEffect(() => {
    if (open) menu.current?.querySelector<HTMLElement>(`[data-i="${hover}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, hover, place]);

  const show = () => {
    setHover(at);
    setOpen(true);
  };
  const pick = (v: string) => {
    onChange(v);
    setOpen(false);
    button.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (options.length === 0) return;
    const from = open ? hover : at;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) return show();
      setHover((from + (e.key === "ArrowDown" ? 1 : options.length - 1)) % options.length);
    } else if ((e.key === "Home" || e.key === "End") && open) {
      e.preventDefault();
      setHover(e.key === "Home" ? 0 : options.length - 1);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (open) pick(options[hover].value);
      else show();
    } else if (e.key === "Escape" && open) {
      e.stopPropagation();
      setOpen(false);
    } else if (e.code.startsWith("Key") && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // J again and again: JavaScript fetch, axios, XMLHttpRequest, then Java.
      const letter = e.code.slice(3);
      const order = [...options.slice(from + 1), ...options.slice(0, from + 1)];
      const found = order.find((o) => o.label.toUpperCase().startsWith(letter));
      if (!found) return;
      e.preventDefault();
      if (open) setHover(options.indexOf(found));
      else onChange(found.value);
    }
  };

  // Options of a group stand together, in the order they come.
  const groups = [...new Set(options.map((o) => o.group ?? ""))];
  return (
    <div className={`picker ${className}`}>
      <button
        ref={button}
        className="picker-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        title={title}
        disabled={options.length === 0}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
      >
        <span className={`ell ${current?.faint ? "faint" : ""}`}>{current ? current.label : "…"}</span>
        <svg className="chevron" width="9" height="6" viewBox="0 0 9 6" aria-hidden="true">
          <path d="M1 1l3.5 3.5L8 1" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && place && (
        <div ref={menu} className="menu picker-menu" role="listbox" style={place}>
          {groups.map((g) => (
            <div key={g} role="group" aria-label={g || undefined}>
              {g && <div className="menu-group">{g}</div>}
              {options.map((o, i) =>
                (o.group ?? "") !== g ? null : (
                  <button
                    key={o.value}
                    data-i={i}
                    role="option"
                    aria-selected={o.value === value}
                    tabIndex={-1}
                    className={`menu-item ${g ? "in-group" : ""} ${o.faint ? "faint" : ""} ${i === hover ? "hover" : ""}`}
                    onMouseEnter={() => setHover(i)}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => pick(o.value)}
                  >
                    <span>{o.item ?? o.label}</span>
                    {o.value === value && <span aria-hidden="true">✓</span>}
                  </button>
                ),
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
