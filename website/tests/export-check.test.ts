import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { inspectHtml, validateLink, exportedFile } from "../lib/export-check";
import { basePath } from "../lib/site";

test("export inspection extracts actual anchors and preserves code characters", () => {
  const html = inspectHtml(
    '<h1 id="title">Title<span class="sr-only">Copy</span></h1><pre><code><span>&lt;T&gt; &amp; x</span>\n</code></pre><a href="other/#target">Next</a>',
  );
  assert.deepEqual(html.headings, ["Title"]);
  assert.deepEqual(html.code, ["<T> & x"]);
  assert.ok(html.ids.has("title"));
  assert.deepEqual(html.links, ["other/#target"]);
});

test("static validation rejects missing pages, fragments and wrong deployment prefixes", () => {
  const out = mkdtempSync(path.join(tmpdir(), "windwatcher-export-"));
  try {
    mkdirSync(path.join(out, "docs"), { recursive: true });
    writeFileSync(
      path.join(out, "docs/index.html"),
      '<h1 id="title">Title</h1>',
    );
    validateLink(out, `${basePath}/docs/#title`, "/docs/");
    assert.throws(
      () => validateLink(out, `${basePath}/docs/#missing`, "/docs/"),
      /Missing heading/,
    );
    assert.throws(
      () => validateLink(out, `${basePath}/missing/`, "/docs/"),
      /Missing exported target/,
    );
    if (basePath)
      assert.throws(
        () => exportedFile(out, new URL("https://example.com/docs/")),
        /base path/,
      );
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});
