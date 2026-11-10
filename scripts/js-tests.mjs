// Runs every tests/js/*.test.js with node's built in test runner.
// Works the same in sh, PowerShell and cmd, and on Node 18 and newer
// (globs in `node --test` need Node 21+).
import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "tests", "js");
const files = readdirSync(dir).filter((f) => f.endsWith(".test.js")).sort().map((f) => join(dir, f));
if (files.length === 0) {
  console.error("No tests found in " + dir);
  process.exit(1);
}
const r = spawnSync(process.execPath, ["--test", ...files], { stdio: "inherit" });
process.exit(r.status ?? 1);
