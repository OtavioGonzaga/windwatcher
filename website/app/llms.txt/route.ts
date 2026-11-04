import { docsLlms, agentMarkdown } from "@/lib/llms";
import { publicUrl, siteOrigin } from "@/lib/site";

export const dynamic = "force-static";

export async function GET() {
  const index = await docsLlms.index();
  // Fumadocs emits app-relative index links; agents need the deployment prefix.
  const links = await agentMarkdown(index);
  return new Response(
    `${links}\n\n- [Complete Markdown](${siteOrigin}${publicUrl("/llms-full.txt")})\n- [OpenAPI 3.1 JSON](${siteOrigin}${publicUrl("/openapi/v1.json")})\n`,
    { headers: { "Content-Type": "text/plain; charset=utf-8" } },
  );
}
