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

  await expect(page.locator(".appearance-nav-button")).toHaveCount(1);
  await expect(page.locator(".appearance-nav-button")).toHaveText("Pet");
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
