import assert from "node:assert/strict";
import { chromium, webkit } from "playwright";
import { serveSite } from "./serve-site.mjs";

const { server, url } = await serveSite(0, process.env.LILI_SITE_BASE_PATH ?? "/");
try {
  for (const engine of [chromium, webkit]) {
    const browser = await engine.launch();
    try {
      const page = await browser.newPage({ reducedMotion: "no-preference", colorScheme: "light" });
      const errors = [];
      const wasmUrls = new Set();
      page.on("pageerror", error => errors.push(error.message));
      page.on("response", response => {
        if (!response.ok() && response.status() !== 302) errors.push(`${response.status()}: ${response.url()}`);
        if (new URL(response.url()).pathname.endsWith(".wasm")) {
          wasmUrls.add(response.url());
          if (!/^application\/wasm/.test(response.headers()["content-type"])) errors.push("Invalid WASM MIME type");
        }
      });
      page.on("request", request => {
        const target = new URL(request.url());
        if (target.origin === new URL(url).origin && !target.pathname.startsWith(new URL(url).pathname)) {
          errors.push(`Request escaped the site base path: ${target.pathname}`);
        }
        if (target.pathname.includes("/api/")) errors.push(`Unexpected desktop request: ${target.pathname}`);
      });
      await page.route(new URL("lili-ui.js", url).href, route => route.fulfill({ contentType: "text/javascript", body: "throw new Error('Stale loader was used');" }));
      await page.goto(url);
      await page.locator("#lili-website").waitFor();
      assert.match(await page.locator("html").getAttribute("data-site-assets"), /^\.\/assets\/[a-f0-9]{20}\/$/);
      assert.equal(page.frames().length, 1);
      assert.equal(await page.locator("iframe").count(), 0);
      assert.equal(await page.locator("main").count(), 1);
      assert.equal(await page.locator(".extensions dt").count(), 3);
      const preview = page.locator(".pet-showcase");
      const atlas = preview.locator(".appearance-pet-atlas");
      await atlas.evaluate(image => image.decode());
      assert.deepEqual(await atlas.evaluate(image => [image.naturalWidth, image.naturalHeight]), [1536, 2288]);
      const initialFrame = await atlas.getAttribute("data-frame-column");
      await page.waitForFunction(initial => document.querySelector(".appearance-pet-atlas").dataset.frameColumn !== initial, initialFrame);
      assert.equal(await page.locator("#site-status").count(), 0);
      assert.equal(wasmUrls.size, 1);

      for (const [label, scene, hasNotification] of [["Idle", "idle", false], ["Failed", "failed", true], ["Review", "review", true]]) {
        await preview.getByRole("button", { name: label, exact: true }).click();
        await preview.locator(`#appearance-scene[data-scene="${scene}"]`).waitFor();
        assert.equal(await preview.locator(".notification-card-preview").count(), Number(hasNotification));
      }
      for (const width of [1440, 1024, 768, 390, 320]) {
        await page.setViewportSize({ width, height: 900 });
        const layout = await page.locator(".demo").evaluate(root => {
          const container = root.getBoundingClientRect();
          return {
            horizontal: document.documentElement.scrollWidth > innerWidth + 1,
            nestedScroll: root.scrollHeight > root.clientHeight + 1,
            clipped: [...root.querySelectorAll("button, .appearance-pet, .showcase-note")].some(element => {
              const bounds = element.getBoundingClientRect();
              return bounds.top < container.top - 1 || bounds.bottom > container.bottom + 1 || bounds.left < container.left - 1 || bounds.right > container.right + 1;
            }),
          };
        });
        assert.deepEqual(layout, { horizontal: false, nestedScroll: false, clipped: false }, `Layout at ${width}px`);
      }
      const cardColor = await preview.locator(".notification-card-preview").evaluate(card => getComputedStyle(card).color);
      await page.emulateMedia({ colorScheme: "dark" });
      assert.equal(await preview.evaluate(root => getComputedStyle(root).backgroundColor), "rgb(255, 255, 255)");
      assert.equal(await preview.locator(".notification-card-preview").evaluate(card => getComputedStyle(card).color), cardColor);
      assert.deepEqual(errors, []);
      await page.close();

      // A missing import table must show recovery UI even before Rust can mount the page.
      const broken = await browser.newPage();
      const importErrors = [];
      broken.on("pageerror", error => importErrors.push(error.message));
      await broken.addInitScript(() => {
        const instantiate = WebAssembly.instantiateStreaming;
        WebAssembly.instantiateStreaming = function(source, imports, ...options) {
          const keys = Object.keys(imports);
          delete imports[keys.find(key => key.includes("/snippets/")) ?? keys[0]];
          return instantiate.call(WebAssembly, source, imports, ...options);
        };
      });
      await broken.goto(url);
      await broken.getByRole("button", { name: "Reload page" }).waitFor({ state: "visible" });
      assert.match(await broken.getByRole("status").innerText(), /could not load/);
      assert.ok(importErrors.some(message => /import|module/i.test(message)));
      console.log(`${engine.name()}: one Leptos page, shared interactions, responsive layout, assets, and recovery passed.`);
      await broken.close();
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
}
