import { StreamLanguage, type StringStream } from "@codemirror/language";

const KEYWORDS = new Set(
  (
    "ABSENT ABSTRACT-SYNTAX ALL APPLICATION AUTOMATIC BEGIN BIT BMPString BOOLEAN BY CHARACTER CHOICE CLASS " +
    "COMPONENT COMPONENTS CONSTRAINED CONTAINING DEFAULT DEFINITIONS EMBEDDED ENCODED ENUMERATED EXCEPT EXPLICIT " +
    "EXPORTS EXTENSIBILITY EXTERNAL FALSE FROM GeneralizedTime GeneralString GraphicString IA5String IDENTIFIER " +
    "IMPLICIT IMPLIED IMPORTS INCLUDES INSTANCE INTEGER INTERSECTION ISO646String MAX MIN MINUS-INFINITY NULL " +
    "NumericString OBJECT ObjectDescriptor OCTET OF OPTIONAL PATTERN PDV PLUS-INFINITY PRESENT PrintableString " +
    "PRIVATE REAL RELATIVE-OID SEQUENCE SET SIZE STRING SYNTAX T61String TAGS TeletexString TRUE TYPE-IDENTIFIER " +
    "UNION UNIQUE UNIVERSAL UniversalString UTCTime UTF8String VideotexString VisibleString WITH"
  ).split(" "),
);

interface State {
  blockComment: number;
}

function token(stream: StringStream, state: State): string | null {
  if (state.blockComment > 0) {
    while (!stream.eol()) {
      if (stream.match("*/")) {
        state.blockComment -= 1;
        if (state.blockComment === 0) return "comment";
      } else if (stream.match("/*")) {
        state.blockComment += 1;
      } else {
        stream.next();
      }
    }
    return "comment";
  }
  if (stream.eatSpace()) return null;
  if (stream.match("/*")) {
    state.blockComment = 1;
    return token(stream, state);
  }
  if (stream.match("--")) {
    while (!stream.eol()) {
      if (stream.match("--")) break;
      stream.next();
    }
    return "comment";
  }
  if (stream.match(/^"(?:[^"]|"")*"?/)) return "string";
  if (stream.match(/^'[0-9A-Fa-f\s]*'[BH]/)) return "number";
  if (stream.match("::=")) return "operator";
  if (stream.match(/^&[A-Za-z][A-Za-z0-9-]*/)) return "propertyName";
  if (stream.match(/^[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?/)) return "number";
  if (stream.match(/^[A-Za-z][A-Za-z0-9]*(-[A-Za-z0-9]+)*/)) {
    const word = stream.current();
    if (KEYWORDS.has(word) || /^[A-Z][A-Z0-9]*(-[A-Z0-9]+)*$/.test(word)) return "keyword";
    return /^[A-Z]/.test(word) ? "typeName" : "variableName";
  }
  stream.next();
  return "punctuation";
}

export const asn1 = StreamLanguage.define<State>({
  name: "asn1",
  startState: () => ({ blockComment: 0 }),
  copyState: (state) => ({ ...state }),
  token,
  languageData: { commentTokens: { line: "--", block: { open: "/*", close: "*/" } } },
});
