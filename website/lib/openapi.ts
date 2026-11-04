import { createOpenAPI } from "fumadocs-openapi/server";
import { operations } from "./contract";
import spec from "../generated/openapi-v1.json";
import type { Document } from "fumadocs-openapi";

export const contract = spec as Document;
export const contractOperations = operations(contract);
export const openapi = createOpenAPI({ input: { windwatcher: contract } });

export const apiSource = await openapi.staticSource({
  baseDir: "api",
  per: "operation",
  name(output) {
    if (output.type !== "operation")
      throw new Error("Unexpected non-operation page");
    const entry = contractOperations.find(
      (op) => op.path === output.item.path && op.method === output.item.method,
    );
    if (!entry)
      throw new Error(`Unmapped OpenAPI operation: ${output.item.path}`);
    return `${entry.tag.toLowerCase()}/${entry.id}`;
  },
});

// Validate the integration's actual output, not another hand-maintained operation list.
const rendered = apiSource.files
  .filter((file) => file.type === "page")
  .flatMap((file) => file.data.getOpenAPIPageProps().operations ?? []);
const keys = rendered.map((op) => `${op.method} ${op.path}`);
if (
  new Set(keys).size !== keys.length ||
  keys.length !== contractOperations.length ||
  contractOperations.some((op) => !keys.includes(`${op.method} ${op.path}`))
) {
  throw new Error(
    "Fumadocs page collection does not cover OpenAPI operations exactly once",
  );
}
