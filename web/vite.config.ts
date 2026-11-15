import { createReadStream, existsSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

/** In development, serves the data that corpus-build wrote to corpus/out at `/data`, as the site does. */
function localData(): Plugin {
  const root = fileURLToPath(new URL("../corpus/out/data/", import.meta.url));
  return {
    name: "glint-local-data",
    configureServer(server) {
      server.middlewares.use("/data", (request, response, next) => {
        const relative = decodeURIComponent((request.url ?? "").split("?")[0] ?? "").replace(/^\/+/, "");
        const path = fileURLToPath(new URL(relative, `file://${root}`));
        if (!path.startsWith(root) || !existsSync(path) || statSync(path).isDirectory()) {
          next();
          return;
        }
        response.setHeader("Content-Type", path.endsWith(".json") ? "application/json" : "application/octet-stream");
        createReadStream(path).pipe(response);
      });
    },
  };
}

/**
 * GitHub Pages serves no response headers of ours, so the built page carries its Content-Security-Policy in a meta
 * tag. Not in development, where Vite's client needs its websocket. `frame-ancestors` cannot be set this way.
 */
const CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval' https://cdn.jsdelivr.net",
  "style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net",
  "font-src 'self' https://cdn.jsdelivr.net",
  "img-src 'self' data:",
  "connect-src 'self'",
  "worker-src 'self'",
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'self'",
].join("; ");

function contentSecurityPolicy(): Plugin {
  return {
    name: "glint-csp",
    apply: "build",
    // Right after the charset, so it comes before every resource the page loads.
    transformIndexHtml: (html) => {
      const charset = '<meta charset="utf-8" />';
      if (!html.includes(charset)) throw new Error(`index.html has no ${charset}`);
      return html.replace(charset, `${charset}\n    <meta http-equiv="Content-Security-Policy" content="${CSP}" />`);
    },
  };
}

export default defineConfig({
  // Relative, so the site works under any path: at the custom domain's root, or at /glint/ on github.io.
  base: "./",
  plugins: [localData(), contentSecurityPolicy()],
  worker: { format: "es" },
  build: { outDir: "dist", sourcemap: true },
  test: { environment: "node" },
});
