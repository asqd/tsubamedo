// A right-click menu drawn by the page: native menus cannot be built from the web view here,
// and this one also asks "Точно?" in place before anything goes to the Trash.
import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface MenuItem {
  label: string;
  onClick?: () => void;
  /** Asks again in place before running. */
  confirm?: string;
  danger?: boolean;
  disabled?: boolean;
  submenu?: MenuItem[];
  separator?: boolean;
  indent?: number;
}

export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const [asking, setAsking] = useState<number | null>(null);
  const [open, setOpen] = useState<number | null>(null);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    setPos({ x: Math.min(x, window.innerWidth - r.width - 6), y: Math.min(y, window.innerHeight - r.height - 6) });
  }, [x, y]);

  useEffect(() => {
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", key);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", key);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  const run = (item: MenuItem) => {
    item.onClick?.();
    onClose();
  };

  return (
    <div ref={ref} className="menu" role="menu" style={{ left: pos.x, top: pos.y }} onContextMenu={(e) => e.preventDefault()}>
      {items.map((item, i) =>
        item.separator ? (
          <div key={i} className="menu-sep" />
        ) : asking === i ? (
          <div key={i} className="menu-ask">
            <div className="small">{item.confirm}</div>
            <div className="menu-ask-row">
              <button className="danger" autoFocus onClick={() => run(item)}>
                Да
              </button>
              <button onClick={() => setAsking(null)}>Нет</button>
            </div>
          </div>
        ) : (
          <div key={i} className="menu-entry" onMouseEnter={() => setOpen(item.submenu ? i : null)}>
            <button
              role="menuitem"
              className={`menu-item ${item.danger ? "danger" : ""}`}
              disabled={item.disabled}
              style={item.indent ? { paddingLeft: 12 + item.indent * 12 } : undefined}
              onClick={() => (item.submenu ? setOpen(open === i ? null : i) : item.confirm ? setAsking(i) : run(item))}
            >
              <span className="ell">{item.label}</span>
              {item.submenu && <span className="faint">▸</span>}
            </button>
            {item.submenu && open === i && (
              <div className="menu submenu" role="menu">
                {item.submenu.map((sub, j) => (
                  <button
                    key={j}
                    role="menuitem"
                    className="menu-item"
                    disabled={sub.disabled}
                    style={sub.indent ? { paddingLeft: 12 + sub.indent * 12 } : undefined}
                    onClick={() => run(sub)}
                  >
                    <span className="ell">{sub.label}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        ),
      )}
    </div>
  );
}
