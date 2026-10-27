// Import from Insomnia, Postman, OpenAPI or HAR: what the file brings, what was left out, and
// where it goes — a new collection or a folder of the open one.
import { useState } from "react";
import { CollectionInfo, ImportPreview } from "../api";
import { home } from "../platform";

export type ImportTarget = { kind: "current" } | { kind: "new" };

export function ImportDialog({
  path,
  preview,
  collection,
  busy,
  onImport,
  onCancel,
}: {
  path: string;
  preview: ImportPreview;
  collection: CollectionInfo | null;
  busy: boolean;
  onImport: (target: ImportTarget) => void;
  onCancel: () => void;
}) {
  const [target, setTarget] = useState<ImportTarget["kind"]>(collection ? "current" : "new");
  const onlyVars = preview.requests === 0;
  const what = [
    preview.requests > 0 && `${preview.requests} ${plural(preview.requests, "запрос", "запроса", "запросов")}`,
    preview.folders > 0 && `${preview.folders} ${plural(preview.folders, "папка", "папки", "папок")}`,
    preview.environments.length > 0 && `окружения: ${preview.environments.join(", ")}`,
    preview.vars > 0 && `${preview.vars} ${plural(preview.vars, "переменная", "переменные", "переменных")} коллекции`,
  ].filter(Boolean);

  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && !busy && onCancel()}>
      <div className="modal import-dialog" role="dialog" aria-label="Импорт" onKeyDown={(e) => e.key === "Escape" && !busy && onCancel()}>
        <h2>
          Импорт: {preview.format} «{preview.name}»
        </h2>
        <div className="faint small ell" title={path}>
          {home(path)}
        </div>
        <p>{what.length ? what.join(" · ") : "В файле нет ни запросов, ни переменных."}</p>
        {preview.warnings.length > 0 && (
          <ul className="import-warnings small">
            {preview.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}
        <fieldset className="import-target">
          <label className={collection ? "" : "off"}>
            <input type="radio" name="target" checked={target === "current"} disabled={!collection} onChange={() => setTarget("current")} />
            <span>
              В открытую коллекцию{collection && ` «${collection.name}»`}
              <span className="faint small">
                {onlyVars
                  ? " — окружения и переменные добавятся к её; что уже есть, останется как было"
                  : ` — в новую папку «${preview.name}»; переменные, которых в ней нет, добавятся`}
              </span>
            </span>
          </label>
          <label>
            <input type="radio" name="target" checked={target === "new"} onChange={() => setTarget("new")} />
            <span>
              Новой коллекцией<span className="faint small"> — папка «{preview.name}» там, где вы выберете</span>
            </span>
          </label>
        </fieldset>
        <p className="faint small">Секреты — токены, пароли, ключи — попадут в файлы *.local.toml, которые не уходят в git.</p>
        <div className="modal-actions">
          <button onClick={onCancel} disabled={busy}>
            Отмена
          </button>
          <button className="primary" autoFocus disabled={busy || what.length === 0} onClick={() => onImport({ kind: target })}>
            {busy ? "Импортирую…" : target === "new" ? "Выбрать папку и импортировать" : "Импортировать"}
          </button>
        </div>
      </div>
    </div>
  );
}

function plural(n: number, one: string, few: string, many: string): string {
  const d = n % 10;
  const dd = n % 100;
  if (d === 1 && dd !== 11) return one;
  if (d >= 2 && d <= 4 && (dd < 12 || dd > 14)) return few;
  return many;
}
