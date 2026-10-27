// CodeMirror 6 for every field where {{variables}} live: one-line fields (address, values) and
// the body. Variables are colored (red when not defined), explained on hover and suggested
// after "{{".
import { useEffect, useRef } from "react";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  CompletionContext,
  CompletionResult,
  completionStatus,
  startCompletion,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { bracketMatching, foldGutter, foldKeymap, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { Annotation, Compartment, EditorState, Extension, Facet, Prec, Transaction } from "@codemirror/state";
import {
  Decoration,
  DecorationSet,
  drawSelection,
  EditorView,
  hoverTooltip,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  MatchDecorator,
  placeholder as placeholderExt,
  ViewPlugin,
  ViewUpdate,
} from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { BUILTINS, lookup, shown, useVars, VAR_PATTERN, VarInfo } from "../vars";

export type EditorMode = "line" | "json" | "text" | "js";

const varsFacet = Facet.define<VarInfo[], VarInfo[]>({ combine: (values) => values[0] ?? [] });
/** Marks changes that came from React (another request opened), not from typing. */
const external = Annotation.define<boolean>();

const varDecorator = new MatchDecorator({
  regexp: new RegExp(VAR_PATTERN.source, "g"),
  decoration: (match, view) =>
    Decoration.mark({ class: lookup(match[1], view.state.facet(varsFacet)) ? "cm-var" : "cm-var cm-var-missing" }),
});

const varHighlighter = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = varDecorator.createDeco(view);
    }
    update(u: ViewUpdate) {
      this.decorations =
        u.startState.facet(varsFacet) !== u.state.facet(varsFacet)
          ? varDecorator.createDeco(u.view)
          : varDecorator.updateDeco(u, this.decorations);
    }
  },
  { decorations: (p) => p.decorations },
);

const varTooltip = hoverTooltip((view, pos) => {
  const line = view.state.doc.lineAt(pos);
  for (const m of line.text.matchAll(new RegExp(VAR_PATTERN.source, "g"))) {
    const from = line.from + m.index!;
    const to = from + m[0].length;
    if (pos < from || pos > to) continue;
    const v = lookup(m[1], view.state.facet(varsFacet));
    return {
      pos: from,
      end: to,
      above: true,
      create: () => {
        const dom = document.createElement("div");
        dom.className = "var-tip";
        const name = dom.appendChild(document.createElement("b"));
        name.textContent = m[1];
        const value = dom.appendChild(document.createElement("div"));
        value.className = v ? "var-value" : "var-missing";
        value.textContent = v ? shown(v) || (v.source.startsWith("из ответа") ? "ещё не получено" : "(пусто)") : "не задана в этом окружении";
        if (v) {
          const source = dom.appendChild(document.createElement("div"));
          source.className = "var-source";
          source.textContent = v.source;
        }
        return { dom };
      },
    };
  }
  return null;
});

function varCompletion(ctx: CompletionContext): CompletionResult | null {
  const before = ctx.matchBefore(/\{\{\s*[\w.$-]*$/);
  if (!before) return null;
  const from = before.from + before.text.search(/[\w.$-]*$/);
  const closed = ctx.state.sliceDoc(ctx.pos, ctx.pos + 2) === "}}";
  const vars = ctx.state.facet(varsFacet);
  return {
    from,
    options: [...vars, ...BUILTINS].map((v) => ({
      label: v.name,
      detail: v.source,
      info: v.secret ? shown(v) : v.value,
      type: v.name.startsWith("$") ? "keyword" : "variable",
      boost: v.name.startsWith("$") ? -1 : 0,
      apply: closed || v.name.endsWith(".") ? v.name : `${v.name}}}`,
    })),
    validFor: /^[\w.$-]*$/,
  };
}

/** What request scripts can call (see core/tsubamedo-core/src/scripts/prelude.js). */
const pairsApi: [string, string][] = [
  ["get", "get(имя) — значение или undefined"],
  ["set", "set(имя, значение)"],
  ["has", "has(имя)"],
  ["remove", "remove(имя)"],
  ["all", "all() — объект имя → значение"],
];
const SCRIPT_API: Record<string, [string, string][]> = {
  "": [
    ["vars", "переменные: get, set, has, render"],
    ["request", "запрос: method, url, headers, query, body"],
    ["response", "ответ (в скрипте после): status, headers, time, size, text(), json()"],
    ["test", 'test("имя", () => expect(…).toBe(…))'],
    ["expect", "expect(значение).toBe(…), .not.toBe(…)…"],
    ["console", "console.log(…) — в журнал под ответом"],
    ["crypto", "sha1, sha256, sha512, hmac, randomUUID"],
    ["btoa", "btoa(текст) — в base64"],
    ["atob", "atob(base64) — обратно в текст"],
  ],
  "vars.": [
    ["get", "get(имя) — значение переменной"],
    ["set", "set(имя, значение) — для этого и других запросов"],
    ["has", "has(имя)"],
    ["render", "render(текст) — подставить {{переменные}}"],
  ],
  "request.": [
    ["method", "метод"],
    ["url", "адрес, как в файле — с {{переменными}}"],
    ["headers", "заголовки: get, set, has, remove, all"],
    ["query", "параметры адреса: get, set, has, remove, all"],
    ["body", "тело JSON или текст; можно присвоить объект"],
  ],
  "request.headers.": pairsApi,
  "request.query.": pairsApi,
  "response.headers.": pairsApi.filter(([n]) => n !== "set" && n !== "remove"),
  "response.": [
    ["status", "код ответа"],
    ["statusText", "текст статуса"],
    ["time", "мс от отправки до последнего байта"],
    ["size", "байт тела"],
    ["headers", "заголовки: get, has, all"],
    ["text", "text() — тело текстом"],
    ["json", "json() — тело как объект"],
  ],
  "console.": [
    ["log", "log(…)"],
    ["warn", "warn(…)"],
    ["error", "error(…)"],
  ],
  "crypto.": [
    ["sha256", 'sha256(текст, "hex" | "base64")'],
    ["sha512", "sha512(текст, кодировка)"],
    ["sha1", "sha1(текст, кодировка)"],
    ["hmac", 'hmac("sha256", ключ, текст, "hex" | "base64")'],
    ["randomUUID", "randomUUID()"],
  ],
};
const MATCHERS = [
  "toBe", "toEqual", "toBeTruthy", "toBeFalsy", "toBeDefined", "toBeUndefined", "toBeNull", "toContain", "toMatch",
  "toHaveLength", "toHaveProperty", "toBeGreaterThan", "toBeGreaterThanOrEqual", "toBeLessThan", "toBeLessThanOrEqual", "not",
];

function scriptCompletion(ctx: CompletionContext): CompletionResult | null {
  const matcher = ctx.matchBefore(/expect\([^()]*(?:\([^()]*\))?[^()]*\)\.(?:not\.)?[\w$]*$/);
  if (matcher) {
    const from = matcher.from + matcher.text.lastIndexOf(".") + 1;
    return { from, options: MATCHERS.map((m) => ({ label: m, type: "method" })), validFor: /^[\w$]*$/ };
  }
  const word = ctx.matchBefore(/(?:[\w$]+\.)*[\w$]*$/);
  if (!word) return null;
  const dot = word.text.lastIndexOf(".");
  const options = SCRIPT_API[word.text.slice(0, dot + 1)];
  if (!options || (dot < 0 && word.text.length === 0 && !ctx.explicit)) return null;
  return {
    from: word.from + dot + 1,
    options: options.map(([label, info]) => ({ label, detail: info, type: dot < 0 ? "variable" : "property" })),
    validFor: /^[\w$]*$/,
  };
}

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.definitionKeyword, t.operatorKeyword, t.moduleKeyword], color: "var(--m-patch)" },
  { tag: t.comment, color: "var(--faint)", fontStyle: "italic" },
  { tag: t.regexp, color: "var(--j-lit)" },
  { tag: t.propertyName, color: "var(--j-key)" },
  { tag: t.string, color: "var(--j-str)" },
  { tag: t.number, color: "var(--j-num)" },
  { tag: [t.bool, t.null], color: "var(--j-lit)" },
  { tag: t.invalid, color: "var(--bad)" },
]);

const theme = EditorView.theme({
  "&": { fontSize: "12px", color: "var(--fg)", backgroundColor: "transparent" },
  "&.cm-focused": { outline: "none" },
  ".cm-content": { fontFamily: "var(--mono)", caretColor: "var(--fg)" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.55" },
  ".cm-gutters": { backgroundColor: "var(--panel)", color: "var(--faint)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--sunken)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--sunken)", color: "var(--muted)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": { backgroundColor: "var(--accent-weak) !important" },
  ".cm-cursor": { borderLeftColor: "var(--fg)" },
  ".cm-matchingBracket": { backgroundColor: "var(--accent-weak)", outline: "1px solid var(--line-strong)" },
  ".cm-foldGutter span": { color: "var(--faint)", padding: "0 3px" },
  ".cm-placeholder": { color: "var(--faint)" },
  ".cm-tooltip": { backgroundColor: "var(--panel)", border: "1px solid var(--line-strong)", borderRadius: "6px", boxShadow: "0 8px 24px rgba(0,0,0,.18)" },
  ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--mono)", fontSize: "12px", maxHeight: "16em" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--accent-weak)", color: "var(--fg)" },
  ".cm-completionDetail": { color: "var(--faint)", fontStyle: "normal", marginLeft: "1em", fontFamily: "var(--sans)" },
  ".cm-completionInfo": { fontFamily: "var(--mono)", fontSize: "12px", maxWidth: "320px", wordBreak: "break-all" },
  ".cm-panels": { backgroundColor: "var(--sunken)", color: "var(--fg)", borderColor: "var(--line)" },
});

const oneLine: Extension = [
  EditorState.transactionFilter.of((tr) => {
    if (!tr.docChanged || tr.newDoc.lines === 1) return tr;
    const flat = tr.newDoc.toString().replace(/\r?\n/g, "");
    return [tr, { changes: { from: 0, to: tr.newDoc.length, insert: flat }, sequential: true }];
  }),
  EditorView.contentAttributes.of({ spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
];

function modeExtensions(mode: EditorMode): Extension[] {
  const sources = mode === "js" ? [varCompletion, scriptCompletion] : [varCompletion];
  const common: Extension[] = [history(), drawSelection(), theme, varHighlighter, varTooltip, autocompletion({ override: sources, icons: false })];
  if (mode === "line") {
    return [...common, oneLine, keymap.of([...completionKeymap, ...historyKeymap, ...defaultKeymap])];
  }
  return [
    ...common,
    lineNumbers(),
    foldGutter(),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    bracketMatching(),
    closeBrackets(),
    indentOnInput(),
    highlightSelectionMatches(),
    search({ top: true }),
    syntaxHighlighting(highlight),
    EditorState.tabSize.of(2),
    EditorView.contentAttributes.of({ spellcheck: "false" }),
    keymap.of([...closeBracketsKeymap, ...completionKeymap, ...searchKeymap, ...foldKeymap, ...historyKeymap, ...defaultKeymap, indentWithTab]),
    ...(mode === "json" ? [json()] : []),
    ...(mode === "js" ? [javascript()] : []),
  ];
}

export function CodeEditor({
  value,
  onChange,
  mode,
  placeholder,
  onSubmit,
  onPaste,
  autoFocus,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  mode: EditorMode;
  placeholder?: string;
  /** Enter in a one-line field, ⌘↵ anywhere. */
  onSubmit?: () => void;
  /** Return true when the pasted text was taken care of (a curl command, say). */
  onPaste?: (text: string) => boolean;
  autoFocus?: boolean;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const props = useRef({ onChange, onSubmit, onPaste });
  props.current = { onChange, onSubmit, onPaste };
  const vars = useVars();
  const varsSlot = useRef(new Compartment());
  const placeholderSlot = useRef(new Compartment());

  useEffect(() => {
    const submit = (view: EditorView) => {
      // Enter picks a suggestion when the list is open.
      if (completionStatus(view.state) === "active") return false;
      props.current.onSubmit?.();
      return Boolean(props.current.onSubmit);
    };
    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          Prec.highest(keymap.of([{ key: "Mod-Enter", run: submit }, ...(mode === "line" ? [{ key: "Enter", run: submit }] : [])])),
          ...modeExtensions(mode),
          varsSlot.current.of(varsFacet.of(vars)),
          placeholderSlot.current.of(placeholder ? placeholderExt(placeholder) : []),
          EditorView.updateListener.of((u) => {
            if (!u.docChanged || u.transactions.some((tr) => tr.annotation(external))) return;
            props.current.onChange(u.state.doc.toString());
            // Suggestions open right after "{{", before any letter is typed.
            const head = u.state.selection.main.head;
            if (u.transactions.some((tr) => tr.isUserEvent("input.type")) && u.state.sliceDoc(head - 2, head) === "{{") {
              setTimeout(() => startCompletion(u.view));
            }
          }),
          EditorView.domEventHandlers({
            paste: (e) => {
              const text = e.clipboardData?.getData("text/plain") ?? "";
              if (text && props.current.onPaste?.(text)) {
                e.preventDefault();
                return true;
              }
              return false;
            },
          }),
        ],
      }),
    });
    view.current = v;
    if (autoFocus) v.focus();
    return () => {
      v.destroy();
      view.current = null;
    };
    // The editor is built once per mode; value, vars and placeholder are pushed in below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode]);

  useEffect(() => {
    const v = view.current;
    if (!v || v.state.doc.toString() === value) return;
    v.dispatch({
      changes: { from: 0, to: v.state.doc.length, insert: value },
      annotations: [external.of(true), Transaction.addToHistory.of(false)],
      selection: { anchor: Math.min(v.state.selection.main.head, value.length) },
    });
  }, [value]);

  useEffect(() => {
    view.current?.dispatch({ effects: varsSlot.current.reconfigure(varsFacet.of(vars)) });
  }, [vars]);

  useEffect(() => {
    view.current?.dispatch({ effects: placeholderSlot.current.reconfigure(placeholder ? placeholderExt(placeholder) : []) });
  }, [placeholder]);

  return <div ref={host} className={`editor editor-${mode} ${className ?? ""}`} />;
}
