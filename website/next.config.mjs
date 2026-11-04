import { createMDX } from "fumadocs-mdx/next";
import { basePath } from "./lib/site.ts";

const withMDX = createMDX();

export default withMDX({
  output: "export",
  basePath,
  trailingSlash: true,
  images: { unoptimized: true },
  experimental: { cpus: 2 },
});
