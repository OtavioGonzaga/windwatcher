import { notFound } from "next/navigation";
import { DocsPage, DocsBody } from "fumadocs-ui/layouts/docs/page";
import { source } from "@/lib/source";
import { getMDXComponents } from "@/mdx-components";
import { OpenAPIPage } from "@/components/api-page";
import { ResponseHeaders } from "@/components/response-headers";
import { contractOperations } from "@/lib/openapi";
import { publicUrl } from "@/lib/site";
import Link from "next/link";

export const dynamicParams = false;

export function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug?: string[] }>;
}) {
  const page = source.getPage((await params).slug);
  if (!page) notFound();
  return { title: page.data.title, description: page.data.description };
}

export default async function Page({
  params,
}: {
  params: Promise<{ slug?: string[] }>;
}) {
  const { slug } = await params;
  const page = source.getPage(slug);
  if (!page) notFound();
  if (page.type === "openapi") {
    const props = page.data.getOpenAPIPageProps();
    const item = props.operations?.[0];
    const entry = contractOperations.find(
      (op) => op.path === item?.path && op.method === item?.method,
    );
    if (!entry) throw new Error(`Missing operation for ${page.url}`);
    return (
      <DocsPage
        full
        toc={[
          ...page.data.toc,
          { title: "Response headers", url: "#response-headers", depth: 2 },
        ]}
      >
        <DocsBody>
          <h1>{page.data.title}</h1>
          <p>
            HTTP API v1 · <code>{entry.id}</code> ·{" "}
            <Link href="/docs/http-api/">HTTP API guide</Link> ·{" "}
            <a href={publicUrl("/openapi/v1.json")}>OpenAPI JSON</a>
          </p>
          <p>
            This is a read-only reference. Requests are not sent from this
            website.
          </p>
          <OpenAPIPage {...props} />
          <ResponseHeaders operation={entry.operation} />
        </DocsBody>
      </DocsPage>
    );
  }
  const MDX = page.data.body;
  return (
    <DocsPage toc={page.data.toc}>
      <DocsBody>
        {page.slugs.join("/") === "http-api" && (
          <p>
            <Link href="/docs/api/sessions/createSession/">
              Generated endpoint reference
            </Link>{" "}
            · <a href={publicUrl("/openapi/v1.json")}>Download OpenAPI JSON</a>
          </p>
        )}
        <MDX components={getMDXComponents()} />
      </DocsBody>
    </DocsPage>
  );
}
