const { expect, test } = require("@playwright/test");

// Allow subpixel scroll rounding while rejecting partially clipped controls.
const FULL_VISIBILITY_RATIO = 0.995;

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

async function expectBoundedAppearance(page) {
  const geometry = await page.evaluate(() => {
    const root = document.querySelector("#lili-appearance");
    const rect = (element) => {
      const { top, left, bottom, right } = element.getBoundingClientRect();
      return { top, left, bottom, right };
    };
    return {
      width: window.innerWidth,
      height: window.innerHeight,
      documentWidth: document.documentElement.scrollWidth,
      documentHeight: document.documentElement.scrollHeight,
      rootHeight: root.clientHeight,
      rootContentHeight: root.scrollHeight,
      rootWidth: root.clientWidth,
      rootContentWidth: root.scrollWidth,
      rootScrollTop: root.scrollTop,
      frame: rect(root.querySelector(".appearance-window-frame")),
      close: rect(root.querySelector(".appearance-window-control-close")),
      scrollOwners: [...root.querySelectorAll("*")].filter((element) =>
        element.scrollHeight > element.clientHeight + 1 &&
        ["auto", "scroll"].includes(getComputedStyle(element).overflowY),
      ).map((element) => element.className),
    };
  });
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.width);
  expect(geometry.documentHeight).toBeLessThanOrEqual(geometry.height);
  expect(geometry.rootContentWidth).toBeLessThanOrEqual(geometry.rootWidth);
  expect(geometry.rootContentHeight).toBeLessThanOrEqual(geometry.rootHeight);
  expect(geometry.rootScrollTop).toBe(0);
  for (const rect of [geometry.frame, geometry.close]) {
    expect(rect.left).toBeGreaterThanOrEqual(0);
    expect(rect.top).toBeGreaterThanOrEqual(0);
    expect(rect.right).toBeLessThanOrEqual(geometry.width);
    expect(rect.bottom).toBeLessThanOrEqual(geometry.height);
  }
  const allowed = geometry.width > 900
    ? ["appearance-preview-content", "appearance-settings"]
    : ["appearance-workbench"];
  for (const owner of geometry.scrollOwners) expect(allowed).toContain(owner);
  await expect(page.getByRole("button", { name: "Close Appearance window" })).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
}

for (const [width, height] of [
  [1180, 900], [1180, 600], [1024, 768], [901, 600],
  [900, 600], [736, 600], [590, 450], [320, 480],
]) {
  test(`Appearance bounds every preview mode at ${width}x${height}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openAppearance(page);
    for (const scene of ["idle", "review"]) {
      await page.locator(`.appearance-scene-button[data-scene=${scene}]`).click();
      await page.locator("#appearance-pet").scrollIntoViewIfNeeded();
      await expect(page.locator("#appearance-pet")).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
      await expectBoundedAppearance(page);
    }
    await page.getByRole("button", { name: "Sprites", exact: true }).click();
    for (const [category, group] of [
      ["Animations", ".appearance-animation-frames"],
      ["Look directions", '[role=group][aria-label="Look directions"]'],
      ["Sprite sheet", ".appearance-sheet-grid"],
    ]) {
      await page.getByRole("button", { name: category, exact: true }).click();
      const last = page.locator(`${group} button`).last();
      await last.focus();
      await page.keyboard.press("Enter");
      await expect(last).toHaveAttribute("aria-pressed", "true");
      await expect(last).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
      await expectBoundedAppearance(page);
      await page.locator(".appearance-sprite-image").scrollIntoViewIfNeeded();
      await expect(page.locator(".appearance-sprite-image")).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
      await expectBoundedAppearance(page);
    }
    await page.getByRole("switch", { name: "Launch at login" }).scrollIntoViewIfNeeded();
    await expect(page.getByRole("switch", { name: "Launch at login" })).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
    await expectBoundedAppearance(page);
    if (width === 1180 && height === 900 || width === 320) {
      await page.locator(".appearance-preview-content, .appearance-workbench").evaluateAll((elements) => {
        for (const element of elements) element.scrollTop = 0;
      });
      await page.screenshot({ path: testInfo.outputPath("bounded-appearance.png") });
    }
  });
}

test("Appearance keeps long Pet lists reachable while resizing an open sprite sheet", async ({ page }) => {
  await page.setViewportSize({ width: 1180, height: 900 });
  await openAppearance(page);
  const original = await page.request.get("/api/v1/appearance").then((response) => response.json());
  const asset = await page.request.get(`/pet-assets/${original.pets[0].assetId}`);
  const body = await asset.body();
  const pets = [original.pets[0], ...Array.from({ length: 63 }, (_, index) => ({
    id: `layout-${index}`,
    displayName: `Pet ${index + 2} with a long display name to exercise the available panel width`,
    assetId: `layout-${index}`,
  }))];
  const current = { pets, selectedPetId: pets[0].id };
  await page.route("**/pet-assets/layout-*", (route) => route.fulfill({ body, contentType: "image/webp" }));
  await page.route("**/api/v1/appearance", (route) => route.fulfill({ json: current }));
  await page.route("**/api/v1/appearance/pet", (route) => {
    current.selectedPetId = route.request().postDataJSON().petId;
    return route.fulfill({ json: current });
  });
  await expect(page.getByRole("option")).toHaveCount(64);
  await page.getByRole("button", { name: "Sprites", exact: true }).click();
  await page.getByRole("button", { name: "Sprite sheet", exact: true }).click();
  for (const [width, height] of [[1180, 900], [590, 450], [320, 480], [1180, 600]]) {
    await page.setViewportSize({ width, height });
    const lastPet = page.getByRole("option").last();
    await lastPet.focus();
    await page.keyboard.press("Enter");
    await expect(lastPet).toHaveAttribute("aria-selected", "true");
    await expect(lastPet).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
    await expectBoundedAppearance(page);
    const lastCell = page.locator(".appearance-sheet-grid button").last();
    await lastCell.focus();
    await page.keyboard.press("Enter");
    await expect(lastCell).toHaveAttribute("aria-pressed", "true");
    await expect(lastCell).toBeInViewport({ ratio: FULL_VISIBILITY_RATIO });
    await expectBoundedAppearance(page);
  }
});

test("Appearance fits the initial desktop window without vertical scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 1180, height: 900 });
  await openAppearance(page);

  const geometry = await page.locator("#lili-appearance").evaluate((surface) => ({
    height: surface.clientHeight,
    contentHeight: surface.scrollHeight,
    frameBottom: surface.querySelector(".appearance-window-frame").getBoundingClientRect().bottom,
  }));
  expect(geometry.contentHeight).toBeLessThanOrEqual(geometry.height);
  expect(geometry.frameBottom).toBeLessThanOrEqual(geometry.height);
  await expectNoHorizontalClipping(page);
});

test("Appearance keeps Pet navigation and scene controls keyboard reachable", async ({
  page,
}) => {
  await openAppearance(page);

  await expect(page.locator(".appearance-window-frame")).toBeVisible();
  await expect(page.locator(".appearance-window-frame")).toHaveCSS(
    "overflow",
    "hidden",
  );
  await expect(page.locator(".appearance-main")).toHaveCSS(
    "overflow",
    "hidden",
  );
  await expect(page.locator(".appearance-topbar")).toHaveAttribute(
    "data-tauri-drag-region",
    "deep",
  );
  const closeControl = page.locator(".appearance-window-controls button");
  await expect(closeControl).toHaveCount(1);
  await expect(closeControl).toHaveAttribute(
    "aria-label",
    "Close Appearance window",
  );
  await expect(page.locator(".appearance-page-label")).toHaveCount(0);
  await expect(page.locator(".appearance-brand-mark")).toHaveCount(0);
  await expect(page.locator(".appearance-desktop-surface")).toHaveCount(0);
  await expect(page.locator(".appearance-scene-badge")).toHaveCount(0);
  const topbarOrder = await page.evaluate(() => {
    const control = document.querySelector(".appearance-window-controls");
    const brand = document.querySelector(".appearance-brand");
    return {
      controlLeft: control.getBoundingClientRect().left,
      brandLeft: brand.getBoundingClientRect().left,
    };
  });
  expect(topbarOrder.controlLeft).toBeLessThan(topbarOrder.brandLeft);
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
  await expect(page.locator(".notification-card-preview")).toHaveCount(1);
  await expect(
    page.locator(".notification-card-preview .notification-controls button"),
  ).toHaveCount(2);
  const previewControls = page.locator(
    ".notification-card-preview .notification-controls button",
  );
  await expect(previewControls.nth(0)).toBeDisabled();
  await expect(previewControls.nth(1)).toBeDisabled();
  await expect(page.locator(".notification-card-preview")).toContainText(
    "Task completed successfully.",
  );

  await expectNoHorizontalClipping(page);
});

test("Appearance keeps text selection inside its own container", async ({
  page,
}) => {
  await openAppearance(page);

  await expect(page.locator("#lili-appearance")).toHaveCSS("user-select", "none");
  const selectableCopy = page.locator(".appearance-pet-item-copy").first();
  await expect(selectableCopy).toHaveCSS("user-select", "text");
  const selectableHeading = page.locator("#appearance-heading");
  await expect(selectableHeading).toHaveCSS("user-select", "text");
  const selectableCopyText = await selectableCopy.innerText();
  const selectableHeadingText = await selectableHeading.innerText();
  const headingBox = await selectableHeading.boundingBox();
  const copyBox = await selectableCopy.boundingBox();
  if (!headingBox || !copyBox) {
    throw new Error("selectable text containers must be measurable");
  }
  await page.mouse.move(headingBox.x + 1, headingBox.y + headingBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    copyBox.x + copyBox.width - 1,
    copyBox.y + copyBox.height - 1,
  );
  await page.mouse.up();
  const selectedText = await page.evaluate(() => {
    const selection = window.getSelection();
    return selection?.toString() ?? "";
  });
  expect(selectedText).toContain(selectableHeadingText);
  expect(selectedText).not.toContain(selectableCopyText);
  expect(selectedText).not.toContain("Live preview");
  expect(selectedText).not.toContain("✓");
  await page.evaluate(() => window.getSelection()?.removeAllRanges());

  await page.mouse.move(
    copyBox.x + copyBox.width - 1,
    copyBox.y + copyBox.height - 1,
  );
  await page.mouse.down();
  await page.mouse.move(headingBox.x + headingBox.width - 1, headingBox.y + 1);
  await page.mouse.up();
  const reverseSelectedText = await page.evaluate(() => {
    const selection = window.getSelection();
    return selection?.toString() ?? "";
  });
  expect(reverseSelectedText).toContain(selectableCopyText);
  expect(reverseSelectedText).not.toContain(selectableHeadingText);
  await page.evaluate(() => window.getSelection()?.removeAllRanges());
});

test("each Pet list entry shows its idle atlas preview", async ({ page }) => {
  await openAppearance(page);
  const previews = page.locator(".appearance-pet-thumb-atlas");
  await expect(previews).toHaveCount(await page.locator("[role=option]").count());
  await expect(previews.first()).toHaveCSS(
    "animation-name",
    "appearance-idle-preview",
  );
  await expect(page.locator(".appearance-pet-atlas")).toHaveAttribute(
    "draggable",
    "false",
  );
  await expect(previews.first()).toHaveAttribute("draggable", "false");
  await expect(page.locator(".appearance-pet-atlas")).toHaveCSS(
    "pointer-events",
    "none",
  );
  await expect(previews.first()).toHaveCSS("pointer-events", "none");

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
      page.locator(".notification-card-preview"),
    ).toHaveCount(expected.notification ? 1 : 0);
    await expect(
      page.locator(".notification-card-preview .notification-controls button"),
    ).toHaveCount(expected.notification ? 2 : 0);
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


test("pet preview supports idle interactions and isolated dragging in every scene", async ({ page }) => {
  await openAppearance(page);
  await page.evaluate(() => {
    window.__previewNativeCalls = [];
    window.__TAURI_INTERNALS__ = {
      invoke: (...args) => {
        window.__previewNativeCalls.push(args);
        return Promise.resolve();
      },
    };
  });
  const requests = [];
  page.on("request", (request) => {
    if (request.method() !== "GET") requests.push(request.url());
  });
  const pet = page.locator("#appearance-pet");
  const scene = page.locator("#appearance-scene");
  const atlas = page.locator(".appearance-pet-atlas");
  await expect(page.locator(".appearance-pet-tag")).toHaveCount(0);
  await pet.hover({ position: { x: 180, y: 104 } });
  await expect(atlas).toHaveAttribute("data-frame-row", /^(9|10)$/);
  await pet.click();
  await expect(scene).toHaveAttribute("data-animation", "waving");
  await expect(scene).toHaveAttribute("data-animation", "idle");
  await pet.dblclick();
  await expect(scene).toHaveAttribute("data-animation", "jumping");
  await expect(scene).toHaveAttribute("data-animation", "idle");
  await pet.focus();
  await page.keyboard.press("Space");
  await expect(scene).toHaveAttribute("data-animation", "waving");

  for (const state of ["idle", "running", "review", "attention", "failed", "waiting", "click"]) {
    await page.locator(`.appearance-scene-button[data-scene='${state}']`).click();
    const baseline = await scene.getAttribute("data-animation");
    const box = await pet.boundingBox();
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + 80, y);
    await expect(scene).toHaveAttribute("data-animation", "running-right");
    const initialColumn = await atlas.getAttribute("data-frame-column");
    // Hold beyond the live pet's velocity timeout without sending pointer events.
    await page.waitForTimeout(400);
    await expect(scene).toHaveAttribute("data-animation", "running-right");
    await expect(atlas).not.toHaveAttribute("data-frame-column", initialColumn);
    await page.mouse.move(x - 80, y);
    await expect(scene).toHaveAttribute("data-animation", "running-left");
    await page.waitForTimeout(400);
    await expect(scene).toHaveAttribute("data-animation", "running-left");
    expect(await pet.boundingBox()).toEqual(box);
    await page.mouse.up();
    await expect(scene).toHaveAttribute("data-animation", baseline);
    await expect(scene).toHaveAttribute("data-scene", state);
  }
  expect(requests).toEqual([]);
  expect(await page.evaluate(() => window.__previewNativeCalls)).toEqual([]);
});


test("scaled pet preview keeps gaze centered and follows both horizontal directions", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await openAppearance(page);
  const pet = page.locator("#appearance-pet");
  await pet.scrollIntoViewIfNeeded();
  const box = await pet.boundingBox();
  const atlas = page.locator(".appearance-pet-atlas");
  const centerX = box.x + box.width / 2;
  const centerY = box.y + box.height / 2;
  await page.mouse.move(centerX, centerY);
  await expect(atlas).toHaveAttribute("data-frame-row", "0");
  await page.mouse.move(box.x + box.width - 10, centerY);
  await expect(atlas).toHaveAttribute("data-frame-row", /^(9|10)$/);
  const rightRow = await atlas.getAttribute("data-frame-row");
  await page.mouse.move(box.x + 10, centerY);
  await expect(atlas).toHaveAttribute("data-frame-row", /^(9|10)$/);
  await expect(atlas).not.toHaveAttribute("data-frame-row", rightRow);
  await page.mouse.move(centerX, centerY);
  await expect(atlas).toHaveAttribute("data-frame-row", "0");
});

test("Startup is unavailable in browser previews", async ({ page }) => {
  await openAppearance(page);
  await expect(page.getByRole("switch", { name: "Launch at login" })).toBeDisabled();
  await expect(page.getByText("Available in the desktop Settings window.")).toBeVisible();
});

test("Sprites exposes every animation frame with deterministic manual playback", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openAppearance(page);
  await page.getByRole("button", { name: "Sprites", exact: true }).click();
  const atlas = page.locator(".appearance-sprite-image img");
  const animations = page.getByRole("group", { name: "Animations", exact: true });
  await expect(animations.getByRole("button")).toHaveCount(9);
  const counts = [6, 8, 8, 4, 5, 8, 6, 6, 6];
  for (let row = 0; row < counts.length; row += 1) {
    await animations.getByRole("button").nth(row).click();
    const frames = page.getByRole("group", { name: "Animation frames" }).getByRole("button");
    await expect(frames).toHaveCount(counts[row]);
    for (let column = 0; column < counts[row]; column += 1) {
      await frames.nth(column).click();
      await expect(atlas).toHaveAttribute("data-frame-row", String(row));
      await expect(atlas).toHaveAttribute("data-frame-column", String(column));
    }
    await page.getByRole("button", { name: "Next frame", exact: true }).click();
    await expect(atlas).toHaveAttribute("data-frame-column", "0");
    await page.getByRole("button", { name: "Previous frame", exact: true }).click();
    await expect(atlas).toHaveAttribute("data-frame-column", String(counts[row] - 1));
  }
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeDisabled();
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(atlas).not.toHaveAttribute("data-frame-column", "5");
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const column = await atlas.getAttribute("data-frame-column");
  await page.waitForTimeout(350);
  await expect(atlas).toHaveAttribute("data-frame-column", column);
});

test("Sprites fixes every gaze direction and exposes the complete sheet without runtime actions", async ({ page }, testInfo) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1180, height: 900 });
  const mutations = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/") && request.method() !== "GET") {
      mutations.push(request.url());
    }
  });
  await openAppearance(page);
  await page.locator(".appearance-scene-button[data-scene=review]").click();
  await page.getByRole("button", { name: "Sprites", exact: true }).click();
  await page.getByRole("button", { name: "Look directions", exact: true }).click();
  const atlas = page.locator(".appearance-sprite-image img");
  const directions = page.getByRole("group", { name: "Look directions", exact: true }).getByRole("button");
  await expect(directions).toHaveCount(17);
  for (let index = 0; index < 16; index += 1) {
    await directions.nth(index + 1).click();
    await expect(atlas).toHaveAttribute("data-frame-row", String(9 + Math.floor(index / 8)));
    await expect(atlas).toHaveAttribute("data-frame-column", String(index % 8));
  }
  await directions.first().focus();
  await page.keyboard.press("Enter");
  await expect(atlas).toHaveAttribute("data-frame-row", "0");
  await expect(atlas).toHaveAttribute("data-frame-column", "6");
  await page.locator(".appearance-sprite-image").dblclick();
  await page.waitForTimeout(350);
  await expect(atlas).toHaveAttribute("data-frame-column", "6");
  await page.getByRole("button", { name: "Sprite sheet", exact: true }).click();
  const cells = page.locator(".appearance-sheet-grid button");
  await expect(cells).toHaveCount(88);
  await expect(page.locator(".appearance-sprite-unused")).toHaveCount(14);
  for (let index = 0; index < 88; index += 1) {
    await cells.nth(index).click();
    await expect(atlas).toHaveAttribute("data-frame-row", String(Math.floor(index / 8)));
    await expect(atlas).toHaveAttribute("data-frame-column", String(index % 8));
  }
  await page.locator("#lili-appearance").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("sprite-sheet.png"), fullPage: true });
  await page.getByRole("button", { name: "Scenes", exact: true }).click();
  await expect(page.locator("#appearance-scene")).toHaveAttribute("data-scene", "review");
  await expect(page.locator(".notification-card-preview")).toBeVisible();
  expect(mutations).toEqual([]);
});

test("Sprites keeps its target and resets the frame when switching pets", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openAppearance(page);
  const appearance = await page.request.get("/api/v1/appearance").then((response) => response.json());
  const original = appearance.pets[0];
  const alternate = { id: "preview-alternate", displayName: "Alternate pet", assetId: "preview-alternate" };
  const current = { ...appearance, pets: [original, alternate] };
  await page.route("**/pet-assets/preview-alternate", async (route) => {
    const response = await page.request.get(`/pet-assets/${original.assetId}`);
    await route.fulfill({ response });
  });
  await page.route("**/api/v1/appearance/pet", (route) => {
    current.selectedPetId = route.request().postDataJSON().petId;
    return route.fulfill({ json: current });
  });
  await page.route("**/api/v1/appearance", (route) => route.fulfill({ json: current }));
  await expect(page.getByRole("option")).toHaveCount(2);
  await page.getByRole("button", { name: "Sprites", exact: true }).click();
  await page.getByRole("button", { name: "Run left", exact: true }).click();
  await page.getByRole("button", { name: "Next frame", exact: true }).click();
  const atlas = page.locator(".appearance-sprite-image img");
  await expect(atlas).toHaveAttribute("data-frame-column", "1");
  await page.getByRole("option", { name: /Alternate pet/ }).click();
  await expect(atlas).toHaveAttribute("src", "/pet-assets/preview-alternate");
  await expect(atlas).toHaveAttribute("data-frame-row", "2");
  await expect(atlas).toHaveAttribute("data-frame-column", "0");
  await expect(page.getByRole("button", { name: "Run left", exact: true })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Next frame", exact: true }).click();
  await page.waitForTimeout(600);
  await expect(atlas).toHaveAttribute("data-frame-column", "1");
  await page.getByRole("button", { name: "Look directions", exact: true }).click();
  await page.getByRole("button", { name: "Look 90°", exact: true }).click();
  await page.getByRole("option").first().click();
  await expect(atlas).toHaveAttribute("src", `/pet-assets/${original.assetId}`);
  await expect(atlas).toHaveAttribute("data-frame-row", "9");
  await expect(atlas).toHaveAttribute("data-frame-column", "4");
});

test("Sprites keeps the full cell visible and confines sheet scrolling on narrow windows", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await openAppearance(page);
  await page.getByRole("button", { name: "Sprites", exact: true }).click();
  await expectNoHorizontalClipping(page);
  await page.getByRole("button", { name: "Sprite sheet", exact: true }).click();
  const last = page.locator(".appearance-sheet-grid button").last();
  await last.focus();
  await page.keyboard.press("Enter");
  await expect(last).toHaveAttribute("aria-pressed", "true");
  const geometry = await page.locator(".appearance-sprite-image").evaluate((image) => ({
    width: image.getBoundingClientRect().width,
    height: image.getBoundingClientRect().height,
    left: image.getBoundingClientRect().left,
    right: image.getBoundingClientRect().right,
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: document.documentElement.clientWidth,
  }));
  expect(geometry.width / geometry.height).toBeCloseTo(192 / 208);
  expect(geometry.left).toBeGreaterThanOrEqual(0);
  expect(geometry.right).toBeLessThanOrEqual(geometry.viewportWidth);
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
  await page.locator("#lili-appearance").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: testInfo.outputPath("sprite-sheet-narrow.png"), fullPage: true });
});

test("Startup reads system state, toggles, and recovers from failures", async ({ page }) => {
  await page.addInitScript(() => {
    window.startupEnabled = true;
    window.startupFail = false;
    window.startupCommands = [];
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "appearance" } },
      invoke: async (command) => {
        window.startupCommands.push(command);
        if (window.startupFail) throw new Error("Registration failed");
        if (command === "plugin:autostart|enable") window.startupEnabled = true;
        if (command === "plugin:autostart|disable") window.startupEnabled = false;
        return window.startupEnabled;
      },
    };
  });
  await openAppearance(page);
  const toggle = page.getByRole("switch", { name: "Launch at login" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await page.evaluate(() => { window.startupFail = true; });
  await toggle.click();
  await expect(page.getByRole("alert")).toContainText("Unable to update or read");
  await expect(toggle).toBeDisabled();
  await page.evaluate(() => { window.startupFail = false; });
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(toggle).toBeEnabled();
  await page.evaluate(() => {
    window.startupEnabled = false;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await page.evaluate(() => {
    window.startupEnabled = true;
    window.dispatchEvent(new Event("lili-appearance-shown"));
  });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(await page.evaluate(() => window.startupCommands)).toContain("plugin:autostart|disable");
  expect(await page.evaluate(() => window.startupCommands)).toContain("plugin:autostart|enable");
  await page.screenshot({ path: test.info().outputPath("startup-settings.png"), fullPage: true });
});
