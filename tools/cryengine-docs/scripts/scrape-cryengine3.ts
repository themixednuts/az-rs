import { createHash } from "node:crypto";
import {
  mkdir,
  readFile,
  stat,
  writeFile
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "node-html-parser";
import type HTMLElement from "node-html-parser/dist/nodes/html.js";

const DEFAULT_ROOT =
  "https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113";
const DEFAULT_WORKER_URL =
  process.env.CRYENGINE_DOCS_WORKER_URL || "http://127.0.0.1:8787";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROJECT_DIR = path.resolve(__dirname, "..");

type DiscoverMode = "local" | "worker";

type ScrapeOptions = {
  rootUrl: string;
  workerUrl: string;
  outDir: string;
  limit: number;
  delayMs: number;
  retries: number;
  resume: boolean;
  downloadAssets: boolean;
  combined: boolean;
  dryRun: boolean;
  discover: DiscoverMode;
};

type Page = {
  url: string;
  title: string;
  fileName?: string;
  sourceSha256?: string;
};

type ManifestPage = {
  url: string;
  title: string;
  fileName?: string;
  status: "ok" | "skipped" | "dry-run" | "error";
  markdownSha256?: string;
  sourceSha256?: string;
  contentHtmlBytes?: number;
  error?: string;
};

type AssetRecord = {
  fileName: string;
  sha256: string;
  bytes: number;
};

type AssetFailure = {
  url: string;
  error: string;
};

type Manifest = {
  rootUrl: string;
  scrapedAt: string;
  workerUrl: string | null;
  pages: ManifestPage[];
  assets: Array<{ url: string } & AssetRecord>;
  assetFailures: AssetFailure[];
};

type ExtractedContent = {
  title: string;
  articleHtml: string;
};

function parseArgs(argv: string[]): ScrapeOptions {
  const options: ScrapeOptions = {
    rootUrl: DEFAULT_ROOT,
    workerUrl: DEFAULT_WORKER_URL,
    outDir: path.join(PROJECT_DIR, "out", "cryengine-3"),
    limit: Number.POSITIVE_INFINITY,
    delayMs: 1000,
    retries: 8,
    resume: true,
    downloadAssets: true,
    combined: true,
    dryRun: false,
    discover: "local"
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case "--root":
        options.rootUrl = argv[++i];
        break;
      case "--worker-url":
        options.workerUrl = argv[++i];
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
      case "--discover":
        options.discover = argv[++i] as DiscoverMode;
        break;
      case "--dry-run":
        options.dryRun = true;
        break;
      case "--no-resume":
        options.resume = false;
        break;
      case "--no-assets":
        options.downloadAssets = false;
        break;
      case "--no-combined":
        options.combined = false;
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

  if (!["local", "worker"].includes(options.discover)) {
    throw new Error("--discover must be local or worker");
  }

  return options;
}

function printHelp() {
  console.log(`Usage: npm run scrape -- [options]

Options:
  --root <url>          Root CRYENGINE docs category URL.
  --worker-url <url>    Wrangler Worker URL. Defaults to ${DEFAULT_WORKER_URL}
  --out <dir>           Output directory. Defaults to tools/cryengine-docs/out/cryengine-3
  --limit <n>           Scrape only the first n pages.
  --delay-ms <n>        Delay between page conversions. Defaults to 1000.
  --retries <n>         Fetch retry count. Defaults to 8.
  --discover local|worker
                        Discover page links locally or through the Worker. Defaults to local.
  --dry-run             Discover and extract pages without calling the Markdown Worker.
  --no-resume           Rebuild pages even if Markdown files already exist.
  --no-assets           Do not download article images.
  --no-combined         Do not write cryengine-3-manual.md.
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
  return `${ordinal}-${slugify(title)}-${pageIdFromUrl(url)}.md`;
}

function pageUrlAllowed(url: string, rootUrl: string): boolean {
  const parsed = new URL(url);
  const root = new URL(rootUrl);
  const escapedRootPath = root.pathname.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pagePattern = new RegExp(`^${escapedRootPath}/pages/\\d+$`);
  return parsed.origin === root.origin && pagePattern.test(parsed.pathname);
}

async function fetchWithRetries(url: string, options: RequestInit = {}, retries = 3): Promise<Response> {
  let lastError: unknown;

  for (let attempt = 0; attempt <= retries; attempt += 1) {
    try {
      const response = await fetch(url, {
        ...options,
        headers: {
          "user-agent": "az-rs cryengine-docs offline scraper",
          ...Object.fromEntries(new Headers(options.headers || {}).entries())
        }
      });

      if (response.ok) {
        return response;
      }

      const body = await response.text().catch(() => "");
      const retryAfter = retryAfterMs(response);
      lastError = new Error(
        `${response.status} ${response.statusText} for ${url}${body ? `: ${body.slice(0, 300)}` : ""}`
      );

      if (![408, 429, 500, 502, 503, 504].includes(response.status)) {
        throw lastError;
      }

      if (response.status === 429 && retryAfter !== null && attempt < retries) {
        console.warn(`Rate limited by ${url}; waiting ${Math.round(retryAfter / 1000)}s before retry`);
        await sleep(retryAfter);
        continue;
      }
    } catch (error) {
      lastError = error;
    }

    if (attempt < retries) {
      await sleep(Math.min(30_000, 1_000 * 2 ** attempt));
    }
  }

  throw lastError;
}

function retryAfterMs(response: Response): number | null {
  const value = response.headers.get("retry-after");
  if (!value) {
    return response.status === 429 ? 20_000 : null;
  }

  const seconds = Number.parseInt(value, 10);
  if (Number.isFinite(seconds)) {
    return Math.max(1_000, seconds * 1_000);
  }

  const dateMs = Date.parse(value);
  if (Number.isFinite(dateMs)) {
    return Math.max(1_000, dateMs - Date.now());
  }

  return response.status === 429 ? 20_000 : null;
}

async function fetchText(url: string, retries: number): Promise<string> {
  const response = await fetchWithRetries(url, {}, retries);
  return response.text();
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

function removeNoise(root: HTMLElement): void {
  const selectors = [
    "script",
    "style",
    "svg",
    "form",
    "input",
    ".layout__details--aside",
    ".visibility-hidden"
  ];

  for (const selector of selectors) {
    for (const element of root.querySelectorAll(selector)) {
      element.remove();
    }
  }
}

function normalizeTablesForMarkdown(root: HTMLElement): void {
  for (const table of root.querySelectorAll("table")) {
    if (!table.querySelector("li")) {
      continue;
    }

    const rows = table.querySelectorAll("tr").map((row) => row.querySelectorAll("th, td"));
    const firstRow = rows[0] || [];
    const hasColumnHeadings =
      rows.length > 1 &&
      firstRow.length > 0 &&
      firstRow.every((cell) => cleanText(cell.textContent).length > 0 && cleanText(cell.textContent).length < 120);

    if (hasColumnHeadings) {
      const blocks: string[] = [];

      for (let column = 0; column < firstRow.length; column += 1) {
        blocks.push(`<h5>${firstRow[column].innerHTML}</h5>`);

        for (let row = 1; row < rows.length; row += 1) {
          const cell = rows[row][column];
          if (cell && cleanText(cell.textContent)) {
            blocks.push(`<div>${cell.innerHTML}</div>`);
          }
        }
      }

      table.replaceWith(`<div>${blocks.join("\n")}</div>`);
    }
  }

  for (const cell of root.querySelectorAll("td, th")) {
    for (const list of cell.querySelectorAll("ul, ol")) {
      const items = list.querySelectorAll("li");
      if (items.length === 0) {
        continue;
      }

      const itemHtml = items
        .map((item) => item.innerHTML.trim())
        .filter(Boolean)
        .join("<br>");

      list.replaceWith(`<p>${itemHtml}</p>`);
    }
  }
}

function extractContentHtml(html: string, pageUrl: string): ExtractedContent {
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
  normalizeTablesForMarkdown(content);

  const articleHtml = unwrapCustomConfluenceTags(
    `<article><h1>${escapeHtml(title)}</h1>${content.innerHTML}</article>`
  );

  return { title, articleHtml };
}

function escapeHtml(value: string): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

async function discoverLocal(rootUrl: string, retries: number): Promise<Page[]> {
  const html = await fetchText(rootUrl, retries);
  const root = parse(html);
  const pages: Page[] = [];
  const seen = new Set<string>();

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
      title: cleanText(anchor.getAttribute("title") || anchor.textContent) || pageIdFromUrl(url.toString())
    });
  }

  return pages;
}

async function discoverWorker(rootUrl: string, workerUrl: string, retries: number): Promise<Page[]> {
  const endpoint = new URL("/links", workerUrl);
  endpoint.searchParams.set("url", rootUrl);

  const response = await fetchWithRetries(endpoint.toString(), {}, retries);
  const data = await parseQuickActionResponse(response);
  const links = Array.isArray(data) ? data : [];
  const pages: Page[] = [];
  const seen = new Set<string>();

  for (const link of links) {
    const url = new URL(link, rootUrl);
    url.hash = "";

    if (!pageUrlAllowed(url.toString(), rootUrl) || seen.has(url.toString())) {
      continue;
    }

    seen.add(url.toString());
    pages.push({
      url: url.toString(),
      title: `page-${pageIdFromUrl(url.toString())}`
    });
  }

  return pages;
}

async function parseQuickActionResponse(response: Response): Promise<unknown> {
  const text = await response.text();
  const contentType = response.headers.get("content-type") || "";

  if (contentType.includes("application/json")) {
    const data = JSON.parse(text);
    if (data.success === false) {
      throw new Error(JSON.stringify(data.errors || data));
    }
    return data.result ?? data;
  }

  return text;
}

function workerEndpoint(workerUrl: string, pathName: string): string {
  const base = workerUrl.endsWith("/") ? workerUrl : `${workerUrl}/`;
  return new URL(pathName.replace(/^\//, ""), base).toString();
}

async function markdownFromWorker(workerUrl: string, html: string, retries: number): Promise<string> {
  const response = await fetchWithRetries(
    workerEndpoint(workerUrl, "/markdown"),
    {
      method: "POST",
      headers: {
        "content-type": "application/json"
      },
      body: JSON.stringify({ html })
    },
    retries
  );

  const result = await parseQuickActionResponse(response);
  if (typeof result !== "string") {
    throw new Error(`expected markdown string, got ${typeof result}`);
  }

  return normalizeMarkdown(result);
}

function normalizeMarkdown(markdown: string): string {
  return String(markdown)
    .replace(/\r\n/g, "\n")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{4,}/g, "\n\n\n")
    .trim()
    .concat("\n");
}

function yamlString(value: string | undefined): string {
  return JSON.stringify(String(value));
}

function frontmatter(page: Page, title: string, markdown: string, scrapedAt: string): string {
  return `---\ntitle: ${yamlString(title)}\nsource_url: ${yamlString(page.url)}\nscraped_at: ${yamlString(scrapedAt)}\nsource_sha256: ${yamlString(page.sourceSha256)}\n---\n\n${markdown}`;
}

function rewriteDocLinks(markdown: string, urlToFile: Map<string, string>): string {
  let result = markdown;

  for (const [url, file] of urlToFile.entries()) {
    result = result.replaceAll(url, `./${file}`);
  }

  return result;
}

async function exists(filePath: string): Promise<boolean> {
  try {
    await stat(filePath);
    return true;
  } catch {
    return false;
  }
}

function contentTypeExtension(contentType: string | null): string {
  const normalized = String(contentType || "").split(";")[0].trim().toLowerCase();
  const byType: Record<string, string> = {
    "image/jpeg": ".jpg",
    "image/png": ".png",
    "image/gif": ".gif",
    "image/webp": ".webp",
    "image/svg+xml": ".svg"
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

class AssetStore {
  assetDir: string;
  retries: number;
  assets: Map<string, AssetRecord>;
  failures: AssetFailure[];

  constructor(assetDir: string, retries: number) {
    this.assetDir = assetDir;
    this.retries = retries;
    this.assets = new Map();
    this.failures = [];
  }

  async localMarkdownPath(url: string): Promise<string> {
    if (this.assets.has(url)) {
      return `../assets/${this.assets.get(url).fileName}`;
    }

    try {
      await mkdir(this.assetDir, { recursive: true });
      const response = await fetchWithRetries(url, {}, this.retries);
      const buffer = Buffer.from(await response.arrayBuffer());
      const hash = sha256(buffer).slice(0, 16);
      const ext = urlExtension(url) || contentTypeExtension(response.headers.get("content-type")) || ".bin";
      const fileName = `${hash}${ext}`;
      const filePath = path.join(this.assetDir, fileName);
      await writeFile(filePath, buffer);
      this.assets.set(url, { fileName, sha256: sha256(buffer), bytes: buffer.length });
      return `../assets/${fileName}`;
    } catch (error) {
      this.failures.push({ url, error: error instanceof Error ? error.message : String(error) });
      return url;
    }
  }
}

async function localizeImages(
  articleHtml: string,
  pageUrl: string,
  assetStore: AssetStore,
  enabled: boolean
): Promise<string> {
  if (!enabled) {
    return articleHtml;
  }

  const root = parse(articleHtml);
  const images = root.querySelectorAll("img[src]");

  for (const image of images) {
    const src = image.getAttribute("src");
    if (!src || src.startsWith("data:")) {
      continue;
    }

    const absolute = new URL(src, pageUrl).toString();
    const localPath = await assetStore.localMarkdownPath(absolute);
    image.setAttribute("src", localPath);
  }

  return root.toString();
}

async function writeIndex(outDir: string, pages: Page[], manifest: Manifest): Promise<void> {
  const lines = [
    "# CRYENGINE 3 Manual",
    "",
    `Scraped pages: ${manifest.pages.filter((page) => page.status === "ok" || page.status === "skipped").length}`,
    "",
    "## Pages",
    ""
  ];

  for (const page of pages) {
    lines.push(`- [${page.title}](pages/${page.fileName})`);
  }

  await writeFile(path.join(outDir, "index.md"), `${lines.join("\n")}\n`, "utf8");
}

async function writeCombined(outDir: string, manifest: Manifest): Promise<void> {
  const chunks = ["# CRYENGINE 3 Manual", ""];

  for (const page of manifest.pages) {
    if ((page.status !== "ok" && page.status !== "skipped") || !page.fileName) {
      continue;
    }

    const markdown = await readFile(path.join(outDir, "pages", page.fileName), "utf8");
    chunks.push(markdown.replaceAll("../assets/", "assets/").trim(), "");
  }

  await writeFile(path.join(outDir, "cryengine-3-manual.md"), `${chunks.join("\n\n")}\n`, "utf8");
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const pageDir = path.join(options.outDir, "pages");
  const assetDir = path.join(options.outDir, "assets");

  console.log(`Discovering pages from ${options.rootUrl} (${options.discover})`);
  const discovered =
    options.discover === "worker"
      ? await discoverWorker(options.rootUrl, options.workerUrl, options.retries)
      : await discoverLocal(options.rootUrl, options.retries);

  const selected = discovered.slice(0, options.limit);
  selected.forEach((page, index) => {
    page.fileName = pageFileName(index, page.title, page.url);
  });

  const urlToFile = new Map(selected.map((page) => [page.url, page.fileName]));
  console.log(`Found ${discovered.length} pages; selected ${selected.length}`);

  await mkdir(pageDir, { recursive: true });
  if (options.downloadAssets) {
    await mkdir(assetDir, { recursive: true });
  }

  const assetStore = new AssetStore(assetDir, options.retries);
  const scrapedAt = new Date().toISOString();
  const manifest: Manifest = {
    rootUrl: options.rootUrl,
    scrapedAt,
    workerUrl: options.dryRun ? null : options.workerUrl,
    pages: [],
    assets: [],
    assetFailures: []
  };

  for (let index = 0; index < selected.length; index += 1) {
    const page = selected[index];
    const outputPath = path.join(pageDir, page.fileName);
    const prefix = `[${index + 1}/${selected.length}]`;

    if (options.resume && !options.dryRun && (await exists(outputPath))) {
      console.log(`${prefix} skip ${page.title}`);
      manifest.pages.push({
        url: page.url,
        title: page.title,
        fileName: page.fileName,
        status: "skipped"
      });
      continue;
    }

    try {
      console.log(`${prefix} fetch ${page.url}`);
      const sourceHtml = await fetchText(page.url, options.retries);
      const extracted = extractContentHtml(sourceHtml, page.url);
      const localizedHtml = await localizeImages(
        extracted.articleHtml,
        page.url,
        assetStore,
        options.downloadAssets
      );

      page.title = extracted.title || page.title;
      page.sourceSha256 = sha256(sourceHtml);

      if (options.dryRun) {
        manifest.pages.push({
          url: page.url,
          title: page.title,
          fileName: page.fileName,
          status: "dry-run",
          contentHtmlBytes: Buffer.byteLength(localizedHtml, "utf8"),
          sourceSha256: page.sourceSha256
        });
        continue;
      }

      const markdown = await markdownFromWorker(options.workerUrl, localizedHtml, options.retries);
      const rewritten = rewriteDocLinks(markdown, urlToFile);
      const finalMarkdown = frontmatter(page, page.title, rewritten, scrapedAt);

      await writeFile(outputPath, finalMarkdown, "utf8");
      manifest.pages.push({
        url: page.url,
        title: page.title,
        fileName: page.fileName,
        status: "ok",
        markdownSha256: sha256(finalMarkdown),
        sourceSha256: page.sourceSha256
      });

      if (options.delayMs > 0 && index + 1 < selected.length) {
        await sleep(options.delayMs);
      }
    } catch (error) {
      console.error(`${prefix} error ${page.url}: ${error instanceof Error ? error.message : String(error)}`);
      manifest.pages.push({
        url: page.url,
        title: page.title,
        fileName: page.fileName,
        status: "error",
        error: error instanceof Error ? error.message : String(error)
      });
    }
  }

  manifest.assets = [...assetStore.assets.entries()].map(([url, asset]) => ({ url, ...asset }));
  manifest.assetFailures = assetStore.failures;

  await writeIndex(options.outDir, selected, manifest);

  if (!options.dryRun && options.combined) {
    await writeCombined(options.outDir, manifest);
  }

  await writeFile(
    path.join(options.outDir, "manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
    "utf8"
  );

  const ok = manifest.pages.filter((page) => page.status === "ok").length;
  const skipped = manifest.pages.filter((page) => page.status === "skipped").length;
  const errors = manifest.pages.filter((page) => page.status === "error").length;
  console.log(`Done. ok=${ok} skipped=${skipped} errors=${errors} out=${options.outDir}`);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? (error.stack || error.message) : String(error));
  process.exitCode = 1;
});
