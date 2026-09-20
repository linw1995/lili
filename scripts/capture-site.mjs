import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rename, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { chromium } from "playwright";
import { serveSite } from "./serve-site.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const options = process.argv.slice(2);
if (options.some(option => !["--release", "--no-build"].includes(option)) || (options.includes("--release") && options.includes("--no-build"))) {
  throw new Error("Usage: node scripts/capture-site.mjs [--release | --no-build]");
}
execFileSync("magick", ["-version"], { stdio: "ignore" });
if (!options.includes("--no-build")) {
  execFileSync(process.execPath, [path.join(root, "scripts/build-site.mjs"), ...options], { stdio: "inherit" });
}

const output = path.join(root, "dist/site/media");
await mkdir(output, { recursive: true });
const temporary = await mkdtemp(path.join(tmpdir(), "lili-site-media-"));
const { server, url } = await serveSite(0, process.env.LILI_SITE_BASE_PATH ?? "/");
const frameDurationMs = 100;
const framesPerScene = 15;
const scenes = ["Review", "Idle", "Click", "Running", "Attention", "Failed"];
const durationMs = scenes.length * framesPerScene * frameDurationMs;
let browser;
try {
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1, reducedMotion: "no-preference", colorScheme: "light" });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("requestfailed", request => errors.push(`${request.url()}: ${request.failure()?.errorText}`));
  page.on("response", response => { if (!response.ok()) errors.push(`${response.status()}: ${response.url()}`); });
  page.on("request", request => {
    if (new URL(request.url()).pathname.includes("/api/")) errors.push(`Unexpected runtime request: ${request.url()}`);
  });
  // Step the actual Rust animation controller, independent of CI screenshot speed.
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.goto(url);
  const preview = page.locator(".pet-showcase");
  await preview.locator("#appearance-pet").waitFor();
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all([...document.images].map(image => image.decode()));
  });
  await page.locator(".demo").scrollIntoViewIfNeeded();
  await page.clock.pauseAt(new Date("2026-01-01T01:00:00Z"));
  const frames = [];
  for (const scene of scenes) {
    await preview.getByRole("button", { name: scene, exact: true }).click();
    await page.mouse.move(0, 0);
    const atlasFrames = new Set();
    for (let frame = 0; frame < framesPerScene; frame++) {
      const file = path.join(temporary, `frame-${String(frames.length).padStart(3, "0")}.png`);
      await page.locator(".demo").screenshot({ path: file, animations: "disabled" });
      atlasFrames.add(await preview.locator(".appearance-pet-atlas").evaluate(image => `${image.dataset.frameRow}:${image.dataset.frameColumn}`));
      frames.push(file);
      await page.clock.runFor(frameDurationMs);
    }
    assert.ok(atlasFrames.size > 1, `${scene} animation did not advance`);
  }
  if (errors.length) throw new Error(errors.join("\n"));
  await browser.close();
  browser = undefined;

  // One shared palette keeps unchanged UI pixels stable between frames.
  const palette = path.join(temporary, "palette.png");
  const limits = ["-limit", "memory", "256MiB", "-limit", "map", "512MiB"];
  execFileSync("magick", [...limits, ...frames, "-resize", "25%", "-append", "-colors", "256", "-unique-colors", palette]);
  const gif = path.join(temporary, "lili-demo.gif");
  execFileSync("magick", [...limits, "-delay", String(frameDurationMs / 10), "-loop", "0", ...frames, "-dither", "None", "-remap", palette, "-layers", "Optimize", gif]);

  const delays = execFileSync("magick", ["identify", "-format", "%T\n", gif], { encoding: "utf8" }).trim().split(/\s+/).map(Number);
  assert.ok(delays.length > 1, "Expected an animated GIF");
  assert.equal(delays.reduce((sum, delay) => sum + delay, 0) * 10, durationMs);
  const bytes = await readFile(gif);
  assert.equal(bytes.subarray(0, 6).toString(), "GIF89a");
  assert.ok(bytes.includes(Buffer.from("NETSCAPE2.0")), "Expected a looping GIF");
  assert.ok((await stat(gif)).size < 5 * 1024 * 1024, "README GIF exceeds 5 MiB");

  await copyFile(gif, path.join(output, ".lili-demo.gif.tmp"));
  await rename(path.join(output, ".lili-demo.gif.tmp"), path.join(output, "lili-demo.gif"));
  await copyFile(frames[0], path.join(output, "lili-demo.png"));
  const response = await fetch(new URL("media/lili-demo.gif", url));
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type"), /^image\/gif/);
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), bytes);
  console.log(`Exported media/lili-demo.gif: ${delays.length} frames, ${durationMs / 1000} seconds, ${Math.ceil(bytes.length / 1024)} KiB. PNG poster included.`);
} finally {
  await browser?.close();
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  await rm(temporary, { recursive: true, force: true });
}
