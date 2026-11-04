import { createMDX } from "fumadocs-mdx/next";
import { basePath } from "./lib/site.ts";
import { execFileSync } from "node:child_process";

const withMDX = createMDX();

export default withMDX({
  output: "export",
  generateBuildId: async () =>
    execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  basePath,
  trailingSlash: true,
  images: { unoptimized: true },
  experimental: { cpus: 2 },
});
