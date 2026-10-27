// The request as code: curl, Python, JavaScript, Go and the rest, with variables substituted.
// The language picked last is picked again next time.
import { useEffect, useState } from "react";
import { api, Language, Request } from "../api";

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

  const groups = [...new Set(languages.map((l) => l.group))];
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
      <div className="modal code-dialog" role="dialog" aria-label="Код" onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <div className="code-head">
          <h2>Код запроса</h2>
          <select value={lang} onChange={(e) => setLang(e.target.value)} aria-label="Язык">
            {groups.map((g) => (
              <optgroup key={g} label={g}>
                {languages
                  .filter((l) => l.group === g)
                  .map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.group === l.name || l.group === "Shell" ? l.name : `${l.group} — ${l.name}`}
                    </option>
                  ))}
              </optgroup>
            ))}
          </select>
          <span className="sp" />
          <button className="icon" title="Закрыть" onClick={onClose}>
            ×
          </button>
        </div>
        <p className="faint small">
          С подставленными переменными{env ? ` окружения ${env}` : ""}; незаданные — как &lt;имя&gt;. Файлы multipart и тела — по их путям.
        </p>
        {problem && <div className="error-box small">{problem}</div>}
        {code != null && <pre className="code-view mono sel">{code}</pre>}
        <div className="modal-actions">
          <span className="faint small">{copied ? "Скопировано" : ""}</span>
          <button onClick={onClose}>Закрыть</button>
          <button className="primary" autoFocus onClick={copy} disabled={code == null}>
            Скопировать
          </button>
        </div>
      </div>
    </div>
  );
}
