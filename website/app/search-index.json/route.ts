import { flexsearchFromSource } from "fumadocs-core/search/flexsearch";
import { source } from "@/lib/source";
import { contractOperations } from "@/lib/openapi";
import { operationSearchText } from "@/lib/contract";

export const dynamic = "force-static";

export const { staticGET: GET } = flexsearchFromSource(source, {
  // Prefix matching avoids quadratic substring indexes for long Rust identifiers.
  document: { tokenize: "forward" },
  buildIndex(page) {
    const structuredData = page.data.structuredData;
    const entry =
      page.type === "openapi"
        ? contractOperations.find(
            (entry) => entry.url === `${page.url.replace(/\/$/, "")}/`,
          )
        : undefined;
    return {
      id: page.url,
      url: page.url,
      title: entry
        ? `${entry.method.toUpperCase()} ${entry.path} · ${entry.id}`
        : (page.data.title ?? page.url),
      description: page.data.description,
      structuredData: entry
        ? {
            ...structuredData,
            contents: [
              ...structuredData.contents,
              {
                heading: undefined,
                content: operationSearchText(entry.operation),
              },
            ],
          }
        : structuredData,
    };
  },
});
