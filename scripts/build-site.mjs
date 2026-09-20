import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, readFile, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = path.join(root, "dist/site");
const compiled = path.join(root, "target/site-build");
const options = process.argv.slice(2);
if (options.some(option => option !== "--release")) throw new Error("Usage: node scripts/build-site.mjs [--release]");

execFileSync("trunk", ["build", "--locked", "--config", "Trunk.site.toml", ...options], { cwd: root, stdio: "inherit" });

async function fingerprint(directory) {
  const hash = createHash("sha256");
  async function visit(relative) {
    for (const entry of (await readdir(path.join(directory, relative), { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const name = path.join(relative, entry.name);
      if (entry.isDirectory()) await visit(name);
      else {
        hash.update(name.split(path.sep).join("/"));
        hash.update("\0");
        hash.update(await readFile(path.join(directory, name)));
        hash.update("\0");
      }
    }
  }
  await visit("");
  return hash.digest("hex").slice(0, 20);
}

// Version the entire Leptos application graph so JS, WASM, snippets, and CSS stay paired.
const version = await fingerprint(compiled);
const assets = path.join(output, "assets");
const destination = path.join(assets, version);
await mkdir(assets, { recursive: true });
if (!(await stat(destination).catch(() => null))) {
  const staging = path.join(assets, `.staging-${process.pid}`);
  try {
    await cp(compiled, staging, { recursive: true });
    await rename(staging, destination);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

// The boot document stays at the site root; module-relative imports stay inside the version.
const shell = await readFile(path.join(compiled, "index.html"), "utf8");
if (!shell.includes('data-site-assets="./"')) throw new Error("The website asset base is missing from the Trunk shell.");
const html = shell.replace(/(["'])\.\//g, (_, quote) => `${quote}./assets/${version}/`);
await writeFile(path.join(output, ".index.html.tmp"), html);
await rename(path.join(output, ".index.html.tmp"), path.join(output, "index.html"));

// Preserve old direct-demo bookmarks after removing the separate iframe application.
await mkdir(path.join(output, "preview"), { recursive: true });
await writeFile(path.join(output, "preview/index.html"), '<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=../#demo"><title>Lili demo</title><a href="../#demo">Open the Lili demo</a></html>\n');
console.log(`Built Leptos website: ${output} (version ${version})`);
