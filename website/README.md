# Windwatcher documentation website

Fumadocs and Next.js export the repository documentation and the server's OpenAPI
contract to static files. Production needs no JavaScript runtime, database, search
service, API proxy or Windwatcher instance.

Target URL: <https://otaviogonzaga.github.io/windwatcher/>.

## Prerequisites

- The repository Rust toolchain (`../rust-toolchain.toml`). Cargo builds the
  server package to export OpenAPI; no configuration or credentials are needed.
- Bun 1.4.4 (declared in `.bun-version` and `package.json`). Node.js and pnpm
  are not required; Bun installs dependencies and runs all website tooling.
- Chromium for browser tests: `bun --bun playwright install --with-deps chromium`.

Run the following commands from `website/` (or use `bun run --cwd website <script>`
from the repository root):

```bash
bun install --frozen-lockfile
bun run dev
```

Open <http://localhost:3000/windwatcher/>. Markdown edits are watched directly;
restart development after changing Rust OpenAPI annotations or adding/removing
documentation assets. The preparation command refreshes generated assets.

## Build and validate

```bash
bun run format:check
bun run check
bun run test
bun run build
bun run validate
bun run test:e2e
bun run preview
```

- `check` regenerates inputs, generates MDX/Next types and runs TypeScript.
- `test` checks URL mapping, AST transformations, navigation, contract validation
  and static integrity-check behavior. Use `bun run test` rather than bare
  `bun test`, which would also discover the separate Playwright browser tests.
- `build` always exports the current Rust contract, validates the source inventory
  and copies assets before generating `out/`. Never deploy `.next/` or the source tree.
- `validate` checks all source pages against exported HTML, original heading
  anchors, unchanged code examples, local links/assets, operation coverage,
  response headers, schema references, raw JSON, search coverage and LLM exports.
- `test:e2e` starts an ordinary static server against `out/`, with no SPA fallback,
  then checks Chromium desktop/mobile navigation, refresh, search, themes, network
  errors and overflow. The report includes screenshots of docs, an API page and
  the landing page in light/dark themes on desktop and mobile.
- `preview` serves only `out/` at <http://127.0.0.1:4173/windwatcher/>. Build first;
  this is a local inspection tool, not a deployment server.

The Rust baseline in [Contributing](../CONTRIBUTING.md) still applies. Run tests
sequentially by package/target, with `CARGO_BUILD_JOBS=4` and
`-- --test-threads=1`. The dedicated docs workflow repeats the focused exporter
and existing OpenAPI contracts; the existing Rust CI remains unchanged.

## Content ownership and navigation

- `../docs/**/*.md` is authoritative, including both README indexes, every ADR
  and `0000-template.md`. `../CONTRIBUTING.md` is ingested as well.
- Collections in `source.config.ts` compile those files **directly**, without a
  second Markdown copy. Titles come from the first H1; the original H1 and its
  anchor remain in the page. Frontmatter is not required.
- `docs/README.md` maps to `/docs/`; `docs/adr/README.md` maps to `/docs/adr/`.
  Other paths retain their names without `.md`, with a trailing slash.
- AST transformations resolve Markdown links relative to the original file.
  Links to other docs become site routes; assets are copied to an ignored static
  directory; other repository files link to GitHub. Code examples are not rewritten.
  Rustdoc fence attributes are normalized only for syntax highlighting.
- New Markdown files automatically become routes and search records. Add their
  slug to a category in `lib/navigation.ts` for deliberate placement; unclassified
  pages remain visible under **More documentation**. No page is silently dropped.
- Branding, navigation and presentation belong here, not in Rust domain code.
  Rust library API documentation remains on <https://docs.rs/windwatcher>.

## OpenAPI generation

```bash
bun run openapi
```

This invokes the example in
`crates/windwatcher-server/examples/export_openapi.rs` with locked Cargo
dependencies and at most four build jobs. It calls
`windwatcher_server::http::openapi()`, the same contract used by the server's
Swagger UI, with no listener, database, queue or configuration access.

The exporter writes JSON only to stdout and fails on serialization/output errors.
`generated/openapi-v1.json` and `public/openapi/v1.json` receive identical bytes.
Neither is committed. Fumadocs `createOpenAPI()`/`staticSource()` generates a
virtual page per operation under `/docs/api/<lowercase-tag>/<operationId>/`.
The actual generated collection is compared to all exported method/path pairs.
New annotated operations appear automatically; missing/duplicate/invalid IDs,
unresolved schema references and page collisions fail validation.

The conceptual [HTTP guide](../docs/http-api.md) remains separate. Generated pages
show the declared security, requests, responses and schemas. A small supplemental
section renders response headers omitted by the default Fumadocs UI. No contract
is edited to accommodate the renderer, and no generated operation MDX is committed.

## Static search and agent artifacts

`/search-index.json` is generated at build time through Fumadocs' FlexSearch
static export. The browser downloads it and searches locally. Guides, ADRs and
API operations (including method, path and operation ID) are indexed. There is
no runtime `/api/search` endpoint or external service.
Search uses prefix matching to keep the client download bounded even with long
Rust identifiers and source paths.

Fumadocs' `llms()` support produces actual static `/llms.txt` and `/llms-full.txt`
files. Each page also has a **View as Markdown** link under
`/markdown/docs/<slug>/content.md` (the docs index omits `<slug>/`). Prose is read
from its authoritative Markdown and links are resolved for the published origin;
operation exports include their actual contract and reference the shared raw
OpenAPI JSON for schemas. There are no rewrites or content-negotiation middleware.

All paths above are relative to the deployment prefix, normally `/windwatcher`.

## GitHub Pages deployment

In repository **Settings → Pages → Build and deployment → Source**, select
**GitHub Actions**. Ensure Pages and Actions are enabled and that any
`github-pages` environment protection rules allow deployment from `main`.

`.github/workflows/docs.yml` runs **only manually**, through `workflow_dispatch`;
pull requests and pushes never start it. In **Actions → Documentation → Run
workflow**, select `main` to build, validate and publish. The workflow builds and
validates the root and project-prefix configurations sequentially, then uploads
only the final project-prefix `out/` with the official Pages Actions.
Dispatches from other branches validate but cannot deploy. Build has read-only source permissions;
only deploy receives `pages: write` and `id-token: write`.
Next.js uses the checked-out Git commit as its build ID rather than a random ID.

After merging, dispatch the workflow and verify its result and the actual public
URL. A merge or successful local build does **not** publish the website.

### Custom domains and local root hosting

Prefix/origin configuration is centralized in `lib/site.ts`:

```bash
NEXT_PUBLIC_BASE_PATH='' NEXT_PUBLIC_SITE_ORIGIN='https://docs.example.com' bun run build
NEXT_PUBLIC_BASE_PATH='' NEXT_PUBLIC_SITE_ORIGIN='https://docs.example.com' bun run validate
NEXT_PUBLIC_BASE_PATH='' bun run test:e2e
NEXT_PUBLIC_BASE_PATH='' bun run preview
```

Use the same values for generation, validation and preview; prefix changes require
a rebuild. For a real custom domain, also configure GitHub Pages DNS/domain settings
and change the workflow's publication prefix/origin. No navigation rewrite is needed.

## Limitations

- This publishes the current `main` documentation, not a multi-version archive.
- The API reference is read-only. Generated samples illustrate schemas, not
  guaranteed real response values; no live server or request proxy is configured.
- Search downloads a static index; revisit its size if the documentation grows
  substantially. It is not a hosted search service or an offline application.
- Local preview deliberately does not emulate server-side Next.js features.
- The exporter compiles the server and its adapters, so a cold documentation build
  costs more than a Markdown-only site; local Cargo and Bun caches mitigate this.
