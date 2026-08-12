import { createHash } from "node:crypto";
import { lstat, readFile, readlink, readdir, writeFile } from "node:fs/promises";
import path from "node:path";

const [rootArgument, version, platform, signatureKind, workspaceArgument] = process.argv.slice(2);
if (!rootArgument || !version || !platform || !signatureKind || !workspaceArgument) {
  throw new Error("usage: release-manifest <root> <version> <platform> <signature> <workspace>");
}

const root = path.resolve(rootArgument);
const workspace = Buffer.from(path.resolve(workspaceArgument));
const forbiddenSegments = new Set([".git", "fixtures", "target", "tests"]);
const required = [
  /^bin\/lili(?:\.exe)?$/,
  /^bin\/lili-hook(?:\.exe)?$/,
  /^bundles\//,
  /^docs\/configuration\.md$/,
  /^docs\/security-and-operations\.md$/,
  /^examples\/actions\.toml$/,
  /^pets\/lili\/pet\.json$/,
  /^pets\/lili\/spritesheet\.webp$/,
  /^web\/index\.html$/,
  /^LICENSE$/,
  /^NOTICE$/,
  /^THIRD_PARTY_NOTICES\.html$/,
];

async function collect(directory, prefix = "") {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (/(?:fixture|acceptance)/i.test(entry.name)) {
      throw new Error(`private verification artifact in release: ${relative}`);
    }
    if (relative === "manifest.json") continue;
    if (relative.split("/").some((segment) => forbiddenSegments.has(segment))) {
      throw new Error(`forbidden release path: ${relative}`);
    }
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collect(absolute, relative)));
      continue;
    }
    const metadata = await lstat(absolute);
    if (metadata.isSymbolicLink()) {
      const target = await readlink(absolute);
      if (path.isAbsolute(target)) throw new Error(`absolute release symlink: ${relative}`);
      files.push({
        path: relative,
        type: "symlink",
        size: Buffer.byteLength(target),
        sha256: createHash("sha256").update(`symlink:${target}`).digest("hex"),
      });
      continue;
    }
    if (!metadata.isFile()) throw new Error(`unsupported release entry: ${relative}`);
    const contents = await readFile(absolute);
    if (contents.includes(workspace)) throw new Error(`development path leaked into ${relative}`);
    files.push({
      path: relative,
      type: "file",
      size: metadata.size,
      sha256: createHash("sha256").update(contents).digest("hex"),
    });
  }
  return files;
}

const files = await collect(root);
for (const pattern of required) {
  if (!files.some((file) => pattern.test(file.path))) {
    throw new Error(`required release content is missing: ${pattern}`);
  }
}

await writeFile(
  path.join(root, "manifest.json"),
  `${JSON.stringify(
    {
      schemaVersion: 1,
      product: "Lili",
      version,
      platform,
      signatureKind,
      files,
    },
    null,
    2,
  )}\n`,
);
