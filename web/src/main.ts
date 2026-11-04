import { api } from "./api";
import { createChecker } from "./checker";
import { createCorpusBrowser, createModuleViewer, createOidPage } from "./corpus";
import { safeStorage } from "./dom";
import { parseHash } from "./router";
import "./style.css";

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing #${id}`);
  return found;
}

function initTheme(): void {
  const storage = safeStorage();
  const stored = storage?.getItem("glint-theme");
  const preferred = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  const apply = (theme: string) => document.documentElement.setAttribute("data-bs-theme", theme);
  apply(stored === "dark" || stored === "light" ? stored : preferred);
  element("theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.getAttribute("data-bs-theme") === "dark" ? "light" : "dark";
    apply(next);
    try {
      storage?.setItem("glint-theme", next);
    } catch {
      // storage can be blocked; the theme still applies for this visit
    }
  });
}

function main(): void {
  initTheme();
  const views = {
    checker: element("view-checker"),
    corpus: element("view-corpus"),
    module: element("view-module"),
    oid: element("view-oid"),
  };
  const checker = createChecker(views.checker);
  const corpus = createCorpusBrowser(views.corpus);
  const modules = createModuleViewer(views.module);
  const oids = createOidPage(views.oid);

  function route(): void {
    const current = parseHash(window.location.hash);
    for (const [name, section] of Object.entries(views)) section.hidden = name !== current.view;
    const navView = current.view === "module" ? "corpus" : current.view;
    for (const link of document.querySelectorAll<HTMLElement>("#main-nav .nav-link")) {
      link.classList.toggle("active", link.dataset.view === navView);
    }
    switch (current.view) {
      case "checker":
        checker.show(current);
        break;
      case "corpus":
        corpus.show(current.q);
        break;
      case "module":
        modules.show(current.source, current.name, current.from, current.to, current.oid);
        break;
      case "oid":
        oids.show(current.oid);
        break;
    }
  }

  window.addEventListener("hashchange", route);
  route();
  api.corpus().then(
    (info) => {
      element("corpus-version").textContent = `Corpus ${info.version} · ${info.modules} modules`;
    },
    () => {
      element("corpus-version").textContent = "Server not reachable";
    },
  );
}

main();
