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

export default defineConfig({
  plugins: [localData()],
  worker: { format: "es" },
  build: { outDir: "dist", sourcemap: true },
  test: { environment: "node" },
});
