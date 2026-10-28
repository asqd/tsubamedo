// The request as code: curl, Python, JavaScript, Go and the rest, with variables substituted.
// The language picked last is picked again next time.
import { useEffect, useState } from "react";
import { api, Language, Request } from "../api";
import { t } from "../i18n";
import { LanguagePicker } from "./LanguagePicker";

function storedLang(): string {
  try {
    return localStorage.getItem("code-lang") ?? "curl";
  } catch {
    return "curl";
  }
}

export function CodeDialog({ id, request, env, onClose }: { id: string | null; request: Request; env: string | null; onClose: () => void }) {
  const [languages, setLanguages] = useState<Language[]>([]);
  const [lang, setLang] = useState(storedLang);
  const [code, setCode] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.languages().then(setLanguages, () => {});
  }, []);
  useEffect(() => {
    setProblem(null);
    setCopied(false);
    try {
      localStorage.setItem("code-lang", lang);
    } catch {
      // Only a convenience.
    }
    api.code(id, request, env, lang).then(setCode, (e) => {
      setCode(null);
      setProblem((e as Error).message);
    });
  }, [lang, id, request, env]);

  const copy = async () => {
    if (code == null) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal code-dialog" role="dialog" aria-label={t("Код", "Code")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <div className="code-head">
          <h2>{t("Код запроса", "Request code")}</h2>
          <LanguagePicker languages={languages} value={lang} onChange={setLang} />
          <span className="sp" />
          <button className="icon" title={t("Закрыть", "Close")} onClick={onClose}>
            ×
          </button>
        </div>
        <p className="faint small">
          {env
            ? t(`С подставленными переменными окружения ${env}; незаданные — как <имя>. Файлы multipart и тела — по их путям.`, `With variables of the ${env} environment filled in; unset ones as <name>. Multipart files and bodies by their paths.`)
            : t("С подставленными переменными; незаданные — как <имя>. Файлы multipart и тела — по их путям.", "With variables filled in; unset ones as <name>. Multipart files and bodies by their paths.")}
        </p>
        {problem && <div className="error-box small">{problem}</div>}
        {code != null && <pre className="code-view mono sel">{code}</pre>}
        <div className="modal-actions">
          <span className="faint small">{copied ? t("Скопировано", "Copied") : ""}</span>
          <button onClick={onClose}>{t("Закрыть", "Close")}</button>
          <button className="primary" autoFocus onClick={copy} disabled={code == null}>
            {t("Скопировать", "Copy")}
          </button>
        </div>
      </div>
    </div>
  );
}
