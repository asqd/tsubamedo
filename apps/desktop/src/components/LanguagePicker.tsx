// The language of «Код…»: a button in the look of the window with its own list, grouped as the
// core groups the languages, like the method picker next to the address. Keys work as in a
// select: arrows and Enter open and walk the list, a letter goes to the next language starting
// with it (by key position, so a Russian layout works too).
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Language } from "../api";

/** «Python — requests»; a group of one name, and the shell, by the name alone. */
export const languageLabel = (l: Language) => (l.group === l.name || l.group === "Shell" ? l.name : `${l.group} — ${l.name}`);

export function LanguagePicker({ languages, value, onChange }: { languages: Language[]; value: string; onChange: (id: string) => void }) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(0);
  const [place, setPlace] = useState<{ top: number; left: number; maxHeight: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const at = Math.max(0, languages.findIndex((l) => l.id === value));
  const current = languages[at];

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

  // The dialog scrolls and would cut a list inside it, so the list is placed by the window.
  useLayoutEffect(() => {
    if (!open || !button.current) return;
    const r = button.current.getBoundingClientRect();
    setPlace({ top: r.bottom + 4, left: r.left, maxHeight: window.innerHeight - r.bottom - 16 });
  }, [open]);
  useEffect(() => {
    if (open) menu.current?.querySelector<HTMLElement>(`[data-i="${hover}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, hover, place]);

  const show = () => {
    setHover(at);
    setOpen(true);
  };
  const pick = (id: string) => {
    onChange(id);
    setOpen(false);
    button.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (languages.length === 0) return;
    const from = open ? hover : at;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) return show();
      setHover((from + (e.key === "ArrowDown" ? 1 : languages.length - 1)) % languages.length);
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      if (open) setHover(e.key === "Home" ? 0 : languages.length - 1);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (open) pick(languages[hover].id);
      else show();
    } else if (e.key === "Escape" && open) {
      // The dialog closes on Escape too: only the list does.
      e.stopPropagation();
      setOpen(false);
    } else if (e.code.startsWith("Key") && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // J again and again: JavaScript fetch, axios, XMLHttpRequest, then Java.
      const letter = e.code.slice(3);
      const order = [...languages.slice(from + 1), ...languages.slice(0, from + 1)];
      const found = order.find((l) => languageLabel(l).toUpperCase().startsWith(letter));
      if (!found) return;
      e.preventDefault();
      if (open) setHover(languages.indexOf(found));
      else onChange(found.id);
    }
  };

  const groups = [...new Set(languages.map((l) => l.group))];
  return (
    <div className="language-picker">
      <button
        ref={button}
        className="language"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label="Язык"
        title="Язык и библиотека"
        disabled={languages.length === 0}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
      >
        <span className="ell">{current ? languageLabel(current) : "…"}</span>
        <svg className="chevron" width="9" height="6" viewBox="0 0 9 6" aria-hidden="true">
          <path d="M1 1l3.5 3.5L8 1" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && place && (
        <div ref={menu} className="menu language-menu" role="listbox" style={place}>
          {groups.map((g) => (
            <div key={g} role="group" aria-label={g}>
              <div className="menu-group">{g}</div>
              {languages.map((l, i) =>
                l.group !== g ? null : (
                  <button
                    key={l.id}
                    data-i={i}
                    role="option"
                    aria-selected={l.id === value}
                    tabIndex={-1}
                    className={`menu-item ${i === hover ? "hover" : ""}`}
                    onMouseEnter={() => setHover(i)}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => pick(l.id)}
                  >
                    <span>{l.name}</span>
                    {l.id === value && <span aria-hidden="true">✓</span>}
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
