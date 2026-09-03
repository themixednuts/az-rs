import { createHash } from "node:crypto";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "node-html-parser";
import type HTMLElement from "node-html-parser/dist/nodes/html.js";

const DEFAULT_ROOT =
  "https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_DIR = path.resolve(__dirname, "..");

type ScrapeOptions = {
  rootUrl: string;
  outDir: string;
  limit: number;
  delayMs: number;
  retries: number;
  concurrency: number;
  resume: boolean;
  downloadAssets: boolean;
};

type Page = {
  url: string;
  title: string;
  fileName?: string;
  sourceSha256?: string;
  categoryPath: string[];
};

type AssetRecord = {
  fileName: string;
  localPath: string;
  sha256: string;
  bytes: number;
};

type AssetFailure = {
  url: string;
  error: string;
};

type ManifestPage = {
  url: string;
  title: string;
  fileName: string;
  categoryPath: string[];
  status: "ok" | "skipped" | "refreshed" | "error";
  sourceSha256?: string;
  htmlSha256?: string;
  error?: string;
};

type Manifest = {
  rootUrl: string;
  scrapedAt: string;
  pages: ManifestPage[];
  media: Array<{ url: string } & AssetRecord>;
  mediaFailures: AssetFailure[];
};

function parseArgs(argv: string[]): ScrapeOptions {
  const options: ScrapeOptions = {
    rootUrl: DEFAULT_ROOT,
    outDir: path.join(PROJECT_DIR, "out", "cryengine-3-html"),
    limit: Number.POSITIVE_INFINITY,
    delayMs: 250,
    retries: 3,
    concurrency: 4,
    resume: true,
    downloadAssets: true
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case "--root":
        options.rootUrl = argv[++i];
        break;
      case "--out":
        options.outDir = path.resolve(argv[++i]);
        break;
      case "--limit":
        options.limit = Number.parseInt(argv[++i], 10);
        break;
      case "--delay-ms":
        options.delayMs = Number.parseInt(argv[++i], 10);
        break;
      case "--retries":
        options.retries = Number.parseInt(argv[++i], 10);
        break;
      case "--concurrency":
        options.concurrency = Number.parseInt(argv[++i], 10);
        break;
      case "--no-resume":
        options.resume = false;
        break;
      case "--no-assets":
        options.downloadAssets = false;
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }

  if (!Number.isFinite(options.limit) || options.limit < 1) {
    options.limit = Number.POSITIVE_INFINITY;
  }
  if (!Number.isFinite(options.concurrency) || options.concurrency < 1) {
    options.concurrency = 1;
  }

  return options;
}

function printHelp(): void {
  console.log(`Usage: npm run scrape:html -- [options]

Options:
  --root <url>       Root CRYENGINE docs category URL.
  --out <dir>        Output directory. Defaults to tools/cryengine-docs/out/cryengine-3-html
  --limit <n>        Scrape only the first n pages.
  --delay-ms <n>     Delay between page fetches. Defaults to 250.
  --retries <n>      Fetch retry count. Defaults to 3.
  --concurrency <n>  Concurrent page fetches. Defaults to 4.
  --no-resume        Rebuild pages even if HTML files already exist.
  --no-assets        Do not download article media.
`);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function sha256(value: string | Buffer): string {
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

async function fetchWithRetries(url: string, retries: number): Promise<Response> {
  let lastError: unknown;

  for (let attempt = 0; attempt <= retries; attempt += 1) {
    try {
      const response = await fetch(url, {
        headers: {
          "user-agent": "az-rs cryengine-docs offline html scraper"
        }
      });

      if (response.ok) {
        return response;
      }

      lastError = new Error(`${response.status} ${response.statusText} for ${url}`);
      if (![408, 429, 500, 502, 503, 504].includes(response.status)) {
        throw lastError;
      }
    } catch (error) {
      lastError = error;
    }

    if (attempt < retries) {
      await sleep(Math.min(15_000, 1_000 * 2 ** attempt));
    }
  }

  throw lastError;
}

async function fetchText(url: string, retries: number): Promise<string> {
  const response = await fetchWithRetries(url, retries);
  return response.text();
}

function removeNoise(root: HTMLElement): void {
  for (const selector of [
    "script",
    "style",
    "svg",
    "form",
    "input",
    ".layout__details--aside",
    ".visibility-hidden"
  ]) {
    for (const element of root.querySelectorAll(selector)) {
      element.remove();
    }
  }
}

function unwrapCustomConfluenceTags(html: string): string {
  return html
    .replace(/<\/?(structured-macro|parameter|rich-text-body)\b[^>]*>/gi, "")
    .replace(/\s+class="(?:layout__details--aside|visibility-hidden)"[^>]*>.*?<\/div>/gis, ">");
}

function absolutizeElementUrls(root: HTMLElement, pageUrl: string): void {
  for (const element of root.querySelectorAll("a[href]")) {
    const href = element.getAttribute("href");
    if (!href || href.startsWith("#") || href.startsWith("mailto:")) {
      continue;
    }
    element.setAttribute("href", new URL(href, pageUrl).toString());
  }

  for (const element of root.querySelectorAll("img[src]")) {
    const src = element.getAttribute("src");
    if (!src || src.startsWith("data:")) {
      continue;
    }
    element.setAttribute("src", new URL(src, pageUrl).toString());
  }
}

function extractArticle(html: string, pageUrl: string): { title: string; articleHtml: string } {
  const root = parse(html, {
    blockTextElements: {
      script: true,
      style: true,
      pre: true
    }
  });

  const title =
    cleanText(root.querySelector(".layout__main--content h1")?.textContent) ||
    cleanText(root.querySelector("h1")?.textContent) ||
    cleanText(root.querySelector("title")?.textContent).replace(/^CRYENGINE \| Documentation - /, "") ||
    pageIdFromUrl(pageUrl);

  const content = root.querySelector(".typography--user-content");
  if (!content) {
    throw new Error(`could not find article content in ${pageUrl}`);
  }

  removeNoise(content);
  absolutizeElementUrls(content, pageUrl);

  return {
    title,
    articleHtml: unwrapCustomConfluenceTags(content.innerHTML)
  };
}

function htmlEscape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

async function discoverPages(rootUrl: string, retries: number): Promise<Page[]> {
  const html = await fetchText(rootUrl, retries);
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
    pages.push({
      url: url.toString(),
      title: cleanText(anchor.getAttribute("title") || anchor.textContent) || pageIdFromUrl(url.toString()),
      categoryPath: []
    });
  }

  return pages;
}

function collectPagesFromNavList(
  list: HTMLElement,
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

    const title = directElementChildren(child).find((element) =>
      hasClass(element, "nav-tree__item--title")
    );
    const anchor = title?.querySelector("a[href]");
    const page = anchor ? addPageFromAnchor(anchor, categoryPath, rootUrl, pages, seen) : null;
    const nestedList = directElementChildren(child).find((element) =>
      hasClass(element, "nav-tree__item--list")
    );

    if (nestedList) {
      const nestedCategory = page?.title || cleanText(anchor?.textContent) || "Untitled";
      collectPagesFromNavList(nestedList, [...categoryPath, nestedCategory], rootUrl, pages, seen);
    }
  }
}

function addPageFromAnchor(
  anchor: HTMLElement,
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
  const page = { url: url.toString(), title, categoryPath };
  pages.push(page);
  return page;
}

function elementChildren(element: HTMLElement): HTMLElement[] {
  return element.childNodes.filter((child): child is HTMLElement =>
    typeof (child as HTMLElement).rawTagName === "string" &&
    typeof (child as HTMLElement).getAttribute === "function"
  );
}

function directElementChildren(element: HTMLElement): HTMLElement[] {
  return elementChildren(element);
}

function hasClass(element: HTMLElement, className: string): boolean {
  const classList = (element as unknown as { classList?: { contains(value: string): boolean } }).classList;
  if (classList?.contains(className)) {
    return true;
  }

  return (element.getAttribute("class") || "").split(/\s+/).includes(className);
}

function contentTypeExtension(contentType: string | null): string {
  const normalized = String(contentType || "").split(";")[0].trim().toLowerCase();
  const byType: Record<string, string> = {
    "application/pdf": ".pdf",
    "application/zip": ".zip",
    "audio/mpeg": ".mp3",
    "audio/ogg": ".ogg",
    "audio/wav": ".wav",
    "audio/webm": ".webm",
    "image/jpeg": ".jpg",
    "image/png": ".png",
    "image/gif": ".gif",
    "image/svg+xml": ".svg",
    "image/webp": ".webp",
    "video/mp4": ".mp4",
    "video/ogg": ".ogv",
    "video/quicktime": ".mov",
    "video/webm": ".webm"
  };
  return byType[normalized] || "";
}

function urlExtension(url: string): string {
  const ext = path.extname(new URL(url).pathname).toLowerCase();
  if (/^\.[a-z0-9]{2,6}$/.test(ext)) {
    return ext;
  }
  return "";
}

const MEDIA_EXTENSIONS = new Set([
  ".apng",
  ".avif",
  ".bmp",
  ".gif",
  ".ico",
  ".jpeg",
  ".jpg",
  ".m4a",
  ".mov",
  ".mp3",
  ".mp4",
  ".oga",
  ".ogg",
  ".ogv",
  ".pdf",
  ".png",
  ".svg",
  ".tga",
  ".tif",
  ".tiff",
  ".wav",
  ".webm",
  ".webp",
  ".zip"
]);

class AssetStore {
  outDir: string;
  pageDir: string;
  assets = new Map<string, AssetRecord>();
  inflight = new Map<string, Promise<string>>();
  usedLocalPaths = new Set<string>();
  failures: AssetFailure[] = [];
  retries: number;

  constructor(outDir: string, pageDir: string, retries: number) {
    this.outDir = outDir;
    this.pageDir = pageDir;
    this.retries = retries;
  }

  seed(records: Array<{ url: string } & AssetRecord>): void {
    for (const record of records) {
      this.assets.set(record.url, {
        fileName: record.fileName,
        localPath: record.localPath,
        sha256: record.sha256,
        bytes: record.bytes
      });
      this.usedLocalPaths.add(path.join(this.outDir, record.localPath));
    }
  }

  async localPath(url: string): Promise<string> {
    return this.localPathFrom(url, this.pageDir);
  }

  async localPathFrom(url: string, fromDir: string): Promise<string> {
    if (this.assets.has(url)) {
      return this.relativeFromDir(this.assets.get(url)!.localPath, fromDir);
    }

    if (this.inflight.has(url)) {
      await this.inflight.get(url);
      return this.assets.has(url) ? this.relativeFromDir(this.assets.get(url)!.localPath, fromDir) : url;
    }

    const download = this.download(url, fromDir);
    this.inflight.set(url, download);
    try {
      return await download;
    } finally {
      this.inflight.delete(url);
    }
  }

  private async download(url: string, fromDir: string): Promise<string> {
    try {
      const response = await fetchWithRetries(url, this.retries);
      const buffer = Buffer.from(await response.arrayBuffer());
      const hash = sha256(buffer).slice(0, 16);
      const localPath = this.localFilePath(url, hash, response.headers.get("content-type"));
      await mkdir(path.dirname(localPath), { recursive: true });
      await writeFile(localPath, buffer);

      const record = {
        fileName: path.basename(localPath),
        localPath: path.relative(this.outDir, localPath).replace(/\\/g, "/"),
        sha256: sha256(buffer),
        bytes: buffer.length
      };

      this.assets.set(url, record);
      return this.relativeFromDir(record.localPath, fromDir);
    } catch (error) {
      this.failures.push({ url, error: error instanceof Error ? error.message : String(error) });
      return url;
    }
  }

  absolutePath(url: string): string | null {
    const record = this.assets.get(url);
    return record ? path.join(this.outDir, record.localPath) : null;
  }

  private localFilePath(url: string, hash: string, contentType: string | null): string {
    const parsed = new URL(url);
    const isCryengine = parsed.hostname === "www.cryengine.com";
    const prefix = isCryengine ? [] : ["external", sanitizeSegment(parsed.hostname)];
    const rawSegments = parsed.pathname.split("/").filter(Boolean);
    const segments = rawSegments.length > 0 ? rawSegments.map(sanitizeSegment) : ["index"];
    let fileName = segments.pop() || "index";
    let ext = path.extname(fileName);

    if (!ext) {
      ext = contentTypeExtension(contentType) || ".bin";
      fileName = `${fileName}${ext}`;
    }

    if (parsed.search) {
      const base = fileName.slice(0, fileName.length - ext.length);
      fileName = `${base}-${hash.slice(0, 8)}${ext}`;
    }

    let localPath = path.join(this.outDir, ...prefix, ...segments, fileName);
    if (this.usedLocalPaths.has(localPath)) {
      const currentExt = path.extname(localPath);
      localPath = path.join(
        path.dirname(localPath),
        `${path.basename(localPath, currentExt)}-${hash.slice(0, 8)}${currentExt}`
      );
    }

    this.usedLocalPaths.add(localPath);
    return localPath;
  }

  private relativeFromDir(localPath: string, fromDir: string): string {
    const absolute = path.isAbsolute(localPath) ? localPath : path.join(this.outDir, localPath);
    return path.relative(fromDir, absolute).replace(/\\/g, "/");
  }
}

function sanitizeSegment(segment: string): string {
  let decoded = segment;
  try {
    decoded = decodeURIComponent(segment);
  } catch {
    decoded = segment;
  }

  const sanitized = decoded
    .replace(/[<>:"|?*\x00-\x1f]/g, "_")
    .replace(/^\.+$/, "_")
    .trim();

  return sanitized || "_";
}

function isMediaUrl(value: string, pageUrl: string): boolean {
  try {
    const parsed = new URL(value, pageUrl);
    return MEDIA_EXTENSIONS.has(path.extname(parsed.pathname).toLowerCase());
  } catch {
    return false;
  }
}

async function localizeMedia(articleHtml: string, pageUrl: string, assetStore: AssetStore, enabled: boolean): Promise<string> {
  if (!enabled) {
    return articleHtml;
  }

  const root = parse(`<div>${articleHtml}</div>`);
  const srcSelectors = [
    "audio[src]",
    "embed[src]",
    "img[src]",
    "source[src]",
    "track[src]",
    "video[src]"
  ];

  for (const element of root.querySelectorAll(srcSelectors.join(","))) {
    await localizeAttribute(element, "src", pageUrl, assetStore);
  }

  for (const element of root.querySelectorAll("video[poster]")) {
    await localizeAttribute(element, "poster", pageUrl, assetStore);
  }

  for (const element of root.querySelectorAll("img[srcset], source[srcset]")) {
    await localizeSrcset(element, pageUrl, assetStore);
  }

  for (const anchor of root.querySelectorAll("a[href]")) {
    const href = anchor.getAttribute("href");
    if (!href || href.startsWith("#") || href.startsWith("mailto:") || !isMediaUrl(href, pageUrl)) {
      continue;
    }

    anchor.setAttribute("href", await assetStore.localPath(new URL(href, pageUrl).toString()));
  }

  return root.firstChild?.toString() || articleHtml;
}

async function localizeAttribute(
  element: HTMLElement,
  attribute: string,
  pageUrl: string,
  assetStore: AssetStore
): Promise<void> {
  const value = element.getAttribute(attribute);
  if (!value || value.startsWith("data:")) {
    return;
  }

  element.setAttribute(attribute, await assetStore.localPath(new URL(value, pageUrl).toString()));
}

async function localizeSrcset(element: HTMLElement, pageUrl: string, assetStore: AssetStore): Promise<void> {
  const srcset = element.getAttribute("srcset");
  if (!srcset) {
    return;
  }

  const rewritten = [];
  for (const candidate of srcset.split(",")) {
    const trimmed = candidate.trim();
    if (!trimmed) {
      continue;
    }

    const [urlPart, ...descriptor] = trimmed.split(/\s+/);
    if (!urlPart || urlPart.startsWith("data:")) {
      rewritten.push(trimmed);
      continue;
    }

    const local = await assetStore.localPath(new URL(urlPart, pageUrl).toString());
    rewritten.push([local, ...descriptor].join(" "));
  }

  element.setAttribute("srcset", rewritten.join(", "));
}

async function discoverStylesheets(rootUrl: string, retries: number): Promise<string[]> {
  const html = await fetchText(rootUrl, retries);
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

    const url = new URL(href, rootUrl).toString();
    if (!seen.has(url)) {
      seen.add(url);
      stylesheets.push(url);
    }
  }

  return stylesheets;
}

async function mirrorStylesheets(
  stylesheetUrls: string[],
  assetStore: AssetStore
): Promise<{ pageHrefs: string[]; indexHrefs: string[] }> {
  for (const stylesheetUrl of stylesheetUrls) {
    await assetStore.localPath(stylesheetUrl);
    const localPath = assetStore.absolutePath(stylesheetUrl);
    if (!localPath) {
      continue;
    }

    const response = await fetchWithRetries(stylesheetUrl, assetStore.retries);
    const css = await response.text();
    const rewritten = await rewriteCssUrls(css, stylesheetUrl, localPath, assetStore);
    await writeFile(localPath, rewritten, "utf8");
  }

  return {
    pageHrefs: await Promise.all(stylesheetUrls.map((url) => assetStore.localPath(url))),
    indexHrefs: await Promise.all(stylesheetUrls.map((url) => assetStore.localPathFrom(url, assetStore.outDir)))
  };
}

async function rewriteCssUrls(
  css: string,
  stylesheetUrl: string,
  stylesheetLocalPath: string,
  assetStore: AssetStore
): Promise<string> {
  const pattern = /url\(\s*(["']?)(?!data:|#|about:)([^"')]+)\1\s*\)/gi;
  let output = "";
  let lastIndex = 0;

  for (const match of css.matchAll(pattern)) {
    const matchIndex = match.index ?? 0;
    const rawUrl = match[2].trim();
    output += css.slice(lastIndex, matchIndex);

    try {
      const absolute = new URL(rawUrl, stylesheetUrl).toString();
      const local = await assetStore.localPathFrom(absolute, path.dirname(stylesheetLocalPath));
      output += `url("${local}")`;
    } catch {
      output += match[0];
    }

    lastIndex = matchIndex + match[0].length;
  }

  output += css.slice(lastIndex);
  return output;
}

function rewriteDocLinks(articleHtml: string, urlToFile: Map<string, string>): string {
  const root = parse(`<div>${articleHtml}</div>`);

  for (const anchor of root.querySelectorAll("a[href]")) {
    const href = anchor.getAttribute("href");
    if (!href) {
      continue;
    }

    let url: URL;
    try {
      url = new URL(href);
    } catch {
      continue;
    }

    if (url.protocol !== "http:" && url.protocol !== "https:") {
      continue;
    }

    const hash = url.hash;
    url.hash = "";

    const local = urlToFile.get(url.toString());
    if (local) {
      anchor.setAttribute("href", `${local}${hash}`);
    }
  }

  return root.firstChild?.toString() || articleHtml;
}

function pageDocument(
  page: Page,
  articleHtml: string,
  pages: Page[],
  index: number,
  stylesheetHrefs: string[]
): string {
  const previous = pages[index - 1];
  const next = pages[index + 1];
  const source = htmlEscape(page.url);
  const sidebar = globalToc(pages, page.fileName, "");

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${htmlEscape(page.title)} - CRYENGINE 3 Manual</title>
  ${stylesheetHrefs.map((href) => `<link rel="stylesheet" href="${htmlEscape(href)}">`).join("\n  ")}
  <link rel="stylesheet" href="../style.css">
</head>
<body>
  <header class="offline-topbar">
    <a href="../index.html">CRYENGINE 3 Manual</a>
    <nav>
      ${previous ? `<a href="${previous.fileName}">Previous</a>` : ""}
      ${next ? `<a href="${next.fileName}">Next</a>` : ""}
      <a href="${source}">Source</a>
    </nav>
  </header>
  <div class="offline-layout">
    ${sidebar}
    <main class="offline-doc">
      <h1>${htmlEscape(page.title)}</h1>
      ${articleHtml}
    </main>
  </div>
  ${tocScript()}
</body>
</html>
`;
}

function indexDocument(pages: Page[], scrapedAt: string, stylesheetHrefs: string[]): string {
  const sidebar = globalToc(pages, null, "pages/");

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
    ${sidebar}
    <main class="offline-doc offline-index">
      <h1>CRYENGINE 3 Manual</h1>
      <p class="offline-muted">Content-only offline mirror. Scraped ${htmlEscape(scrapedAt)}.</p>
      <p>Use the table of contents to jump to any mirrored route. The same TOC is available from every page.</p>
    </main>
  </div>
  ${tocScript()}
</body>
</html>
`;
}

async function refreshExistingPageShell(
  outputPath: string,
  page: Page,
  pages: Page[],
  index: number,
  stylesheetHrefs: string[]
): Promise<ManifestPage> {
  const existingHtml = await readFile(outputPath, "utf8");
  const root = parse(existingHtml);
  const main = root.querySelector(".offline-doc") || root.querySelector(".doc") || root.querySelector("main");

  if (!main) {
    return {
      url: page.url,
      title: page.title,
      fileName: page.fileName!,
      categoryPath: page.categoryPath,
      status: "skipped"
    };
  }

  const heading = main.querySelector("h1");
  if (heading) {
    heading.remove();
  }

  const document = pageDocument(page, main.innerHTML, pages, index, stylesheetHrefs);
  await writeFile(outputPath, document, "utf8");

  return {
    url: page.url,
    title: page.title,
    fileName: page.fileName!,
    categoryPath: page.categoryPath,
    status: "refreshed",
    htmlSha256: sha256(document)
  };
}

function globalToc(pages: Page[], currentFileName: string | null | undefined, prefix: string): string {
  return `<aside class="offline-toc" aria-label="Table of contents">
    <div class="offline-toc-inner">
      <a class="offline-toc-home" href="${prefix ? "index.html" : "../index.html"}">CRYENGINE 3 Manual</a>
      <input class="offline-toc-filter" type="search" placeholder="Filter ${pages.length} pages" aria-label="Filter table of contents">
      <ol class="offline-toc-list">
${renderTocGroup(buildTocTree(pages), currentFileName, prefix, 0)}
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

function renderTocGroup(
  group: TocGroup,
  currentFileName: string | null | undefined,
  prefix: string,
  depth: number
): string {
  const entries: string[] = [];

  for (const page of group.pages) {
    entries.push(renderTocPage(page, currentFileName, prefix));
  }

  for (const child of group.children) {
    const visibleTitle = htmlEscape(child.title);
    const current = child.indexPage?.fileName === currentFileName;
    const categoryTitle = child.indexPage
      ? `<a class="offline-toc-category-link" href="${tocHref(child.indexPage, prefix)}"${current ? ' aria-current="page"' : ""}>${visibleTitle}</a>`
      : `<span class="offline-toc-category-title">${visibleTitle}</span>`;
    entries.push(`<li class="offline-toc-category${current ? " is-current" : ""}" data-title="${visibleTitle.toLowerCase()}">
      <details open data-category-key="${htmlEscape(child.key)}">
        <summary><span class="offline-toc-category-title">${categoryTitle}</span></summary>
        <ol class="offline-toc-children" data-depth="${depth + 1}">
${renderTocGroup(child, currentFileName, prefix, depth + 1)}
        </ol>
      </details>
    </li>`);
  }

  return entries.join("\n");
}

function renderTocPage(page: Page, currentFileName: string | null | undefined, prefix: string): string {
  const current = page.fileName === currentFileName;
  const title = htmlEscape(page.title);
  const href = tocHref(page, prefix);
  const ordinal = page.fileName?.slice(0, 4) || "000";
  return `<li class="offline-toc-item${current ? " is-current" : ""}" data-title="${title.toLowerCase()}">
    <a href="${href}"${current ? ' aria-current="page"' : ""}>
      <span class="offline-toc-number">${ordinal}</span>
      <span>${title}</span>
    </a>
  </li>`;
}

function tocHref(page: Page, prefix: string): string {
  return `${prefix}${page.fileName}`;
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
  const current = document.querySelector('.offline-toc-item.is-current, .offline-toc-category.is-current');

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
    if (savedScroll) {
      list.scrollTop = Number(savedScroll) || 0;
    } else if (current) {
      list.scrollTop = Math.max(0, current.offsetTop - list.clientHeight / 2);
    }
    list.addEventListener('scroll', () => {
      const href = current?.querySelector('a')?.getAttribute('href') || location.pathname;
      localStorage.setItem(storagePrefix + 'scrollTop:' + href, String(list.scrollTop));
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

const STYLE = `:root {
  color-scheme: light dark;
  --bg: #f7f7f5;
  --fg: #1f2328;
  --muted: #667085;
  --border: #d0d7de;
  --panel: #ffffff;
  --link: #0969da;
  --note: #fff8c5;
  --code: #f0f3f6;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #101418;
    --fg: #e6edf3;
    --muted: #9da7b3;
    --border: #30363d;
    --panel: #151b23;
    --link: #79c0ff;
    --note: #352f16;
    --code: #20262d;
  }
}

* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.55 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.offline-topbar {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  justify-content: space-between;
  gap: 24px;
  padding: 12px 18px;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}
.offline-topbar nav { display: flex; gap: 14px; }
.offline-layout {
  display: grid;
  grid-template-columns: minmax(240px, 320px) minmax(0, 1040px);
  gap: 24px;
  align-items: start;
  width: min(1420px, calc(100vw - 32px));
  margin: 24px auto 56px;
}
.offline-toc {
  position: sticky;
  top: 57px;
  max-height: calc(100vh - 74px);
  overflow: hidden;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
}
.offline-toc-inner {
  display: flex;
  flex-direction: column;
  min-height: 0;
  max-height: calc(100vh - 74px);
}
.offline-toc-home {
  padding: 14px 16px 10px;
  font-weight: 700;
  text-decoration: none;
}
.offline-toc-filter {
  width: calc(100% - 24px);
  margin: 0 12px 10px;
  padding: 9px 10px;
  color: var(--fg);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 4px;
}
.offline-toc-list {
  margin: 0;
  padding: 0 0 10px;
  overflow: auto;
  list-style: none;
}
.offline-toc-children {
  margin: 0;
  padding: 0 0 0 12px;
  list-style: none;
}
.offline-toc-category summary {
  display: flex;
  align-items: center;
  min-height: 32px;
  padding: 7px 12px;
  color: var(--muted);
  font-weight: 700;
  cursor: pointer;
  user-select: none;
}
.offline-toc-category summary:hover {
  color: var(--fg);
  background: color-mix(in srgb, var(--link) 8%, transparent);
}
.offline-toc-category-title {
  min-width: 0;
  overflow-wrap: anywhere;
}
.offline-toc-category-link {
  color: inherit;
  text-decoration: none;
}
.offline-toc-category-link:hover {
  color: var(--link);
}
.offline-toc-category.is-current > details > summary {
  color: var(--link);
  background: color-mix(in srgb, var(--link) 12%, transparent);
  border-left: 3px solid var(--link);
  padding-left: 9px;
}
.offline-toc-item a {
  display: grid;
  grid-template-columns: 40px 1fr;
  gap: 8px;
  padding: 7px 12px;
  color: var(--fg);
  text-decoration: none;
  border-left: 3px solid transparent;
}
.offline-toc-item a:hover {
  background: color-mix(in srgb, var(--link) 10%, transparent);
}
.offline-toc-item.is-current a {
  color: var(--link);
  background: color-mix(in srgb, var(--link) 12%, transparent);
  border-left-color: var(--link);
  font-weight: 700;
}
.offline-toc-number {
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
.offline-doc {
  width: min(1040px, calc(100vw - 32px));
  min-width: 0;
  margin: 0;
  padding: 40px 44px 64px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
}
.offline-index { min-height: 360px; }
a { color: var(--link); }
img { max-width: 100%; height: auto; }
table { width: 100%; border-collapse: collapse; margin: 1rem 0; }
th, td { border: 1px solid var(--border); padding: 8px 10px; vertical-align: top; }
pre, code { background: var(--code); border-radius: 4px; }
pre { overflow: auto; padding: 12px; }
code { padding: 1px 4px; }
blockquote, .note {
  margin: 1rem 0;
  padding: 12px 14px;
  border-left: 4px solid var(--border);
  background: var(--note);
}
.note__header { font-weight: 700; }
.offline-muted { color: var(--muted); }
@media (max-width: 900px) {
  .offline-topbar { position: static; align-items: flex-start; flex-direction: column; }
  .offline-layout {
    width: 100%;
    margin: 0;
    display: block;
  }
  .offline-toc {
    position: static;
    max-height: 48vh;
    border-left: 0;
    border-right: 0;
    border-radius: 0;
  }
  .offline-toc-inner { max-height: 48vh; }
  .offline-doc {
    width: 100%;
    padding: 24px 18px 40px;
    border-left: 0;
    border-right: 0;
    border-radius: 0;
  }
}
`;

async function exists(filePath: string): Promise<boolean> {
  try {
    await stat(filePath);
    return true;
  } catch {
    return false;
  }
}

async function readExistingMedia(outDir: string): Promise<Array<{ url: string } & AssetRecord>> {
  try {
    const manifest = JSON.parse(await readFile(path.join(outDir, "manifest.json"), "utf8")) as Partial<Manifest>;
    return Array.isArray(manifest.media) ? manifest.media : [];
  } catch {
    return [];
  }
}

async function runPagePool(
  pages: Page[],
  concurrency: number,
  processPage: (index: number) => Promise<ManifestPage>
): Promise<ManifestPage[]> {
  const results = new Array<ManifestPage>(pages.length);
  let nextIndex = 0;

  async function worker(): Promise<void> {
    for (;;) {
      const index = nextIndex;
      nextIndex += 1;
      if (index >= pages.length) {
        return;
      }

      results[index] = await processPage(index);
    }
  }

  const workers = Array.from(
    { length: Math.min(concurrency, pages.length) },
    () => worker()
  );
  await Promise.all(workers);
  return results;
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));
  const pageDir = path.join(options.outDir, "pages");

  console.log(`Discovering pages from ${options.rootUrl}`);
  const discovered = await discoverPages(options.rootUrl, options.retries);
  const stylesheetUrls = await discoverStylesheets(options.rootUrl, options.retries);
  const pages = discovered.slice(0, options.limit);
  pages.forEach((page, index) => {
    page.fileName = pageFileName(index, page.title, page.url);
  });

  const urlToFile = new Map(pages.map((page) => [page.url, page.fileName!]));
  const scrapedAt = new Date().toISOString();
  const assetStore = new AssetStore(options.outDir, pageDir, options.retries);
  assetStore.seed(await readExistingMedia(options.outDir));
  const manifest: Manifest = {
    rootUrl: options.rootUrl,
    scrapedAt,
    pages: [],
    media: [],
    mediaFailures: []
  };

  await mkdir(pageDir, { recursive: true });
  await writeFile(path.join(options.outDir, "style.css"), STYLE, "utf8");
  const stylesheets = await mirrorStylesheets(stylesheetUrls, assetStore);

  console.log(`Found ${discovered.length} pages; selected ${pages.length}; stylesheets ${stylesheetUrls.length}`);

  manifest.pages = await runPagePool(pages, options.concurrency, async (index) => {
    const page = pages[index];
    const outputPath = path.join(pageDir, page.fileName!);
    const prefix = `[${index + 1}/${pages.length}]`;

    if (options.resume && (await exists(outputPath))) {
      console.log(`${prefix} refresh shell ${page.title}`);
      return refreshExistingPageShell(outputPath, page, pages, index, stylesheets.pageHrefs);
    }

    try {
      console.log(`${prefix} fetch ${page.url}`);
      const sourceHtml = await fetchText(page.url, options.retries);
      const extracted = extractArticle(sourceHtml, page.url);
      page.title = extracted.title;
      page.sourceSha256 = sha256(sourceHtml);

      const localized = await localizeMedia(extracted.articleHtml, page.url, assetStore, options.downloadAssets);
      const linked = rewriteDocLinks(localized, urlToFile);
      const document = pageDocument(page, linked, pages, index, stylesheets.pageHrefs);

      await writeFile(outputPath, document, "utf8");
      if (options.delayMs > 0) {
        await sleep(options.delayMs);
      }

      return {
        url: page.url,
        title: page.title,
        fileName: page.fileName!,
        categoryPath: page.categoryPath,
        status: "ok",
        sourceSha256: page.sourceSha256,
        htmlSha256: sha256(document)
      };
    } catch (error) {
      console.error(`${prefix} error ${page.url}: ${error instanceof Error ? error.message : String(error)}`);
      return {
        url: page.url,
        title: page.title,
        fileName: page.fileName!,
        categoryPath: page.categoryPath,
        status: "error",
        error: error instanceof Error ? error.message : String(error)
      };
    }
  });

  manifest.media = [...assetStore.assets.entries()].map(([url, asset]) => ({ url, ...asset }));
  manifest.mediaFailures = assetStore.failures;

  await writeFile(path.join(options.outDir, "index.html"), indexDocument(pages, scrapedAt, stylesheets.indexHrefs), "utf8");
  await writeFile(path.join(options.outDir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

  const ok = manifest.pages.filter((page) => page.status === "ok").length;
  const skipped = manifest.pages.filter((page) => page.status === "skipped").length;
  const refreshed = manifest.pages.filter((page) => page.status === "refreshed").length;
  const errors = manifest.pages.filter((page) => page.status === "error").length;
  console.log(`Done. ok=${ok} refreshed=${refreshed} skipped=${skipped} errors=${errors} out=${options.outDir}`);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? (error.stack || error.message) : String(error));
  process.exitCode = 1;
});
