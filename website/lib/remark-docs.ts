import type { Root } from "mdast";
import { visit } from "unist-util-visit";

/** Rustdoc fence attributes are metadata, not part of the highlight language. */
export function remarkDocs() {
  return (tree: Root) => {
    visit(tree, "code", (node) => {
      if (node.lang?.startsWith("rust,")) node.lang = "rust";
    });
  };
}
