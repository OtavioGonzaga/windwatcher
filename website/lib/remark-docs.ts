import type { Root } from "mdast";
import { visit } from "unist-util-visit";
import path from "node:path";
import { root, resolveContentUrl } from "./content";

/** Rustdoc fence attributes are metadata, not part of the highlight language. */
export function remarkDocs() {
  return (tree: Root, file: { path: string }) => {
    const sourceFile = path.relative(root, file.path.split("?")[0]);
    visit(tree, "code", (node) => {
      if (node.lang?.startsWith("rust,")) node.lang = "rust";
    });
    visit(tree, (node) => {
      if (
        node.type === "link" ||
        node.type === "image" ||
        node.type === "definition"
      ) {
        node.url = resolveContentUrl(node.url, sourceFile);
      }
    });
  };
}
