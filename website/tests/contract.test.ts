import assert from "node:assert/strict";
import { test } from "node:test";
import type { Document } from "fumadocs-openapi";
import { operations, validateContract } from "../lib/contract";

function fixture() {
  return {
    openapi: "3.1.0",
    info: { title: "Test", version: "1.2.3" },
    components: {
      securitySchemes: {
        bearerAuth: { type: "http", scheme: "bearer" },
        realtimeTicket: { type: "apiKey", in: "query", name: "ticket" },
      },
      schemas: { Test: { type: "string" } },
    },
    paths: {
      "/v1/example": {
        get: {
          operationId: "getExample",
          tags: ["Users"],
          responses: {
            "200": {
              description: "Success",
              content: {
                "application/json": {
                  schema: { $ref: "#/components/schemas/Test" },
                },
              },
            },
          },
        },
      },
    },
  } as unknown as Document;
}

test("coverage derives from method/path pairs and preserves stable operation IDs", () => {
  const entries = validateContract(fixture(), "1.2.3");
  assert.equal(entries.length, 1);
  assert.equal(entries[0].url, "/docs/api/users/getExample/");
  assert.equal(entries[0].path, "/v1/example");
  assert.equal(entries[0].method, "get");
});

test("rejects incompatible version, missing security, invalid references and duplicate IDs", () => {
  assert.throws(
    () => validateContract(fixture(), "different"),
    /version differs/,
  );
  const spec = fixture();
  spec.components!.securitySchemes = {};
  assert.throws(() => validateContract(spec, "1.2.3"), /security schemes/);
  const broken = fixture();
  broken.components!.schemas = {};
  assert.throws(() => validateContract(broken, "1.2.3"), /Unresolved schema/);
  const duplicate = fixture();
  duplicate.paths!["/v1/duplicate"] = duplicate.paths!["/v1/example"];
  assert.throws(() => operations(duplicate), /duplicate operationId/);
});
