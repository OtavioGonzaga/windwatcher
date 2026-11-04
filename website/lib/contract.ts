import type { Document, OperationObject } from "fumadocs-openapi";

export const methods = [
  "get",
  "put",
  "post",
  "delete",
  "options",
  "head",
  "patch",
  "trace",
] as const;

export function operations(spec: Document) {
  const ids = new Set<string>();
  return Object.entries(spec.paths ?? {}).flatMap(([path, item]) =>
    methods.flatMap((method) => {
      const operation = item?.[method] as OperationObject | undefined;
      if (!operation) return [];
      const id = operation.operationId;
      const tag = operation.tags?.[0] ?? "Other";
      if (!id || !/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(id) || ids.has(id)) {
        throw new Error(`Missing, invalid or duplicate operationId: ${id}`);
      }
      ids.add(id);
      if (!/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(tag))
        throw new Error(`Invalid operation tag: ${tag}`);
      return [
        {
          path,
          method,
          id,
          tag,
          operation,
          url: `/docs/api/${tag.toLowerCase()}/${id}/`,
        },
      ];
    }),
  );
}

export function validateContract(spec: Document, version: string) {
  if ((spec.openapi as string) !== "3.1.0")
    throw new Error(`Expected OpenAPI 3.1.0, got ${spec.openapi}`);
  if (spec.info?.version !== version)
    throw new Error("OpenAPI version differs from the server package");
  if (
    !spec.components?.securitySchemes?.bearerAuth ||
    !spec.components.securitySchemes.realtimeTicket
  ) {
    throw new Error("Missing documented security schemes");
  }
  const visit = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    if ("$ref" in value && typeof value.$ref === "string") {
      if (!value.$ref.startsWith("#/"))
        throw new Error(`Nonlocal schema reference: ${value.$ref}`);
      let target: unknown = spec;
      for (const part of value.$ref.slice(2).split("/")) {
        const key = decodeURIComponent(part)
          .replace(/~1/g, "/")
          .replace(/~0/g, "~");
        target =
          target && typeof target === "object"
            ? (target as Record<string, unknown>)[key]
            : undefined;
      }
      if (target === undefined)
        throw new Error(`Unresolved schema reference: ${value.$ref}`);
    }
    for (const child of Object.values(value)) visit(child);
  };
  visit(spec);
  const result = operations(spec);
  if (result.length === 0) throw new Error("OpenAPI has no operations");
  return result;
}
