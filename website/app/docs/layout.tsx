import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { source } from "@/lib/source";
import { repository } from "@/lib/site";
import type { ReactNode } from "react";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout
      tree={source.getPageTree()}
      nav={{ title: "Windwatcher", url: "/docs/" }}
      githubUrl={repository}
    >
      {children}
    </DocsLayout>
  );
}
