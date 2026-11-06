// The Glint worker: loads the wasm code and the data, then answers the page's calls. Checks run here so the page
// stays responsive while a large document is checked.
import init, { Glint } from "./wasm/glint_wasm.js";
import { SCHEMA } from "./wasm/schema";

interface Scope {
  postMessage(message: unknown): void;
  addEventListener(type: "message", listener: (event: MessageEvent) => void): void;
}
const scope = self as unknown as Scope;

type Message = { type: "start"; base: string } | { type: "call"; id: number; method: string; args: unknown[] };

let glint: Glint | null = null;
let ready: Promise<void> | null = null;
let documents: Promise<void> = Promise.resolve();
let documentsFailed = false;

async function fetchBytes(url: URL): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url.pathname}: ${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

/** Fetches the corpus while the wasm compiles, and the document texts after both. Data of another schema or build
 *  than the code and the pointer expect is refused, loudly; pasted text can still be checked. */
function start(base: string): Promise<void> {
  const dataDir = new URL(`data/v${SCHEMA}/`, base);
  const corpus = (async () => {
    const response = await fetch(new URL("latest.json", dataDir), { cache: "no-cache" });
    if (!response.ok) throw new Error(`latest.json: ${response.status} ${response.statusText}`);
    const pointer = (await response.json()) as { schema?: unknown; build?: unknown };
    if (pointer.schema !== SCHEMA || typeof pointer.build !== "string") {
      throw new Error(`latest.json names schema ${String(pointer.schema)}, but this page reads schema ${SCHEMA}`);
    }
    return { build: pointer.build, bytes: await fetchBytes(new URL(`${pointer.build}/corpus.json`, dataDir)) };
  })();
  const code = init().then(() => {
    glint = new Glint();
    if (Glint.schema() !== SCHEMA) console.error(`glint: the wasm reads schema ${Glint.schema()}, the page ${SCHEMA}`);
  });
  return code.then(async () => {
    try {
      const { build, bytes } = await corpus;
      const info = JSON.parse((glint as Glint).loadCorpus(bytes)) as { build: string };
      if (info.build !== build) {
        glint = new Glint();
        throw new Error(`corpus.json is build ${info.build}, but latest.json names ${build}`);
      }
      scope.postMessage({ event: "corpus", ok: true, info });
      documents = fetchBytes(new URL(`${build}/documents.bin`, dataDir))
        .then((texts) => {
          (glint as Glint).loadDocuments(texts);
          scope.postMessage({ event: "documents", ok: true });
        })
        .catch((error: unknown) => {
          documentsFailed = true;
          console.error("glint: refused the document texts:", error);
          scope.postMessage({ event: "documents", ok: false, error: String(error) });
        });
    } catch (error) {
      console.error("glint: refused the corpus data:", error);
      scope.postMessage({ event: "corpus", ok: false, error: error instanceof Error ? error.message : String(error) });
    }
  });
}

/** The `{ code, message }` that glint-wasm puts in the message of its errors. */
function errorOf(error: unknown): { code: string; message: string } {
  const text = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(text) as { code?: unknown; message?: unknown };
    if (typeof parsed.code === "string" && typeof parsed.message === "string") return { code: parsed.code, message: parsed.message };
  } catch {
    // not one of ours
  }
  return { code: "error", message: text };
}

async function answer(method: string, args: unknown[]): Promise<string> {
  await ready;
  const g = glint;
  if (!g) throw new Error(JSON.stringify({ code: "worker", message: "Glint did not start" }));
  switch (method) {
    case "check":
      return g.check(JSON.stringify(args[0]));
    case "checkDocument":
      await documents;
      if (documentsFailed) {
        throw new Error(JSON.stringify({ code: "documents_unavailable", message: "The document texts could not be loaded; paste or open the document instead." }));
      }
      return g.checkDocument(JSON.stringify(args[0]));
    case "corpus":
      return g.corpus();
    case "modules":
      return g.modules(String(args[0]), Number(args[1]), Number(args[2]));
    case "module":
      return g.module(String(args[0]), String(args[1]), args[2] == null ? undefined : String(args[2]));
    case "oid":
      return g.oid(String(args[0]), Array.isArray(args[1]) ? args[1].map(String) : []);
    case "complete":
      return g.complete(String(args[0]), args[1] == null ? undefined : String(args[1]), args[2] == null ? undefined : String(args[2]), Number(args[3]));
    default:
      throw new Error(JSON.stringify({ code: "bad_request", message: `no method ${method}` }));
  }
}

scope.addEventListener("message", (event: MessageEvent<Message>) => {
  const message = event.data;
  if (message.type === "start") {
    ready ??= start(message.base);
    return;
  }
  answer(message.method, message.args).then(
    (value) => scope.postMessage({ id: message.id, ok: true, value }),
    (error: unknown) => scope.postMessage({ id: message.id, ok: false, error: errorOf(error) }),
  );
});
