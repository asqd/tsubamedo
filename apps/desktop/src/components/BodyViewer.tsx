// The response body viewer, built for bodies of hundreds of megabytes: the core keeps the text
// (raw or pretty JSON in a temporary file) and an index; this component asks only for the lines
// it shows. Scrolling is its own (a browser cannot scroll 28 million rows of 18 px), folding
// hides line ranges, and search runs in the core over the whole view.
import { ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, Line, ResponseMeta, SearchResult, ViewInfo } from "../api";
import { count, noun, size } from "../format";
import { t } from "../i18n";
import { mod, pickSavePath } from "../platform";

const ROW = 18;
/** Rows fetched above and below the screen, so scrolling rarely waits for the core. */
const OVERSCAN = 120;
const CACHE_LIMIT = 20_000;

/** Collapsed ranges as the row mapping needs them: sorted outermost ranges and, for each,
 *  how many lines the ranges before it hide. Lookups are binary searches, so folding every
 *  object of a big array still scrolls smoothly. */
interface Folding {
  opens: number[];
  closes: number[];
  before: number[];
  hidden: number;
}

function folding(collapsed: Map<number, number>): Folding {
  const f: Folding = { opens: [], closes: [], before: [], hidden: 0 };
  let last = -1;
  for (const [open, close] of [...collapsed.entries()].sort((a, b) => a[0] - b[0])) {
    if (open <= last) continue;
    f.opens.push(open);
    f.closes.push(close);
    f.before.push(f.hidden);
    f.hidden += close - open;
    last = close;
  }
  return f;
}

/** The last range whose key is below `value`, or -1. */
function lastBelow(n: number, key: (k: number) => number, value: number): number {
  let lo = 0;
  let hi = n;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (key(mid) < value) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

/** A collapsed range shows its opening line; the lines after it, up to the close, are hidden. */
function rowToLine(row: number, f: Folding): number {
  const k = lastBelow(f.opens.length, (i) => f.opens[i] - f.before[i], row);
  return k < 0 ? row : row + f.before[k] + f.closes[k] - f.opens[k];
}

function lineToRow(line: number, f: Folding): number {
  const k = lastBelow(f.opens.length, (i) => f.opens[i], line);
  if (k < 0) return line;
  if (line <= f.closes[k]) return f.opens[k] - f.before[k];
  return line - f.before[k] - (f.closes[k] - f.opens[k]);
}

const TOKEN = /("(?:[^"\\]|\\.)*")(?=\s*:)|("(?:[^"\\]|\\.)*"?)|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g;

/** JSON colors and search marks; only visible lines are ever tokenized. */
function renderLine(text: string, json: boolean, marks: [number, number][], current: boolean): ReactNode {
  if (!json && marks.length === 0) return text;
  const n = text.length;
  const cls: string[] = new Array(n).fill("");
  if (json) {
    for (const m of text.matchAll(TOKEN)) {
      const c = m[1] ? "j-key" : m[2] ? "j-str" : m[3] ? "j-num" : "j-lit";
      cls.fill(c, m.index!, m.index! + m[0].length);
    }
  }
  const marked = new Uint8Array(n);
  for (const [s, e] of marks) marked.fill(1, s, Math.min(e, n));
  const out: ReactNode[] = [];
  let start = 0;
  for (let i = 1; i <= n; i++) {
    if (i < n && cls[i] === cls[start] && marked[i] === marked[start]) continue;
    let c = cls[start];
    if (marked[start]) c += current ? " mark cur" : " mark";
    const piece = text.slice(start, i);
    out.push(c ? <span key={start} className={c}>{piece}</span> : piece);
    start = i;
  }
  return out;
}

export function BodyViewer({ meta }: { meta: ResponseMeta }) {
  const [pretty, setPretty] = useState(meta.isJson);
  const [info, setInfo] = useState<ViewInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [top, setTop] = useState(0);
  const [scrollX, setScrollX] = useState(0);
  const [collapsed, setCollapsed] = useState<Map<number, number>>(new Map());
  const [query, setQuery] = useState("");
  const [applied, setApplied] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [found, setFound] = useState<SearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [current, setCurrent] = useState(-1);
  const [box, setBox] = useState({ width: 600, height: 400 });
  const [charWidth, setCharWidth] = useState(7.4);
  const [flash, setFlash] = useState<string | null>(null);
  const [, setTick] = useState(0);

  const cache = useRef(new Map<number, Line>());
  const pending = useRef(new Set<number>());
  const generation = useRef(0);
  const viewport = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const measure = useRef<HTMLSpanElement>(null);

  const resetLines = useCallback(() => {
    generation.current++;
    cache.current.clear();
    pending.current.clear();
  }, []);

  // The view itself: pretty JSON is formatted in the core, which takes a moment on big bodies.
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    api
      .view(meta.responseId, pretty)
      .then((v) => {
        if (!alive) return;
        resetLines();
        setInfo(v);
        setTop(0);
        setScrollX(0);
        setCollapsed(new Map());
        setFound(null);
        setCurrent(-1);
      })
      .catch((e) => alive && setError(String(e.message ?? e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [meta.responseId, pretty, resetLines]);

  useLayoutEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setBox({ width: el.clientWidth, height: el.clientHeight }));
    observer.observe(el);
    if (measure.current) setCharWidth(measure.current.getBoundingClientRect().width / 20 || 7.4);
    return () => observer.disconnect();
  }, [info]);

  const ranges = useMemo(() => folding(collapsed), [collapsed]);
  const totalRows = info ? info.lines - ranges.hidden : 0;
  const pageRows = Math.max(1, Math.floor(box.height / ROW));
  const maxTop = Math.max(0, totalRows - pageRows);
  const first = Math.floor(top);
  const shown: number[] = [];
  for (let r = first; r < Math.min(totalRows, first + pageRows + 2); r++) shown.push(rowToLine(r, ranges));
  const gutter = String(info?.lines ?? 1).length * charWidth + 34;
  const widest = shown.reduce((w, n) => Math.max(w, cache.current.get(n)?.text.length ?? 0), 0);
  const maxX = Math.max(0, widest * charWidth + gutter + 48 - box.width);

  const limits = useRef({ maxTop, maxX, pageRows });
  limits.current = { maxTop, maxX, pageRows };
  const clampTop = (t: number) => Math.min(Math.max(0, t), limits.current.maxTop);

  // Lines around the screen that are not cached yet.
  useEffect(() => {
    if (!info) return;
    const want: number[] = [];
    const from = Math.max(0, first - OVERSCAN);
    const to = Math.min(totalRows, first + pageRows + OVERSCAN);
    for (let r = from; r < to; r++) {
      const n = rowToLine(r, ranges);
      if (!cache.current.has(n) && !pending.current.has(n)) want.push(n);
    }
    if (want.length === 0) return;
    want.forEach((n) => pending.current.add(n));
    const gen = generation.current;
    api
      .viewLines(info.viewId, want, applied || null, caseSensitive)
      .then((lines) => {
        if (gen !== generation.current) return;
        const c = cache.current;
        if (c.size > CACHE_LIMIT) {
          const keep = new Set(shown);
          for (const n of c.keys()) {
            if (c.size <= CACHE_LIMIT / 2) break;
            if (!keep.has(n)) c.delete(n);
          }
        }
        for (const line of lines) c.set(line.n, line);
        want.forEach((n) => pending.current.delete(n));
        setTick((t) => t + 1);
      })
      .catch(() => want.forEach((n) => pending.current.delete(n)));
  });

  // Search: typed text settles for a moment, then the core scans the whole view.
  useEffect(() => {
    const t = setTimeout(() => setApplied(query), 250);
    return () => clearTimeout(t);
  }, [query]);

  const goTo = useCallback(
    (index: number, result: SearchResult | null = found) => {
      if (!result || result.lines.length === 0) return;
      const i = (index + result.lines.length) % result.lines.length;
      const line = result.lines[i];
      setCurrent(i);
      setCollapsed((prev) => {
        const next = new Map([...prev].filter(([o, c]) => !(o < line && line <= c)));
        const r = folding(next);
        const row = lineToRow(line, r);
        const rows = (info?.lines ?? 0) - r.hidden;
        const max = Math.max(0, rows - limits.current.pageRows);
        const visible = row >= top && row < top + limits.current.pageRows - 1;
        if (!visible) setTop(Math.min(max, Math.max(0, row - Math.floor(limits.current.pageRows / 3))));
        return next.size === prev.size ? prev : next;
      });
    },
    [found, info, top],
  );

  useEffect(() => {
    if (!info) return;
    resetLines();
    setTick((t) => t + 1);
    if (!applied) {
      setFound(null);
      setCurrent(-1);
      return;
    }
    let alive = true;
    setSearching(true);
    api
      .viewSearch(info.viewId, applied, caseSensitive)
      .then((result) => {
        if (!alive) return;
        setFound(result);
        const here = rowToLine(Math.floor(top), ranges);
        const next = result.lines.findIndex((n) => n >= here);
        if (result.lines.length) goTo(next < 0 ? 0 : next, result);
        else setCurrent(-1);
      })
      .catch((e) => alive && setError(String(e.message ?? e)))
      .finally(() => alive && setSearching(false));
    return () => {
      alive = false;
    };
    // Only a new query or view starts a search; scrolling must not.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied, caseSensitive, info]);

  // Wheel: rows vertically, pixels horizontally. Passive listeners cannot cancel the page bounce.
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const unit = e.deltaMode === 1 ? ROW : e.deltaMode === 2 ? el.clientHeight : 1;
      let dx = e.deltaX * unit;
      let dy = e.deltaY * unit;
      if (e.shiftKey && dx === 0) [dx, dy] = [dy, 0];
      if (dy) setTop((t) => clampTop(t + dy / ROW));
      if (dx) setScrollX((x) => Math.min(Math.max(0, x + dx), limits.current.maxX));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [info]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const page = limits.current.pageRows - 1;
    const moves: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page, " ": page };
    if (mod(e) && e.key === "f") {
      e.preventDefault();
      searchInput.current?.focus();
      searchInput.current?.select();
    } else if (e.key === "Home" || (mod(e) && e.key === "ArrowUp")) {
      e.preventDefault();
      setTop(0);
    } else if (e.key === "End" || (mod(e) && e.key === "ArrowDown")) {
      e.preventDefault();
      setTop(limits.current.maxTop);
    } else if (e.key in moves) {
      e.preventDefault();
      setTop((t) => clampTop(Math.round(t) + moves[e.key]));
    } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      e.preventDefault();
      const dx = (e.key === "ArrowRight" ? 1 : -1) * charWidth * 8;
      setScrollX((x) => Math.min(Math.max(0, x + dx), limits.current.maxX));
    }
  };

  const toggleFold = (line: Line) => {
    if (line.foldEnd == null) return;
    setCollapsed((prev) => {
      const next = new Map(prev);
      if (next.has(line.n)) next.delete(line.n);
      else next.set(line.n, line.foldEnd!);
      return next;
    });
  };

  /** Folds the objects and arrays inside the top one: the quickest way to see the shape of
   *  a big answer. The core finds them, so it works on lines never scrolled to. */
  const foldAll = async (expand: boolean) => {
    if (expand || !info) return setCollapsed(new Map());
    try {
      const flat = await api.viewFolds(info.viewId);
      const next = new Map<number, number>();
      for (let i = 0; i < flat.length; i += 2) next.set(flat[i], flat[i + 1]);
      setCollapsed(next);
      setTop(0);
    } catch (e) {
      setFlash(String((e as Error).message ?? e));
      setTimeout(() => setFlash(null), 4000);
    }
  };

  const copy = async () => {
    try {
      const text = await api.bodyText(meta.responseId, info?.pretty ? info.viewId : null);
      await navigator.clipboard.writeText(text);
      setFlash(t("Скопировано", "Copied"));
    } catch (e) {
      setFlash(String((e as Error).message ?? e));
    }
    setTimeout(() => setFlash(null), 2500);
  };

  const saveAs = async () => {
    const ext = meta.isJson ? "json" : "txt";
    const path = await pickSavePath(`response-${meta.responseId}.${ext}`);
    if (!path) return;
    try {
      await api.saveBody(meta.responseId, info?.pretty ? info.viewId : null, path);
      setFlash(t("Сохранено", "Saved"));
    } catch (e) {
      setFlash(String((e as Error).message ?? e));
    }
    setTimeout(() => setFlash(null), 2500);
  };

  // Custom scrollbar.
  const track = box.height;
  const thumb = totalRows > 0 ? Math.max(28, (track * Math.min(pageRows, totalRows)) / totalRows) : track;
  const thumbY = maxTop > 0 ? (top / maxTop) * (track - thumb) : 0;
  const dragThumb = (e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startY = e.clientY;
    const startTop = top;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const free = track - thumb;
      if (free > 0) setTop(clampTop(startTop + ((ev.clientY - startY) / free) * limits.current.maxTop));
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };
  const jumpTrack = (e: React.PointerEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setTop(clampTop(((e.clientY - rect.top - thumb / 2) / (track - thumb)) * maxTop));
  };

  const currentLine = found && current >= 0 ? found.lines[current] : -1;
  const json = Boolean(info?.pretty) || meta.isJson;

  return (
    <div className="viewer">
      <div className="viewer-bar">
        <div className="seg">
          <button className={pretty ? "on" : ""} onClick={() => setPretty(true)} title={t("JSON с отступами и сворачиванием", "Indented, foldable JSON")}>
            {t("Красиво", "Pretty")}
          </button>
          <button className={!pretty ? "on" : ""} onClick={() => setPretty(false)} title={t("Тело как пришло", "The body as it came")}>
            {t("Как есть", "Raw")}
          </button>
        </div>
        {info?.pretty && (
          <div className="seg">
            <button onClick={() => foldAll(false)} title={t("Свернуть объекты верхнего уровня", "Collapse top-level objects")}>
              {t("Свернуть", "Collapse")}
            </button>
            <button onClick={() => foldAll(true)} title={t("Развернуть всё", "Expand all")}>
              {t("Развернуть", "Expand")}
            </button>
          </div>
        )}
        <span className="faint small num ell">
          {info && `${count(info.lines)} ${noun(info.lines, ["строка", "строки", "строк"], ["line", "lines"])} · ${size(info.bytes)}`}
        </span>
      </div>
      <div className="viewer-find">
        <div className="search">
          <input
            ref={searchInput}
            value={query}
            placeholder={t("Поиск в ответе", "Search the response")}
            spellCheck={false}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (query !== applied) setApplied(query);
                else goTo(current + (e.shiftKey ? -1 : 1));
              } else if (e.key === "Escape") {
                setQuery("");
                viewport.current?.focus();
              }
            }}
          />
          <button className={`case ${caseSensitive ? "on" : ""}`} onClick={() => setCaseSensitive(!caseSensitive)} title={t("Учитывать регистр", "Match case")}>
            Aa
          </button>
          <span className="small muted num hits">
            {searching ? <span className="spin" /> : found ? (
              found.total === 0
                ? t("нет", "no matches")
                : t(
                    `${current + 1} из ${count(found.lines.length)}${found.lines.length >= 100_000 ? "+" : ""} стр. · ${count(found.total)}${found.capped ? "+" : ""}`,
                    `${current + 1} of ${count(found.lines.length)}${found.lines.length >= 100_000 ? "+" : ""} ${found.lines.length === 1 ? "line" : "lines"} · ${count(found.total)}${found.capped ? "+" : ""}`,
                  )
            ) : null}
          </span>
          <button className="icon" disabled={!found?.lines.length} onClick={() => goTo(current - 1)} title={t("Предыдущее (⇧↵)", "Previous (⇧↵)")}>
            ↑
          </button>
          <button className="icon" disabled={!found?.lines.length} onClick={() => goTo(current + 1)} title={t("Следующее (↵)", "Next (↵)")}>
            ↓
          </button>
        </div>
        <button className="ghost" onClick={copy} title={t("Скопировать тело (до 20 МБ)", "Copy the body (up to 20 MB)")}>
          {t("Копировать", "Copy")}
        </button>
        <button className="ghost" onClick={saveAs} title={t("Сохранить тело в файл", "Save the body to a file")}>
          {t("Сохранить…", "Save…")}
        </button>
      </div>
      {info?.note && <div className="viewer-note small">{info.note}</div>}
      {flash && <div className="flash small">{flash}</div>}
      <div className="viewport mono" ref={viewport} tabIndex={0} onKeyDown={onKeyDown}>
        <span className="measure" ref={measure}>
          MMMMMMMMMMMMMMMMMMMM
        </span>
        {loading && (
          <div className="viewer-wait muted">
            <span className="spin" /> {pretty ? t(`Форматирую ${size(meta.size)}…`, `Formatting ${size(meta.size)}…`) : t("Открываю…", "Opening…")}
          </div>
        )}
        {error && <div className="viewer-wait bad-text">{error}</div>}
        {!loading && info && info.bytes === 0 && <div className="viewer-wait faint">{t("Пустое тело", "Empty body")}</div>}
        {!loading &&
          info &&
          info.bytes > 0 &&
          shown.map((n, i) => {
            const line = cache.current.get(n);
            const folded = collapsed.has(n);
            const closer = line?.text.trimEnd().endsWith("[") ? "]" : "}";
            return (
              <div key={n} className={`vrow ${n === currentLine ? "cur" : ""}`} style={{ top: (first + i - top) * ROW }}>
                <span className="gut" style={{ width: gutter }}>
                  <span className="ln">{n + 1}</span>
                  <span className={`fold ${line?.foldEnd != null ? "has" : ""}`} onClick={() => line && toggleFold(line)}>
                    {line?.foldEnd != null ? (folded ? "▸" : "▾") : ""}
                  </span>
                </span>
                <span className="txt" style={{ transform: `translateX(${-scrollX}px)` }}>
                  {line ? renderLine(line.text, json, line.marks, n === currentLine) : <span className="faint">·</span>}
                  {folded && line && (
                    <span className="folded" onClick={() => toggleFold(line)}>
                      … {closer}{" "}
                      <i>
                        {count(line.foldEnd! - line.n - 1)} {noun(line.foldEnd! - line.n - 1, ["строка", "строки", "строк"], ["line", "lines"])}
                      </i>
                    </span>
                  )}
                </span>
              </div>
            );
          })}
        {totalRows > pageRows && (
          <div className="vscroll" onPointerDown={jumpTrack}>
            <div className="thumb" style={{ top: thumbY, height: thumb }} onPointerDown={dragThumb} />
          </div>
        )}
      </div>
    </div>
  );
}
