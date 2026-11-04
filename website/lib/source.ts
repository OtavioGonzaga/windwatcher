import { docs, contributing } from "../.source/server";
import { loader, type Source } from "fumadocs-core/source";
import { readDocuments } from "./content";

const documents = readDocuments();
const byFile = new Map(documents.map((doc) => [doc.file, doc]));
const files = [...docs.docs, ...contributing.docs].map((data) => {
  const key =
    data.info.path === "CONTRIBUTING.md"
      ? data.info.path
      : `docs/${data.info.path}`;
  const document = byFile.get(key);
  if (!document) throw new Error(`Unmapped source page: ${key}`);
  return {
    type: "page" as const,
    path: data.info.path,
    slugs: document.slugs,
    data: { ...data, title: document.title },
  };
});

const markdown: Source<{
  pageData: (typeof files)[number]["data"];
  metaData: {};
}> = { files };

export const source = loader({
  baseUrl: "/docs",
  source: markdown,
});
