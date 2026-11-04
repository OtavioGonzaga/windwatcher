import { cpSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import {
  readDocuments,
  website,
  root,
  resolveContentUrl,
} from "../lib/content";

const documents = readDocuments();
const urls = new Set<string>();
for (const doc of documents) {
  if (urls.has(doc.url)) throw new Error(`Duplicate page URL: ${doc.url}`);
  urls.add(doc.url);
  for (const link of doc.links) resolveContentUrl(link, doc.file);
}
mkdirSync(path.join(website, "generated"), { recursive: true });
writeFileSync(
  path.join(website, "generated/documents.json"),
  JSON.stringify(documents, null, 2),
);
mkdirSync(path.join(website, "public/generated/docs"), { recursive: true });
cpSync(
  path.join(root, "docs/assets"),
  path.join(website, "public/generated/docs/assets"),
  { recursive: true },
);
console.log(
  `Prepared ${documents.length} Markdown pages and documentation assets.`,
);
