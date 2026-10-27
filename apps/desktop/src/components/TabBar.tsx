// Open requests as tabs. A tab opened from the sidebar and not touched yet is shown in italics:
// the next request picked in the sidebar takes its place.
import { Tab, tabTitle } from "../tabs";

export function TabBar({
  tabs,
  active,
  onSelect,
  onClose,
  onNew,
}: {
  tabs: Tab[];
  active: string;
  onSelect: (key: string) => void;
  onClose: (key: string) => void;
  onNew: () => void;
}) {
  return (
    <div className="tabbar" onDoubleClick={(e) => e.target === e.currentTarget && onNew()}>
      {tabs.map((t) => {
        const method = t.request.method.toUpperCase();
        return (
          <div
            key={t.key}
            role="tab"
            aria-selected={t.key === active}
            className={`tab ${t.key === active ? "on" : ""} ${t.pristine ? "pristine" : ""}`}
            title={t.id ?? "Черновик: живёт только в окне"}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                onClose(t.key);
              } else if (e.button === 0) onSelect(t.key);
            }}
          >
            <span className={`m m-${method.toLowerCase()}`}>{method === "DELETE" ? "DEL" : method.slice(0, 4)}</span>
            <span className="ell">{tabTitle(t)}</span>
            {t.sendId && <span className="spin" />}
            {!t.id && <span className="faint scratch-mark">черновик</span>}
            <button
              className="close"
              title="Закрыть (⌘W)"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.key);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
      <button className="icon new-tab" title="Новый черновик (⌘T)" onClick={onNew}>
        +
      </button>
    </div>
  );
}
