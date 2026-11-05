import { type CompletionContext, type CompletionResult, autocompletion, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { HighlightStyle, bracketMatching, syntaxHighlighting } from "@codemirror/language";
import { type Action, type Diagnostic as LintDiagnostic, lintGutter, lintKeymap, setDiagnostics } from "@codemirror/lint";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension, type StateEffect, type TransactionSpec } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { asn1 } from "./asn1-mode";

const highlight = HighlightStyle.define([
  { tag: t.keyword, class: "tok-keyword" },
  { tag: t.typeName, class: "tok-type" },
  { tag: t.variableName, class: "tok-value" },
  { tag: t.propertyName, class: "tok-field" },
  { tag: t.string, class: "tok-string" },
  { tag: t.number, class: "tok-number" },
  { tag: t.comment, class: "tok-comment" },
  { tag: t.operator, class: "tok-operator" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bs-body-bg)", color: "var(--bs-body-color)" },
  ".cm-scroller": { fontFamily: "var(--bs-font-monospace)", fontSize: "0.85rem", lineHeight: "1.45" },
  ".cm-gutters": {
    backgroundColor: "var(--bs-tertiary-bg)",
    color: "var(--bs-secondary-color)",
    border: "none",
  },
  ".cm-activeLine": { backgroundColor: "var(--glint-active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--glint-active-line)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--glint-selection)",
  },
  ".cm-cursor": { borderLeftColor: "var(--bs-body-color)" },
  ".cm-tooltip": {
    backgroundColor: "var(--bs-body-bg)",
    color: "var(--bs-body-color)",
    border: "1px solid var(--bs-border-color)",
  },
});

// One compartment shared by every editor: it is only a key, each state holds its own setting.
const language = new Compartment();
const languageExtension = (enabled: boolean): Extension => (enabled ? asn1 : []);

/** Transaction that replaces the whole document and moves the cursor to the start. */
export function replaceTextSpec(state: EditorState, text: string): TransactionSpec {
  return { changes: { from: 0, to: state.doc.length, insert: text }, selection: { anchor: 0 } };
}

export function highlightingSpec(enabled: boolean): TransactionSpec {
  return { effects: language.reconfigure(languageExtension(enabled)) };
}

export function markersSpec(state: EditorState, markers: LintDiagnostic[]): TransactionSpec {
  return setDiagnostics(state, markers);
}

export interface EditorHandle {
  view: EditorView;
  text(): string;
  setText(text: string): void;
  setHighlighting(enabled: boolean): void;
  setMarkers(markers: LintDiagnostic[]): void;
  reveal(from: number, to: number): void;
  /** A fresh state with this editor's extensions, for a document that is not on screen yet. */
  newState(doc?: string): EditorState;
  /** Shows another state in the same view; `scroll` is a value from `scrollSnapshot`. */
  swap(state: EditorState, scroll?: StateEffect<unknown> | null): void;
  scrollSnapshot(): StateEffect<unknown>;
}

export interface EditorOptions {
  doc?: string;
  readOnly?: boolean;
  highlighting?: boolean;
  /** Number of the first line, for a view that shows an excerpt of a larger document. */
  firstLine?: number;
  onChange?: () => void;
  /** Completions at a UTF-16 position of the text. */
  complete?: (text: string, position: number, signal: AbortSignal) => Promise<Completions>;
}

/** What `complete` answers: the start of the word, and what may replace it. */
export interface Completions {
  from: number;
  items: { label: string; type: string; detail: string | null; apply?: string }[];
}

function completionSource(complete: NonNullable<EditorOptions["complete"]>) {
  return async (context: CompletionContext): Promise<CompletionResult | null> => {
    const word = context.matchBefore(/[A-Za-z0-9-]*/);
    const afterBrace = context.matchBefore(/[{(,]\s*$|FROM\s+$/);
    if (!context.explicit && (!word || word.from === word.to) && !afterBrace) return null;
    const controller = new AbortController();
    context.addEventListener("abort", () => controller.abort());
    try {
      const result = await complete(context.state.doc.toString(), context.pos, controller.signal);
      if (result.items.length === 0) return null;
      return {
        from: result.from,
        options: result.items.map((item) => ({ label: item.label, type: item.type, detail: item.detail ?? undefined, apply: item.apply })),
        validFor: /^[A-Za-z0-9-]*$/,
      };
    } catch {
      return null;
    }
  };
}

export function createEditor(parent: HTMLElement, options: EditorOptions = {}): EditorHandle {
  const firstLine = options.firstLine ?? 1;
  const extensions: Extension[] = [
    lineNumbers(firstLine === 1 ? {} : { formatNumber: (n) => String(n + firstLine - 1) }),
    highlightActiveLineGutter(),
    history(),
    drawSelection(),
    bracketMatching(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    lintGutter(),
    syntaxHighlighting(highlight),
    language.of(languageExtension(options.highlighting ?? true)),
    keymap.of([...completionKeymap, ...defaultKeymap, ...historyKeymap, ...searchKeymap, ...lintKeymap]),
    theme,
    EditorView.contentAttributes.of({ "aria-label": options.readOnly ? "Module text" : "Document to check" }),
  ];
  if (options.complete) {
    extensions.push(autocompletion({ override: [completionSource(options.complete)] }));
  }
  if (options.readOnly) {
    extensions.push(EditorState.readOnly.of(true), EditorView.editable.of(false));
  }
  if (options.onChange) {
    const onChange = options.onChange;
    extensions.push(
      EditorView.updateListener.of((update) => {
        if (update.docChanged) onChange();
      }),
    );
  }
  const newState = (doc = "") => EditorState.create({ doc, extensions });
  const view = new EditorView({ parent, state: newState(options.doc) });
  return {
    view,
    text: () => view.state.doc.toString(),
    setText(text) {
      view.dispatch(replaceTextSpec(view.state, text));
    },
    setHighlighting(enabled) {
      view.dispatch(highlightingSpec(enabled));
    },
    setMarkers(markers) {
      view.dispatch(markersSpec(view.state, markers));
    },
    reveal(from, to) {
      const length = view.state.doc.length;
      const start = Math.min(Math.max(0, from), length);
      const end = Math.min(Math.max(start, to), length);
      view.dispatch({ selection: { anchor: start, head: end }, effects: EditorView.scrollIntoView(start, { y: "center" }) });
      view.focus();
    },
    newState,
    swap(state, scroll) {
      view.setState(state);
      if (scroll) view.dispatch({ effects: scroll });
    },
    scrollSnapshot: () => view.scrollSnapshot(),
  };
}

export type { Action, LintDiagnostic };
