import assert from "node:assert/strict";
import { test } from "node:test";
import {
  documentUrl,
  readDocuments,
  resolveContentUrl,
  parseMarkdown,
} from "../lib/content";
import { remarkDocs } from "../lib/remark-docs";
import path from "node:path";
import { root } from "../lib/content";

test("README indexes, ADR template and contributing have stable routes", () => {
  assert.equal(documentUrl("docs/README.md"), "/docs/");
  assert.equal(documentUrl("docs/adr/README.md"), "/docs/adr/");
  assert.equal(
    documentUrl("docs/adr/0000-template.md"),
    "/docs/adr/0000-template/",
  );
  assert.equal(documentUrl("CONTRIBUTING.md"), "/docs/contributing/");
});

test("resolves links relative to their source, preserving fragments and external URLs", () => {
  assert.equal(
    resolveContentUrl("operations.md#health", "docs/http-api.md"),
    "/docs/operations/#health",
  );
  assert.equal(
    resolveContentUrl("../architecture.md", "docs/adr/README.md"),
    "/docs/architecture/",
  );
  assert.equal(
    resolveContentUrl("../CONTRIBUTING.md", "docs/README.md"),
    "/docs/contributing/",
  );
  assert.equal(
    resolveContentUrl("../packaging/README.md", "docs/installation.md"),
    "https://github.com/OtavioGonzaga/windwatcher/blob/main/packaging/README.md",
  );
  for (const url of [
    "https://example.com/docs.md#x",
    "#same-page",
    "mailto:hello@example.com",
    "//example.com",
    "/v1/users",
  ]) {
    assert.equal(resolveContentUrl(url, "docs/http-api.md"), url);
  }
  assert.throws(
    () => resolveContentUrl("missing.md", "docs/README.md"),
    /Missing local target/,
  );
});

test("AST transformation preserves code content, including Markdown-looking links", () => {
  const code = "[not a link](missing.md)\n# async fn example() {}";
  const tree = parseMarkdown(
    "```rust,no_run\n" + code + "\n```\n\n[Architecture](architecture.md)",
  );
  remarkDocs()(tree, { path: path.join(root, "docs/README.md") });
  assert.equal(tree.children[0].type, "code");
  if (tree.children[0].type === "code") {
    assert.equal(tree.children[0].value, code);
    assert.equal(tree.children[0].lang, "rust");
  }
});

test("every source has a title, unique route and resolvable local references", () => {
  const docs = readDocuments();
  assert.equal(new Set(docs.map((doc) => doc.url)).size, docs.length);
  assert.ok(docs.some((doc) => doc.file === "docs/adr/0000-template.md"));
  for (const doc of docs)
    for (const link of doc.links) resolveContentUrl(link, doc.file);
});
