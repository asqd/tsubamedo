// Import from Insomnia, Postman, OpenAPI or HAR: what the file brings, what was left out, and
// where it goes — a new collection or a folder of the open one.
import { useState } from "react";
import { CollectionInfo, ImportPreview } from "../api";
import { noun } from "../format";
import { t } from "../i18n";
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
    preview.requests > 0 && `${preview.requests} ${noun(preview.requests, ["запрос", "запроса", "запросов"], ["request", "requests"])}`,
    preview.folders > 0 && `${preview.folders} ${noun(preview.folders, ["папка", "папки", "папок"], ["folder", "folders"])}`,
    preview.environments.length > 0 && t(`окружения: ${preview.environments.join(", ")}`, `environments: ${preview.environments.join(", ")}`),
    preview.vars > 0 &&
      t(
        `${preview.vars} ${noun(preview.vars, ["переменная", "переменные", "переменных"], ["variable", "variables"])} коллекции`,
        `${preview.vars} collection ${noun(preview.vars, ["переменная", "переменные", "переменных"], ["variable", "variables"])}`,
      ),
  ].filter(Boolean);

  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && !busy && onCancel()}>
      <div className="modal import-dialog" role="dialog" aria-label={t("Импорт", "Import")} onKeyDown={(e) => e.key === "Escape" && !busy && onCancel()}>
        <h2>
          {t(`Импорт: ${preview.format} «${preview.name}»`, `Import: ${preview.format} "${preview.name}"`)}
        </h2>
        <div className="faint small ell" title={path}>
          {home(path)}
        </div>
        <p>{what.length ? what.join(" · ") : t("В файле нет ни запросов, ни переменных.", "The file has no requests and no variables.")}</p>
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
              {t("В открытую коллекцию", "Into the open collection")}
              {collection && t(` «${collection.name}»`, ` "${collection.name}"`)}
              <span className="faint small">
                {onlyVars
                  ? t(
                      " — окружения и переменные добавятся к её; что уже есть, останется как было",
                      " — environments and variables are added to its own; what is already there stays as it was",
                    )
                  : t(
                      ` — в новую папку «${preview.name}»; переменные, которых в ней нет, добавятся`,
                      ` — into a new folder "${preview.name}"; variables it lacks are added`,
                    )}
              </span>
            </span>
          </label>
          <label>
            <input type="radio" name="target" checked={target === "new"} onChange={() => setTarget("new")} />
            <span>
              {t("Новой коллекцией", "As a new collection")}
              <span className="faint small">{t(` — папка «${preview.name}» там, где вы выберете`, ` — a folder "${preview.name}" wherever you choose`)}</span>
            </span>
          </label>
        </fieldset>
        <p className="faint small">
          {t(
            "Секреты — токены, пароли, ключи — попадут в файлы *.local.toml, которые не уходят в git.",
            "Secrets — tokens, passwords, keys — go into *.local.toml files, which stay out of git.",
          )}
        </p>
        <div className="modal-actions">
          <button onClick={onCancel} disabled={busy}>
            {t("Отмена", "Cancel")}
          </button>
          <button className="primary" autoFocus disabled={busy || what.length === 0} onClick={() => onImport({ kind: target })}>
            {busy ? t("Импортирую…", "Importing…") : target === "new" ? t("Выбрать папку и импортировать", "Choose folder and import") : t("Импортировать", "Import")}
          </button>
        </div>
      </div>
    </div>
  );
}
