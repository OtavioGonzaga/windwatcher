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

export default defineConfig({ mdxOptions: { remarkPlugins: [remarkDocs] } });
