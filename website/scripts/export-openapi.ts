import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import { root, website } from "../lib/content";
import { validateContract } from "../lib/contract";

const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "--locked",
    "-p",
    "windwatcher-server",
    "--example",
    "export_openapi",
  ],
  {
    cwd: root,
    env: { ...process.env, CARGO_BUILD_JOBS: "4" },
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  },
);
if (result.error) throw result.error;
if (result.status !== 0)
  throw new Error(`OpenAPI export failed:\n${result.stderr}`);
const spec = JSON.parse(result.stdout);
const manifest = readFileSync(
  path.join(root, "crates/windwatcher-server/Cargo.toml"),
  "utf8",
);
const version = /^version = "([^"]+)"$/m.exec(manifest)?.[1];
if (!version) throw new Error("Missing server package version");
const entries = validateContract(spec, version);
for (const file of ["generated/openapi-v1.json", "public/openapi/v1.json"]) {
  const target = path.join(website, file);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(`${target}.tmp`, result.stdout);
  renameSync(`${target}.tmp`, target);
}
console.log(
  `Exported OpenAPI ${spec.openapi}, API ${version}: ${entries.length} operations.`,
);
