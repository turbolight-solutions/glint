import { onData } from "./api";
import { createChecker } from "./checker";
import { createCorpusBrowser, createOidPage } from "./corpus";
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
    oid: element("view-oid"),
  };
  const checker = createChecker(views.checker);
  const corpus = createCorpusBrowser(views.corpus);
  const oids = createOidPage(views.oid);

  function route(): void {
    const current = parseHash(window.location.hash);
    for (const [name, section] of Object.entries(views)) section.hidden = name !== current.view;
    for (const link of document.querySelectorAll<HTMLElement>("#main-nav .nav-link")) {
      link.classList.toggle("active", link.dataset.view === current.view);
    }
    switch (current.view) {
      case "checker":
        checker.show(current);
        break;
      case "corpus":
        corpus.show(current.q, current.sort ?? null);
        break;
      case "oid":
        oids.show(current.oid);
        break;
    }
  }

  window.addEventListener("hashchange", route);
  route();
  const version = element("corpus-version");
  onData((state) => {
    switch (state.corpus) {
      case "loading":
        version.textContent = "Loading the corpus…";
        break;
      case "failed":
        version.textContent = "Corpus data unavailable; pasted text is checked without it";
        version.title = state.error;
        break;
      case "ready": {
        const { info } = state;
        const texts = state.documents === "failed" ? " · document texts unavailable" : "";
        version.textContent = `Glint ${info.glint_version} · data ${info.build} · ${info.modules} modules${texts}`;
        version.title = "";
        break;
      }
    }
  });
}

main();
