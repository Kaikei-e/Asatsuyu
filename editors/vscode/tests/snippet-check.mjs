// Verify all expected snippet prefixes exist in asatsuyu.code-snippets.

import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const snippetsPath = join(__dirname, "..", "snippets", "asatsuyu.code-snippets");

const EXPECTED_PREFIXES = [
  "fn",
  "pfn",
  "afn",
  "pafn",
  "main",
  "amain",
  "if",
  "match",
  "let",
  "letm",
  "type",
  "ptype",
  "fpi",
  "fpia",
  "try",
];

const snippets = JSON.parse(readFileSync(snippetsPath, "utf8"));
const actual = new Set(Object.values(snippets).map((s) => s.prefix));

let failed = false;
for (const prefix of EXPECTED_PREFIXES) {
  if (!actual.has(prefix)) {
    console.error(`FAIL: missing snippet prefix "${prefix}"`);
    failed = true;
  }
}

if (failed) {
  console.error(
    `\nExpected ${EXPECTED_PREFIXES.length} snippets, found: ${[...actual].join(", ")}`,
  );
  process.exit(1);
}

console.log(`OK: all ${EXPECTED_PREFIXES.length} snippet prefixes present`);
