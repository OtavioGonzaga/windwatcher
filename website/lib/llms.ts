import { llms } from "fumadocs-core/source";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkStringify from "remark-stringify";
import remarkGfm from "remark-gfm";
import { visit } from "unist-util-visit";
import type { Root } from "mdast";
import { source } from "./source";
import { contractOperations } from "./openapi";
import { publicUrl, siteOrigin } from "./site";
import path from "node:path";
import { root, resolveContentUrl } from "./content";

export function markdownUrl(slugs: string[]) {
  return publicUrl(
    `/markdown/docs/${slugs.length ? `${slugs.join("/")}/` : ""}content.md`,
  );
}

function absoluteLinks(sourceFile?: string) {
  return (tree: Root) =>
    visit(tree, (node) => {
      if (
        node.type !== "link" &&
        node.type !== "image" &&
        node.type !== "definition"
      )
        return;
      if (sourceFile) node.url = resolveContentUrl(node.url, sourceFile);
      if (node.url === "/docs" || node.url.startsWith("/docs/")) {
        const url = new URL(node.url, siteOrigin);
        url.pathname = publicUrl(`${url.pathname.replace(/\/$/, "")}/`);
        node.url = url.href;
      } else if (node.url.startsWith("/"))
        node.url = `${siteOrigin}${node.url}`;
    });
}

export async function agentMarkdown(markdown: string, sourceFile?: string) {
  return String(
    await unified()
      .use(remarkParse)
      .use(remarkGfm)
      .use(absoluteLinks, sourceFile)
      .use(remarkStringify)
      .process(markdown),
  );
}

export const docsLlms = llms(source, {
  renderPage: async (page) => {
    const url = `${siteOrigin}${publicUrl(`${page.url.replace(/\/$/, "")}/`)}`;
    if (page.type === "openapi") {
      const props = page.data.getOpenAPIPageProps();
      const item = props.operations?.[0];
      const entry = contractOperations.find(
        (op) => op.path === item?.path && op.method === item?.method,
      );
      if (!entry) throw new Error(`Missing operation: ${page.url}`);
      return `# ${page.data.title}\n\nSource: ${url}\n\n${entry.method.toUpperCase()} ${entry.path}\n\nOperation ID: ${entry.id}\n\n\`\`\`json\n${JSON.stringify(entry.operation, null, 2)}\n\`\`\`\n\nSchema references and security schemes: ${siteOrigin}${publicUrl("/openapi/v1.json")}\n`;
    }
    return `Source: ${url}\n\n${await agentMarkdown(await page.data.getText("raw"), path.relative(root, page.data.info.fullPath))}`;
  },
});
