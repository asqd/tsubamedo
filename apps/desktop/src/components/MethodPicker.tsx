// The method next to the address: a button in the look of the window with its own list, since a
// native select draws the system's control in the middle of the bar. Keys work as in a select:
// arrows and Enter open and walk the list, a letter picks the next method starting with it (by
// key position, so a Russian layout works too).
import { useEffect, useRef, useState } from "react";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function MethodPicker({ value, onChange }: { value: string; onChange: (method: string) => void }) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const list = METHODS.includes(value) ? METHODS : [value, ...METHODS];

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const close = () => setOpen(false);
    window.addEventListener("mousedown", away);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("blur", close);
    };
  }, [open]);

  const show = () => {
    setHover(list.indexOf(value));
    setOpen(true);
  };
  const pick = (method: string) => {
    onChange(method);
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const at = open ? hover : list.indexOf(value);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) return show();
      setHover((at + (e.key === "ArrowDown" ? 1 : list.length - 1)) % list.length);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (open) pick(list[hover]);
      else show();
    } else if (e.key === "Escape" && open) {
      e.stopPropagation();
      setOpen(false);
    } else if (e.code.startsWith("Key") && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // P again and again: POST, PUT, PATCH.
      const letter = e.code.slice(3);
      const found = [...list.slice(at + 1), ...list.slice(0, at + 1)].find((m) => m.startsWith(letter));
      if (!found) return;
      e.preventDefault();
      if (open) setHover(list.indexOf(found));
      else onChange(found);
    }
  };

  return (
    <div className="method-picker" ref={ref}>
      <button
        className={`method m-${value.toLowerCase()}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        title="Метод запроса"
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
      >
        <span className="ell">{value}</span>
        <svg className="chevron" width="9" height="6" viewBox="0 0 9 6" aria-hidden="true">
          <path d="M1 1l3.5 3.5L8 1" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && (
        <div className="menu method-menu" role="listbox">
          {list.map((m, i) => (
            <button
              key={m}
              role="option"
              aria-selected={m === value}
              tabIndex={-1}
              className={`menu-item m-${m.toLowerCase()} ${i === hover ? "hover" : ""}`}
              onMouseEnter={() => setHover(i)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(m)}
            >
              <span>{m}</span>
              {m === value && <span aria-hidden="true">✓</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
