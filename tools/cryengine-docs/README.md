# CRYENGINE 3 docs scraper

Scrapes the static CRYENGINE 3 manual rooted at:

https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113

The local scraper fetches CRYENGINE HTML, extracts only the article content, and
uses a Wrangler Worker with Cloudflare Browser Run to convert that content HTML
to Markdown.

## Setup

```sh
cd tools/cryengine-docs
npm install
wrangler login
```

## Run with local remote Wrangler dev

Start the Worker in one terminal:

```sh
export CLOUDFLARE_ACCOUNT_ID=<your-account-id> # optional when Wrangler already has a default
npm run dev
```

Then run the scraper in another terminal:

```sh
npm run scrape
```

Output defaults to `tools/cryengine-docs/out/cryengine-3`. The scraper and
Worker entrypoints are TypeScript files executed directly by Node/Wrangler.

## Content-only HTML mirror

This does not use Cloudflare. It fetches the CRYENGINE pages directly, extracts
only the article body, rewrites internal doc links to local files, and writes an
offline browser-friendly HTML mirror:

```sh
npm run scrape:html
```

Output defaults to `tools/cryengine-docs/out/cryengine-3-html`.

The HTML mirror includes:

- source CSS mirrored from CRYENGINE, including CSS-referenced fonts/images;
- article media mirrored into their original URL paths under the output root;
- a root `index.html`;
- a searchable global table of contents/sidebar on every page;
- a local `style.css` override that adds comfortable reading padding.

Useful options:

```sh
npm run scrape:html -- --concurrency 4
npm run scrape:html -- --concurrency 8 --delay-ms 100
npm run scrape:html -- --limit 20 --out out/cryengine-3-html-test
```

Running `npm run scrape:html` again in resume mode refreshes existing page shells
with the latest index/sidebar without refetching their article HTML.

You can rebuild just the root index while a scrape is still running:

```sh
npm run html:index
```

Useful options:

```sh
npm run scrape -- --out ../../resources/reference/cryengine-3-docs
npm run scrape -- --limit 10
npm run scrape -- --worker-url http://127.0.0.1:8787
npm run scrape -- --delay-ms 1000 --retries 8
npm run scrape -- --no-assets
npm run scrape -- --dry-run --limit 5
```

## Run against a deployed Worker

```sh
npm run deploy
npm run scrape -- --worker-url https://cryengine-docs-markdown.<your-subdomain>.workers.dev
```

## Output

- `index.md` - ordered offline index.
- `pages/*.md` - one Markdown file per CRYENGINE page.
- `assets/*` - downloaded article images when enabled.
- `cryengine-3-manual.md` - single combined Markdown file unless disabled.
- `manifest.json` - source URLs, output files, hashes, and scrape status.
