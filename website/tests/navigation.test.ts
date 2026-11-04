import assert from "node:assert/strict";
import { test } from "node:test";
import type { Node } from "fumadocs-core/page-tree";
import { documentationTree } from "../lib/navigation";
import { readDocuments } from "../lib/content";

test("navigation includes each document once, including newly discovered pages", () => {
  const pages = readDocuments().map((doc) => ({
    url: doc.url,
    slugs: doc.slugs,
    data: { title: doc.title },
  }));
  pages.push({
    url: "/docs/future-guide/",
    slugs: ["future-guide"],
    data: { title: "New guide" },
  });
  const tree = documentationTree(pages);
  const urls: string[] = [];
  function walk(nodes: Node[]) {
    for (const node of nodes) {
      if (node.type === "page") urls.push(node.url);
      if (node.type === "folder") {
        if (node.index) urls.push(node.index.url);
        walk(node.children);
      }
    }
  }
  walk(tree.children);
  assert.equal(urls.length, pages.length);
  assert.equal(new Set(urls).size, pages.length);
  assert.ok(urls.includes("/docs/future-guide/"));
});
