import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fromHtml } from "hast-util-from-html";
import { visit } from "unist-util-visit";
import type { Nodes } from "hast";
import { basePath } from "./site";

export function text(node: Nodes): string {
  if (
    node.type === "element" &&
    (node.properties.className as string[] | undefined)?.includes("sr-only")
  )
    return "";
  if (node.type === "text") return node.value;
  if ("children" in node) return node.children.map(text).join("");
  return "";
}

export function inspectHtml(html: string) {
  const tree = fromHtml(html);
  const ids = new Set<string>();
  const links: string[] = [];
  const code: string[] = [];
  const headings: string[] = [];
  visit(tree, "element", (node) => {
    if (typeof node.properties.id === "string") ids.add(node.properties.id);
    for (const name of ["href", "src"]) {
      const value = node.properties[name];
      if (typeof value === "string") links.push(value);
    }
    if (node.tagName === "pre") code.push(text(node).replace(/\n$/, ""));
    if (node.tagName === "h1") headings.push(text(node));
  });
  return { ids, links, code, headings, tree };
}

/** Resolve as a static host, deliberately without an SPA/Next fallback. */
export function exportedFile(out: string, url: URL): string {
  let pathname = decodeURIComponent(url.pathname);
  if (
    basePath &&
    pathname !== basePath &&
    !pathname.startsWith(`${basePath}/`)
  ) {
    throw new Error(`URL escapes the configured base path: ${pathname}`);
  }
  pathname = pathname.slice(basePath.length);
  const candidate = path.resolve(out, `.${pathname || "/"}`);
  if (candidate !== out && !candidate.startsWith(`${out}/`))
    throw new Error(`Unsafe export path: ${pathname}`);
  return existsSync(candidate) && statSync(candidate).isDirectory()
    ? path.join(candidate, "index.html")
    : candidate;
}

export function validateLink(out: string, href: string, from: string) {
  if (/^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(href)) return;
  const url = new URL(href, `https://static.example${basePath}${from}`);
  const target = exportedFile(out, url);
  if (!existsSync(target))
    throw new Error(`Missing exported target in ${from}: ${href}`);
  if (url.hash && target.endsWith(".html")) {
    const fragment = decodeURIComponent(url.hash.slice(1));
    if (!inspectHtml(readFileSync(target, "utf8")).ids.has(fragment)) {
      throw new Error(`Missing heading fragment in ${from}: ${href}`);
    }
  }
}
