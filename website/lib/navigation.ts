import type { Root, Node, Item } from "fumadocs-core/page-tree";

interface Page {
  url: string;
  slugs: string[];
  data: { title?: string };
}

/** Group presentation only: source files and public routes never move. */
export function documentationTree(pages: Page[]): Root {
  const remaining = new Map(pages.map((page) => [page.slugs.join("/"), page]));
  function take(slug: string): Item[] {
    const page = remaining.get(slug);
    if (!page) return [];
    remaining.delete(slug);
    return [{ type: "page", name: page.data.title ?? slug, url: page.url }];
  }
  const children: Node[] = [];
  function section(name: string, slugs: string[]) {
    const items = slugs.flatMap(take);
    if (items.length) children.push({ type: "separator", name }, ...items);
  }
  section("Overview", ["", "architecture"]);
  section("Concepts", [
    "identity",
    "authentication",
    "sessions",
    "conversations",
    "messages",
    "pagination",
    "domain-events",
  ]);
  section("Windwatcher Server", [
    "installation",
    "server-configuration",
    "cli",
    "operations",
    "container",
    "persistence",
    "jobs",
    "security",
  ]);
  section("API Reference", ["http-api"]);
  for (const tag of [
    "Instance",
    "Sessions",
    "Users",
    "Conversations",
    "Messages",
    "Realtime",
  ]) {
    const entries = [...remaining.keys()]
      .filter((slug) => slug.startsWith(`api/${tag.toLowerCase()}/`))
      .sort();
    if (entries.length)
      children.push({
        type: "folder",
        name: tag,
        children: entries.flatMap(take),
      });
  }
  children.push(...take("realtime"));
  section("Development", ["releasing", "contributing"]);
  const index = take("adr")[0];
  const records = [...remaining.keys()]
    .filter((slug) => slug.startsWith("adr/"))
    .sort();
  children.push({
    type: "folder",
    name: "Architectural Decisions",
    index,
    children: records.flatMap(take),
  });
  section("More documentation", [...remaining.keys()].sort());
  return { name: "Windwatcher Documentation", children };
}
