import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { source } from "@/lib/source";
import { repository } from "@/lib/site";
import type { ReactNode } from "react";
import { documentationTree } from "@/lib/navigation";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout
      tree={documentationTree(source.getPages())}
      nav={{ title: "Windwatcher", url: "/docs/" }}
      githubUrl={repository}
      links={[
        {
          text: "Rust API",
          url: "https://docs.rs/windwatcher",
          external: true,
        },
      ]}
    >
      {children}
    </DocsLayout>
  );
}
