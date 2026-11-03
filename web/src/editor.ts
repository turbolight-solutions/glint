import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { HighlightStyle, bracketMatching, syntaxHighlighting } from "@codemirror/language";
import { type Action, type Diagnostic as LintDiagnostic, lintGutter, lintKeymap, setDiagnostics } from "@codemirror/lint";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
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

export interface EditorHandle {
  view: EditorView;
  text(): string;
  setText(text: string): void;
  setHighlighting(enabled: boolean): void;
  setMarkers(markers: LintDiagnostic[]): void;
  reveal(from: number, to: number): void;
}

export interface EditorOptions {
  doc?: string;
  readOnly?: boolean;
  highlighting?: boolean;
  onChange?: () => void;
}

export function createEditor(parent: HTMLElement, options: EditorOptions = {}): EditorHandle {
  const language = new Compartment();
  const languageExtension = (enabled: boolean): Extension => (enabled ? asn1 : []);
  const extensions: Extension[] = [
    lineNumbers(),
    highlightActiveLineGutter(),
    history(),
    drawSelection(),
    bracketMatching(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    lintGutter(),
    syntaxHighlighting(highlight),
    language.of(languageExtension(options.highlighting ?? true)),
    keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, ...lintKeymap]),
    theme,
    EditorView.contentAttributes.of({ "aria-label": options.readOnly ? "Module text" : "Document to check" }),
  ];
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
  const view = new EditorView({
    parent,
    state: EditorState.create({ doc: options.doc ?? "", extensions }),
  });
  return {
    view,
    text: () => view.state.doc.toString(),
    setText(text) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: 0 } });
    },
    setHighlighting(enabled) {
      view.dispatch({ effects: language.reconfigure(languageExtension(enabled)) });
    },
    setMarkers(markers) {
      view.dispatch(setDiagnostics(view.state, markers));
    },
    reveal(from, to) {
      const length = view.state.doc.length;
      const start = Math.min(Math.max(0, from), length);
      const end = Math.min(Math.max(start, to), length);
      view.dispatch({ selection: { anchor: start, head: end }, effects: EditorView.scrollIntoView(start, { y: "center" }) });
      view.focus();
    },
  };
}

export type { Action, LintDiagnostic };
