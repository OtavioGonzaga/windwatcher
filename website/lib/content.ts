import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import { toString } from "mdast-util-to-string";
import { visit } from "unist-util-visit";
import GithubSlugger from "github-slugger";
import type { Root } from "mdast";
import { publicUrl, repository } from "./site";

// All package scripts (including Next's build workers) run from website/.
export const root = path.resolve(process.cwd(), "..");
export const website = path.join(root, "website");
const parser = unified().use(remarkParse).use(remarkGfm);

export function parseMarkdown(markdown: string): Root {
  return parser.parse(markdown);
}

export function documentSlug(file: string): string[] {
  if (file === "CONTRIBUTING.md") return ["contributing"];
  const relative = file.replace(/^docs\//, "").replace(/\.md$/, "");
  return relative.split("/").filter((part) => part !== "README");
}

export function documentUrl(file: string) {
  const slug = documentSlug(file).join("/");
  return `/docs/${slug ? `${slug}/` : ""}`;
}

function markdownFiles(dir: string): string[] {
  return readdirSync(path.join(root, dir), { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name, "en"))
    .flatMap((entry) => {
      const file = `${dir}/${entry.name}`;
      return entry.isDirectory()
        ? markdownFiles(file)
        : file.endsWith(".md")
          ? [file]
          : [];
    });
}

export function readDocuments() {
  return [...markdownFiles("docs"), "CONTRIBUTING.md"].map((file) => {
    const markdown = readFileSync(path.join(root, file), "utf8");
    const tree = parseMarkdown(markdown);
    const slugger = new GithubSlugger();
    const headings: { id: string; content: string }[] = [];
    const links: string[] = [];
    const code: string[] = [];
    visit(tree, "heading", (node) => {
      const content = toString(node);
      headings.push({ id: slugger.slug(content), content });
    });
    visit(tree, (node) => {
      if (
        node.type === "link" ||
        node.type === "image" ||
        node.type === "definition"
      )
        links.push(node.url);
      if (node.type === "code") code.push(node.value);
    });
    const title = tree.children.find(
      (node) => node.type === "heading" && node.depth === 1,
    );
    if (!title) throw new Error(`Missing H1 title: ${file}`);
    return {
      file,
      title: toString(title),
      url: documentUrl(file),
      slugs: documentSlug(file),
      markdown,
      headings,
      links,
      code,
    };
  });
}

/** Resolve only Markdown URL nodes; code, external URLs and bare fragments are untouched. */
export function resolveContentUrl(url: string, sourceFile: string): string {
  if (/^(?:[a-z][a-z\d+.-]*:|\/\/|#|\/)/i.test(url)) return url;
  const match = /^([^?#]*)(.*)$/.exec(url)!;
  const target = path.posix.normalize(
    path.posix.join(path.posix.dirname(sourceFile), decodeURI(match[1])),
  );
  const suffix = match[2];
  if (target.startsWith("../") || !existsSync(path.join(root, target))) {
    throw new Error(`Missing local target in ${sourceFile}: ${url}`);
  }
  if (
    (target.startsWith("docs/") && target.endsWith(".md")) ||
    target === "CONTRIBUTING.md"
  ) {
    return `${documentUrl(target)}${suffix}`;
  }
  if (target.startsWith("docs/assets/"))
    return `${publicUrl(`/generated/${target}`)}${suffix}`;
  return `${repository}/blob/main/${target}${suffix}`;
}
