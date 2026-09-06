const { expect, test } = require("@playwright/test");

let diagnostics = [];

test.beforeEach(async ({ page }) => {
  diagnostics = [];
  page.on("console", (message) => {
    if (["error", "warning"].includes(message.type())) {
      diagnostics.push(`console.${message.type()}: ${message.text()}`);
    }
  });
  page.on("pageerror", (error) => diagnostics.push(`pageerror: ${error.message}`));
});

test.afterEach(async () => {
  expect(diagnostics, "browser diagnostics must stay empty").toEqual([]);
});

async function openAppearance(page) {
  await page.goto("/appearance");
  await page.waitForFunction(() => window.__LILI_HYDRATED__ === true);
  await expect(page.locator("#lili-appearance")).toHaveAttribute(
    "data-hydrated",
    "true",
  );
  await expect(page.locator("#lili-appearance")).toHaveAttribute(
    "data-ssr-marker",
    "appearance-ready",
  );
}

async function expectNoHorizontalClipping(page) {
  const result = await page.evaluate(() => {
    const viewportWidth = document.documentElement.clientWidth;
    const bodyWidth = document.body.scrollWidth;
    const documentWidth = document.documentElement.scrollWidth;
    const controls = [
      ...document.querySelectorAll(
        "#lili-appearance button, #lili-appearance [role=option]",
      ),
    ];
    const clipped = controls
      .map((element) => ({
        tag: element.tagName,
        label: element.textContent.trim(),
        left: element.getBoundingClientRect().left,
        right: element.getBoundingClientRect().right,
        width: element.getBoundingClientRect().width,
      }))
      .filter(
        ({ left, right, width }) =>
          width <= 0 || left < -1 || right > viewportWidth + 1,
      );
    return { viewportWidth, bodyWidth, documentWidth, clipped };
  });
  expect(result.bodyWidth).toBeLessThanOrEqual(result.viewportWidth);
  expect(result.documentWidth).toBeLessThanOrEqual(result.viewportWidth);
  expect(result.clipped).toEqual([]);
}

test("Appearance keeps Pet navigation and scene controls keyboard reachable", async ({
  page,
}) => {
  await openAppearance(page);

  await expect(page.locator(".appearance-window-frame")).toBeVisible();
  await expect(page.locator(".appearance-topbar")).toHaveAttribute(
    "data-tauri-drag-region",
    "deep",
  );
  await expect(page.locator(".appearance-window-controls button")).toHaveCount(2);
  await expect(page.locator(".appearance-nav-button")).toHaveCount(1);
  await expect(page.locator(".appearance-nav-button span").last()).toHaveText("Pet");
  await expect(page.locator(".appearance-nav-button")).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(".appearance-sidebar button")).toHaveCount(1);
  await expect(page.locator(".appearance-scene-button")).toHaveCount(7);
  await expect(page.locator("[role=listbox]")).toHaveCount(1);
  await expect(page.locator("[role=option]")).toHaveCount(1);
  await expect(page.locator("[role=option][aria-selected=true]")).toHaveCount(1);
  await expect(page.locator("#lili-appearance")).not.toContainText("Active pet");

  const scene = page.locator(".appearance-scene-button[data-scene='review']");
  await scene.focus();
  await expect(scene).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(scene).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.locator(".appearance-scene-button[data-scene='idle']"),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("#appearance-scene")).toHaveAttribute(
    "data-scene",
    "review",
  );
  await expect(page.locator(".appearance-preview-notification")).toHaveCount(1);
  await expect(page.locator(".appearance-preview-notification button")).toHaveCount(0);
  await expect(page.locator("#appearance-preview-footer-copy")).toContainText(
    "Read-only completion card",
  );

  await expectNoHorizontalClipping(page);
});

test("each Pet list entry shows its idle atlas preview", async ({ page }) => {
  await openAppearance(page);
  const previews = page.locator(".appearance-pet-thumb-atlas");
  await expect(previews).toHaveCount(await page.locator("[role=option]").count());
  await expect(previews.first()).toHaveCSS(
    "animation-name",
    "appearance-idle-preview",
  );

  const geometry = await previews.first().evaluate((element) => {
    const frame = element.parentElement.getBoundingClientRect();
    const atlas = element.getBoundingClientRect();
    return {
      frame: { width: frame.width, height: frame.height },
      atlas: { width: atlas.width, height: atlas.height },
      overflow: getComputedStyle(element.parentElement).overflow,
    };
  });
  expect(geometry.frame.width).toBe(48);
  expect(geometry.frame.height).toBe(52);
  expect(geometry.atlas.width).toBe(384);
  expect(geometry.atlas.height).toBe(572);
  expect(geometry.overflow).toBe("hidden");
});

test("Appearance renders every preview scene with bounded read-only state", async ({
  page,
}) => {
  await openAppearance(page);
  const sceneExpectations = {
    idle: { animation: "idle", notification: false },
    running: { animation: "running", notification: false },
    review: { animation: "review", notification: true },
    attention: { animation: "waiting", notification: true },
    failed: { animation: "failed", notification: true },
    waiting: { animation: "waiting", notification: true },
    click: { animation: "waving", notification: false },
  };

  for (const [scene, expected] of Object.entries(sceneExpectations)) {
    const button = page.locator(`.appearance-scene-button[data-scene='${scene}']`);
    await button.click();
    await expect(page.locator("#appearance-scene")).toHaveAttribute(
      "data-scene",
      scene,
    );
    await expect(page.locator("#appearance-scene")).toHaveAttribute(
      "data-animation",
      expected.animation,
    );
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await expect(
      page.locator(".appearance-preview-notification"),
    ).toHaveCount(expected.notification ? 1 : 0);
    await expect(page.locator(".appearance-notification button")).toHaveCount(0);
  }
});

test("Appearance keeps fallback asset delivery, Pet selection, and preview isolation", async ({
  page,
}) => {
  const requests = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/")) {
      requests.push({ method: request.method(), path: new URL(request.url()).pathname });
    }
  });
  await openAppearance(page);

  const before = await page.request.get("/api/v1/snapshot").then((response) => response.json());
  const petOptions = page.locator("[role=option]");
  const optionCount = await petOptions.count();
  expect(optionCount).toBeGreaterThan(0);
  for (let index = 0; index < optionCount; index += 1) {
    const option = petOptions.nth(index);
    await option.focus();
    const selectionResponsePromise = page.waitForResponse(
      (response) =>
        response.url().endsWith("/api/v1/appearance/pet") &&
        response.request().method() === "PUT",
    );
    await page.keyboard.press("Enter");
    const selectionResponse = await selectionResponsePromise;
    expect(selectionResponse.ok()).toBeTruthy();
    const selectedAppearance = await selectionResponse.json();
    const selectedPetId = await option.getAttribute("data-pet-id");
    const selectedPet = selectedAppearance.pets.find(
      ({ id }) => id === selectedPetId,
    );
    expect(selectedPet).toBeDefined();
    await expect(page.locator(".appearance-pet-atlas")).toHaveAttribute(
      "src",
      `/pet-assets/${selectedPet.assetId}`,
    );
    await expect(option).toHaveAttribute("aria-selected", "true");
  }

  const atlas = page.locator(".appearance-pet-atlas");
  await expect(atlas).toHaveAttribute("src", /\/pet-assets\/.+/);
  const assetUrl = await atlas.getAttribute("src");
  expect(assetUrl).not.toBeNull();
  const fallbackAsset = await page.request.get(new URL(assetUrl, page.url()).toString());
  expect(fallbackAsset.ok()).toBeTruthy();
  expect(fallbackAsset.headers()["content-type"]).toBe("image/webp");

  for (const scene of ["idle", "review", "attention", "failed", "waiting", "click"]) {
    await page.locator(`.appearance-scene-button[data-scene='${scene}']`).click();
  }
  const after = await page.request.get("/api/v1/snapshot").then((response) => response.json());
  expect(after.sessionState).toEqual(before.sessionState);
  expect(after.actions).toEqual(before.actions);
  expect(
    requests.filter(({ path }) =>
      path === "/api/v1/interactions" || path.includes("/notifications/"),
    ),
  ).toEqual([]);

  await page.reload();
  await page.waitForFunction(() => window.__LILI_HYDRATED__ === true);
  await expect(page.locator("[role=option][aria-selected=true]")).toHaveCount(1);
  await expect(page.locator(".appearance-pet-atlas")).toHaveAttribute(
    "src",
    /\/pet-assets\/.+/,
  );
});

for (const [width, height] of [
  [320, 900],
  [736, 900],
  [1024, 900],
]) {
  test(`Appearance controls fit at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await openAppearance(page);
    await expectNoHorizontalClipping(page);

    const layout = await page.locator(".appearance-body").evaluate((element) => ({
      bodyColumns: getComputedStyle(element).gridTemplateColumns,
      workbenchColumns: getComputedStyle(
        element.querySelector(".appearance-workbench"),
      ).gridTemplateColumns,
    }));
    if (width === 1024) {
      expect(layout.bodyColumns).toMatch(/^190px /);
      expect(layout.workbenchColumns).toMatch(/300px$/);
    } else {
      expect(layout.bodyColumns.trim().split(/\s+/)).toHaveLength(1);
      expect(layout.workbenchColumns.trim().split(/\s+/)).toHaveLength(1);
    }
  });
}
