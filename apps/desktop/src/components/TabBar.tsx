// Open requests as tabs. A tab opened from the sidebar and not touched yet is shown in italics:
// the next request picked in the sidebar takes its place.
import { t } from "../i18n";
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
      {tabs.map((tab) => {
        const method = tab.request.method.toUpperCase();
        return (
          <div
            key={tab.key}
            role="tab"
            aria-selected={tab.key === active}
            className={`tab ${tab.key === active ? "on" : ""} ${tab.pristine ? "pristine" : ""}`}
            title={tab.id ?? t("Черновик: живёт только в окне", "Draft: lives only in this window")}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                onClose(tab.key);
              } else if (e.button === 0) onSelect(tab.key);
            }}
          >
            <span className={`m m-${method.toLowerCase()}`}>{method === "DELETE" ? "DEL" : method.slice(0, 4)}</span>
            <span className="ell">{tabTitle(tab)}</span>
            {tab.sendId && <span className="spin" />}
            {!tab.id && <span className="faint scratch-mark">{t("черновик", "draft")}</span>}
            <button
              className="close"
              title={t("Закрыть (⌘W)", "Close (⌘W)")}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.key);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
      <button className="icon new-tab" title={t("Новый черновик (⌘T)", "New draft (⌘T)")} onClick={onNew}>
        +
      </button>
    </div>
  );
}
