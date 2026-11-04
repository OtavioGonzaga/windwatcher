import { notFound } from "next/navigation";
import { source } from "@/lib/source";
import { docsLlms } from "@/lib/llms";

export const dynamic = "force-static";
export const dynamicParams = false;

export function generateStaticParams() {
  return source
    .getPages()
    .map((page) => ({ slug: [...page.slugs, "content.md"] }));
}

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ slug: string[] }> },
) {
  const { slug } = await params;
  if (slug.at(-1) !== "content.md") notFound();
  const page = source.getPage(slug.slice(0, -1));
  if (!page) notFound();
  return new Response(await docsLlms.page(page), {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}
