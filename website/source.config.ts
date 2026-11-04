import { defineDocs, defineConfig } from "fumadocs-mdx/config";
import { z } from "zod";
import { remarkDocs } from "./lib/remark-docs";

export const docs = defineDocs({
  dir: "../docs",
  docs: {
    schema: z.object({
      title: z.string().optional(),
      description: z.string().optional(),
    }),
  },
});

export const contributing = defineDocs({
  dir: "..",
  docs: {
    files: ["CONTRIBUTING.md"],
    schema: z.object({
      title: z.string().optional(),
      description: z.string().optional(),
    }),
  },
  meta: { files: [] },
});

export default defineConfig({ mdxOptions: { remarkPlugins: [remarkDocs] } });
