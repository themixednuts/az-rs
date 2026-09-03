import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { parse } from "node-html-parser";

const DEFAULT_ROOT =
  "https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_DIR = path.resolve(__dirname, "..");

type Page = {
  url: string;
  title: string;
  fileName: string;
  categoryPath: string[];
};

function parseArgs(argv: string[]): { rootUrl: string; outDir: string } {
  const options = {
    rootUrl: DEFAULT_ROOT,
    outDir: path.join(PROJECT_DIR, "out", "cryengine-3-html")
  };

  for (let i = 0; i < argv.length; i += 1) {
    switch (argv[i]) {
      case "--root":
        options.rootUrl = argv[++i];
        break;
      case "--out":
        options.outDir = path.resolve(argv[++i]);
        break;
      default:
        throw new Error(`unknown argument: ${argv[i]}`);
    }
  }

  return options;
}

function sha256(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

function cleanText(value: unknown): string {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function slugify(value: string): string {
  const slug = cleanText(value)
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);

  return slug || "page";
}

function pageIdFromUrl(url: string): string {
  const match = new URL(url).pathname.match(/\/pages\/(\d+)$/);
  return match?.[1] || sha256(url).slice(0, 10);
}

function pageFileName(index: number, title: string, url: string): string {
  const ordinal = String(index + 1).padStart(4, "0");
  return `${ordinal}-${slugify(title)}-${pageIdFromUrl(url)}.html`;
}

function pageUrlAllowed(url: string, rootUrl: string): boolean {
  const parsed = new URL(url);
  const root = new URL(rootUrl);
  const escapedRootPath = root.pathname.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pagePattern = new RegExp(`^${escapedRootPath}/pages/\\d+$`);
  return parsed.origin === root.origin && pagePattern.test(parsed.pathname);
}

async function fetchText(url: string): Promise<string> {
  const response = await fetch(url, {
    headers: { "user-agent": "az-rs cryengine-docs offline html index builder" }
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`);
  }
  return response.text();
}

async function discoverPages(rootUrl: string): Promise<Page[]> {
  const html = await fetchText(rootUrl);
  const root = parse(html);
  const navRoot = root.querySelector("nav.nav-tree .nav-tree__item--list");
  const pages: Page[] = [];
  const seen = new Set<string>();

  if (navRoot) {
    collectPagesFromNavList(navRoot, [], rootUrl, pages, seen);
    return pages;
  }

  for (const anchor of root.querySelectorAll("a[href]")) {
    const href = anchor.getAttribute("href");
    if (!href) {
      continue;
    }

    const url = new URL(href, rootUrl);
    url.hash = "";

    if (!pageUrlAllowed(url.toString(), rootUrl) || seen.has(url.toString())) {
      continue;
    }

    seen.add(url.toString());
    const title = cleanText(anchor.getAttribute("title") || anchor.textContent) || pageIdFromUrl(url.toString());
    pages.push({
      url: url.toString(),
      title,
      fileName: pageFileName(pages.length, title, url.toString()),
      categoryPath: []
    });
  }

  return pages;
}

function collectPagesFromNavList(
  list: ReturnType<typeof parse>,
  categoryPath: string[],
  rootUrl: string,
  pages: Page[],
  seen: Set<string>
): void {
  for (const child of elementChildren(list)) {
    if (child.rawTagName === "a") {
      addPageFromAnchor(child, categoryPath, rootUrl, pages, seen);
      continue;
    }

    if (child.rawTagName !== "div" || !hasClass(child, "nav-tree__item")) {
      continue;
    }

    const title = elementChildren(child).find((element) =>
      hasClass(element, "nav-tree__item--title")
    );
    const anchor = title?.querySelector("a[href]");
    const page = anchor ? addPageFromAnchor(anchor, categoryPath, rootUrl, pages, seen) : null;
    const nestedList = elementChildren(child).find((element) =>
      hasClass(element, "nav-tree__item--list")
    );

    if (nestedList) {
      const nestedCategory = page?.title || cleanText(anchor?.textContent) || "Untitled";
      collectPagesFromNavList(nestedList, [...categoryPath, nestedCategory], rootUrl, pages, seen);
    }
  }
}

function addPageFromAnchor(
  anchor: ReturnType<typeof parse>,
  categoryPath: string[],
  rootUrl: string,
  pages: Page[],
  seen: Set<string>
): Page | null {
  const href = anchor.getAttribute("href");
  if (!href) {
    return null;
  }

  const url = new URL(href, rootUrl);
  url.hash = "";

  if (!pageUrlAllowed(url.toString(), rootUrl) || seen.has(url.toString())) {
    return null;
  }

  seen.add(url.toString());
  const title = cleanText(anchor.getAttribute("title") || anchor.querySelector("span")?.textContent || anchor.textContent)
    || pageIdFromUrl(url.toString());
  const page = {
    url: url.toString(),
    title,
    fileName: pageFileName(pages.length, title, url.toString()),
    categoryPath
  };
  pages.push(page);
  return page;
}

function elementChildren(element: ReturnType<typeof parse>): ReturnType<typeof parse>[] {
  return element.childNodes.filter((child): child is ReturnType<typeof parse> =>
    typeof (child as ReturnType<typeof parse>).rawTagName === "string" &&
    typeof (child as ReturnType<typeof parse>).getAttribute === "function"
  );
}

function hasClass(element: ReturnType<typeof parse>, className: string): boolean {
  const classList = (element as unknown as { classList?: { contains(value: string): boolean } }).classList;
  if (classList?.contains(className)) {
    return true;
  }

  return (element.getAttribute("class") || "").split(/\s+/).includes(className);
}

async function discoverStylesheets(rootUrl: string): Promise<string[]> {
  const html = await fetchText(rootUrl);
  const root = parse(html);
  const stylesheets: string[] = [];
  const seen = new Set<string>();

  for (const link of root.querySelectorAll("link[href]")) {
    const rel = (link.getAttribute("rel") || "").toLowerCase();
    const as = (link.getAttribute("as") || "").toLowerCase();
    const href = link.getAttribute("href");

    if (!href || (rel !== "stylesheet" && as !== "style") || !href.includes(".css")) {
      continue;
    }

    const url = new URL(href, rootUrl);
    const localHref = url.hostname === "www.cryengine.com"
      ? url.pathname.replace(/^\//, "")
      : `external/${url.hostname}${url.pathname}`;

    if (!seen.has(localHref)) {
      seen.add(localHref);
      stylesheets.push(localHref);
    }
  }

  return stylesheets;
}

function htmlEscape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function globalToc(pages: Page[]): string {
  return `<aside class="offline-toc" aria-label="Table of contents">
    <div class="offline-toc-inner">
      <a class="offline-toc-home" href="index.html">CRYENGINE 3 Manual</a>
      <input class="offline-toc-filter" type="search" placeholder="Filter ${pages.length} pages" aria-label="Filter table of contents">
      <ol class="offline-toc-list">
${renderTocGroup(buildTocTree(pages), 0)}
      </ol>
    </div>
  </aside>`;
}

type TocGroup = {
  title: string;
  key: string;
  indexPage?: Page;
  pages: Page[];
  children: TocGroup[];
  childByTitle: Map<string, TocGroup>;
};

function buildTocTree(pages: Page[]): TocGroup {
  const rootGroup = createTocGroup("root", "root");

  for (const page of pages) {
    let group = rootGroup;
    for (const category of page.categoryPath) {
      const key = `${group.key}/${category}`;
      let child = group.childByTitle.get(category);
      if (!child) {
        child = createTocGroup(category, key);
        group.childByTitle.set(category, child);
        group.children.push(child);
      }
      group = child;
    }
    group.pages.push(page);
  }

  attachCategoryIndexPages(rootGroup);
  return rootGroup;
}

function createTocGroup(title: string, key: string): TocGroup {
  return {
    title,
    key,
    pages: [],
    children: [],
    childByTitle: new Map()
  };
}

function attachCategoryIndexPages(group: TocGroup): void {
  for (const child of group.children) {
    const index = group.pages.findIndex((page) => page.title === child.title);
    if (index >= 0) {
      child.indexPage = group.pages[index];
      group.pages.splice(index, 1);
    }
    attachCategoryIndexPages(child);
  }
}

function renderTocGroup(group: TocGroup, depth: number): string {
  const entries: string[] = [];

  for (const page of group.pages) {
    entries.push(renderTocPage(page));
  }

  for (const child of group.children) {
    const visibleTitle = htmlEscape(child.title);
    const categoryTitle = child.indexPage
      ? `<a class="offline-toc-category-link" href="${tocHref(child.indexPage)}">${visibleTitle}</a>`
      : `<span class="offline-toc-category-title">${visibleTitle}</span>`;
    entries.push(`<li class="offline-toc-category" data-title="${visibleTitle.toLowerCase()}">
      <details open data-category-key="${htmlEscape(child.key)}">
        <summary><span class="offline-toc-category-title">${categoryTitle}</span></summary>
        <ol class="offline-toc-children" data-depth="${depth + 1}">
${renderTocGroup(child, depth + 1)}
        </ol>
      </details>
    </li>`);
  }

  return entries.join("\n");
}

function renderTocPage(page: Page): string {
  const title = htmlEscape(page.title);
  const ordinal = page.fileName.slice(0, 4);
  return `<li class="offline-toc-item" data-title="${title.toLowerCase()}">
    <a href="${tocHref(page)}">
      <span class="offline-toc-number">${ordinal}</span>
      <span>${title}</span>
    </a>
  </li>`;
}

function tocHref(page: Page): string {
  return `pages/${page.fileName}`;
}

function tocScript(): string {
  return `<script>
(() => {
  const storagePrefix = 'cryengine3-docs:toc:';
  const input = document.querySelector('.offline-toc-filter');
  const list = document.querySelector('.offline-toc-list');
  const items = [...document.querySelectorAll('.offline-toc-item')];
  const categories = [...document.querySelectorAll('.offline-toc-category')];
  const details = [...document.querySelectorAll('.offline-toc-category details[data-category-key]')];

  for (const detail of details) {
    const key = detail.dataset.categoryKey;
    const saved = localStorage.getItem(storagePrefix + 'category:' + key);
    if (saved === 'closed') detail.open = false;
    if (saved === 'open') detail.open = true;
    detail.addEventListener('toggle', () => {
      localStorage.setItem(storagePrefix + 'category:' + key, detail.open ? 'open' : 'closed');
    });
  }

  for (const link of document.querySelectorAll('.offline-toc-category-link')) {
    link.addEventListener('click', (event) => {
      event.stopPropagation();
    });
  }

  if (list) {
    const selected = localStorage.getItem(storagePrefix + 'selected');
    const savedScroll = localStorage.getItem(storagePrefix + 'scrollTop:' + selected);
    if (savedScroll) list.scrollTop = Number(savedScroll) || 0;
    list.addEventListener('scroll', () => {
      localStorage.setItem(storagePrefix + 'scrollTop:index', String(list.scrollTop));
    }, { passive: true });
  }

  for (const link of document.querySelectorAll('.offline-toc a[href]')) {
    link.addEventListener('click', () => {
      localStorage.setItem(storagePrefix + 'selected', link.getAttribute('href') || '');
      if (list) {
        localStorage.setItem(storagePrefix + 'scrollTop:' + (link.getAttribute('href') || ''), String(list.scrollTop));
      }
    });
  }

  if (!input) return;
  const applyFilter = () => {
    const query = input.value.trim().toLowerCase();
    for (const item of items) {
      item.hidden = query.length > 0 && !item.dataset.title.includes(query);
    }
    for (const category of categories) {
      const matchesCategory = category.dataset.title.includes(query);
      const hasVisibleChild = !!category.querySelector('.offline-toc-item:not([hidden]), .offline-toc-category:not([hidden])');
      category.hidden = query.length > 0 && !matchesCategory && !hasVisibleChild;
      const detail = category.querySelector('details');
      if (detail && query.length > 0 && !category.hidden) {
        detail.open = true;
      }
    }
  };
  input.addEventListener('input', applyFilter);
})();
</script>`;
}

function indexDocument(pages: Page[], stylesheetHrefs: string[]): string {
  const scrapedAt = new Date().toISOString();
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>CRYENGINE 3 Manual</title>
  ${stylesheetHrefs.map((href) => `<link rel="stylesheet" href="${htmlEscape(href)}">`).join("\n  ")}
  <link rel="stylesheet" href="style.css">
</head>
<body>
  <header class="offline-topbar">
    <a href="index.html">CRYENGINE 3 Manual</a>
    <nav>
      <span>${pages.length} pages</span>
    </nav>
  </header>
  <div class="offline-layout">
    ${globalToc(pages)}
    <main class="offline-doc offline-index">
      <h1>CRYENGINE 3 Manual</h1>
      <p class="offline-muted">Content-only offline mirror. Scraped ${htmlEscape(scrapedAt)}.</p>
      <p>Use the table of contents to jump to any mirrored route. Pages still being scraped may appear in the TOC before their local file exists.</p>
    </main>
  </div>
  ${tocScript()}
</body>
</html>
`;
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));
  const [pages, stylesheets] = await Promise.all([
    discoverPages(options.rootUrl),
    discoverStylesheets(options.rootUrl)
  ]);

  await mkdir(options.outDir, { recursive: true });
  await writeFile(path.join(options.outDir, "index.html"), indexDocument(pages, stylesheets), "utf8");
  console.log(`Wrote ${path.join(options.outDir, "index.html")} with ${pages.length} routes`);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? (error.stack || error.message) : String(error));
  process.exitCode = 1;
});
