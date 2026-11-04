import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { readDocuments, website, root, parseMarkdown } from "../lib/content";
import { visit } from "unist-util-visit";
import { siteOrigin, publicUrl } from "../lib/site";
import { operations, validateContract } from "../lib/contract";
import { inspectHtml, text, validateLink } from "../lib/export-check";

const out = path.join(website, "out");
const documents = readDocuments();
const spec = JSON.parse(
  readFileSync(path.join(out, "openapi/v1.json"), "utf8"),
);
const version = /^version = "([^"]+)"$/m.exec(
  readFileSync(path.join(root, "crates/windwatcher-server/Cargo.toml"), "utf8"),
)![1];
const entries = validateContract(spec, version);
assert.deepEqual(
  spec,
  JSON.parse(
    readFileSync(path.join(website, "generated/openapi-v1.json"), "utf8"),
  ),
);
assert.equal(
  readFileSync(path.join(out, "openapi/v1.json"), "utf8"),
  readFileSync(path.join(website, "generated/openapi-v1.json"), "utf8"),
  "Published OpenAPI must be byte-identical to the exported input",
);

const expected = [
  ...documents.map((doc) => doc.url),
  ...entries.map((entry) => entry.url),
];
assert.equal(
  new Set(expected).size,
  expected.length,
  "Page URLs must be unique",
);
for (const url of expected)
  assert.ok(
    existsSync(path.join(out, url, "index.html")),
    `Missing page: ${url}`,
  );
for (const doc of documents) {
  const html = inspectHtml(
    readFileSync(path.join(out, doc.url, "index.html"), "utf8"),
  );
  assert.deepEqual(
    html.headings,
    [doc.title],
    `Expected exactly the original H1 in ${doc.file}`,
  );
  for (const heading of doc.headings)
    assert.ok(
      html.ids.has(heading.id),
      `Missing original anchor ${doc.file}#${heading.id}`,
    );
  for (const code of doc.code)
    assert.ok(
      html.code.includes(code),
      `Code example changed in ${doc.file}: ${code.slice(0, 70)}`,
    );
}
for (const entry of operations(spec)) {
  const html = inspectHtml(
    readFileSync(path.join(out, entry.url, "index.html"), "utf8"),
  );
  const rendered = text(html.tree);
  assert.ok(rendered.includes(entry.id), `Missing operation ID: ${entry.id}`);
  assert.ok(
    rendered.includes(entry.path),
    `Missing operation path: ${entry.id}`,
  );
  for (const [status, response] of Object.entries(
    entry.operation.responses ?? {},
  )) {
    assert.ok(
      rendered.includes(status),
      `Missing response ${entry.id}: ${status}`,
    );
    if (response && "headers" in response)
      for (const name of Object.keys(response.headers ?? {})) {
        assert.ok(
          rendered.includes(name),
          `Missing response header ${entry.id}: ${name}`,
        );
      }
  }
}

let count = 0;
function walk(dir: string) {
  for (const file of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, file.name);
    if (file.isDirectory()) walk(full);
    else if (file.name.endsWith(".html")) {
      const relative = path.relative(out, full);
      const url = `/${relative.replace(/index\.html$/, "")}`;
      for (const href of inspectHtml(readFileSync(full, "utf8")).links)
        validateLink(out, href, url);
      count++;
    }
  }
}
walk(out);
const search = JSON.parse(
  readFileSync(path.join(out, "search-index.json"), "utf8"),
);
assert.equal(search.type, "default");
assert.ok(Object.keys(search.raw).length > 0, "Empty static search data");
const searchData = Object.values(search.raw).join("\n");
for (const url of expected)
  assert.ok(
    searchData.includes(url.replace(/\/$/, "")),
    `Missing searchable page: ${url}`,
  );
const llmsIndex = readFileSync(path.join(out, "llms.txt"), "utf8");
const llmsFull = readFileSync(path.join(out, "llms-full.txt"), "utf8");
for (const doc of documents)
  assert.ok(llmsFull.includes(doc.title), `Missing LLM document: ${doc.file}`);
for (const entry of entries)
  assert.ok(llmsFull.includes(entry.id), `Missing LLM operation: ${entry.id}`);
assert.ok(llmsIndex.includes("/openapi/v1.json"));
for (const url of expected) {
  const slug = url.replace(/^\/docs\//, "").replace(/\/$/, "");
  assert.ok(
    existsSync(path.join(out, "markdown/docs", slug, "content.md")),
    `Missing static Markdown: ${url}`,
  );
  const markdown = readFileSync(
    path.join(out, "markdown/docs", slug, "content.md"),
    "utf8",
  );
  const original = documents.find((document) => document.url === url);
  const code: string[] = [];
  visit(parseMarkdown(markdown), (node) => {
    if (node.type === "code") code.push(node.value);
    if (
      node.type !== "link" &&
      node.type !== "image" &&
      node.type !== "definition"
    )
      return;
    // Authored absolute URLs remain external even in a root-prefix test build.
    if (
      /^[a-z][a-z\d+.-]*:/i.test(node.url) &&
      original?.links.includes(node.url)
    )
      return;
    const target = new URL(node.url, `${siteOrigin}${publicUrl(url)}`);
    if (target.origin === siteOrigin)
      validateLink(out, `${target.pathname}${target.hash}`, url);
  });
  if (original)
    assert.deepEqual(
      code,
      original.code,
      `Markdown export changed code in ${original.file}`,
    );
}
console.log(
  `Validated ${documents.length} source documents, ${entries.length} operations, ${count} exported HTML files, links, anchors, code examples and static search data.`,
);
