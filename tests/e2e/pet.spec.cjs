const { expect, test } = require("@playwright/test");

const now = 1_800_000_000_000;

function notification(id, kind, project, summary, occurredAtMs) {
  return {
    activationId: id,
    kind,
    projectLabel: project,
    summary,
    summaryTruncated: false,
    summaryRedacted: false,
    occurredAtMs,
    unread: true,
  };
}

async function openPet(page) {
  await page.goto("/");
  await page.waitForFunction(() => window.__LILI_HYDRATED__ === true);
  await page.waitForTimeout(100);
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-ssr-marker",
    "lili-ready",
  );
}

async function openNotifications(page) {
  await page.goto("/notifications");
  await page.waitForFunction(() => window.__LILI_HYDRATED__ === true);
  await page.waitForTimeout(100);
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-surface",
    "notifications",
  );
}

async function replacePresentation(page, overrides = {}) {
  const app = page.locator("#lili-app");
  const current = JSON.parse(await app.getAttribute("data-presentation"));
  const next = {
    revision: current.revision,
    lifecycle: "idle",
    petAssetId: current.petAssetId,
    petLabel: "Lili",
    unreadNotificationCount: 0,
    notifications: [],
    actionFeedback: null,
    reducedMotion: false,
    ...overrides,
  };
  const response = await page.request.put("/__fixture/presentation", {
    data: next,
  });
  expect(response.ok()).toBeTruthy();
  const applied = await response.json();
  await expect(app).toHaveAttribute("data-revision", String(applied.revision));
  return applied;
}

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

test("package selection replaces the approved presentation identity", async ({
  page,
}) => {
  await openPet(page);
  await replacePresentation(page, {
    petAssetId: "fixture-selected-pet",
    petLabel: "Selected Pet",
  });

  await expect(page.locator(".pet-sprite")).toHaveAttribute(
    "aria-label",
    "Selected Pet, idle",
  );
  await expect(page.locator(".pet-atlas")).toHaveAttribute(
    "src",
    "/pet-assets/fixture-selected-pet",
  );
  const asset = await page.request.get("/pet-assets/fixture-selected-pet");
  expect(asset.ok()).toBeTruthy();
  expect(asset.headers()["content-type"]).toBe("image/webp");
});

test("every standard animation state is observable", async ({ page }) => {
  await openPet(page);
  const app = page.locator("#lili-app");
  for (const [lifecycle, animation] of [
    ["idle", "idle"],
    ["activity_reminder", "running"],
    ["review", "review"],
    ["failed", "failed"],
    ["waiting", "waiting"],
  ]) {
    await replacePresentation(page, { lifecycle });
    await expect(app).toHaveAttribute("data-animation", animation);
  }

  await replacePresentation(page);
  const pet = page.locator(".pet-sprite");
  await pet.focus();
  await page.keyboard.press("Enter");
  await expect(app).toHaveAttribute("data-animation", "waving");

  await replacePresentation(page);
  await pet.dblclick({ delay: 40 });
  await expect(app).toHaveAttribute("data-animation", "jumping");

  await replacePresentation(page, { lifecycle: "failed" });
  await expect(app).toHaveAttribute("data-animation", "failed");
  await replacePresentation(page);
  await expect(app).toHaveAttribute("data-animation", "idle");

  const box = await pet.boundingBox();
  expect(box).not.toBeNull();
  await pet.dispatchEvent("pointerdown", {
    pointerId: 1,
    clientX: box.x + 20,
    clientY: box.y + 104,
    screenX: box.x + 20,
    screenY: box.y + 104,
    isPrimary: true,
    button: 0,
    buttons: 1,
  });
  await page.waitForTimeout(10);
  await pet.dispatchEvent("pointermove", {
    pointerId: 1,
    clientX: box.x + 172,
    clientY: box.y + 104,
    screenX: box.x + 172,
    screenY: box.y + 104,
    isPrimary: true,
    buttons: 1,
  });
  await expect(app).toHaveAttribute("data-animation", "running-right");
  await page.waitForTimeout(40);
  await pet.dispatchEvent("pointermove", {
    pointerId: 1,
    clientX: box.x + 172,
    clientY: box.y + 104,
    screenX: box.x + 172,
    screenY: box.y + 104,
    isPrimary: true,
    buttons: 1,
  });
  await expect(app).toHaveAttribute("data-animation", "running-right");
  await page.waitForTimeout(140);
  await expect(app).toHaveAttribute("data-animation", "idle");
  await pet.dispatchEvent("pointerup", {
    pointerId: 1,
    isPrimary: true,
    button: 0,
  });

  await pet.dispatchEvent("pointerdown", {
    pointerId: 2,
    clientX: box.x + 172,
    clientY: box.y + 104,
    screenX: box.x + 172,
    screenY: box.y + 104,
    isPrimary: true,
    button: 0,
    buttons: 1,
  });
  await page.waitForTimeout(10);
  await pet.dispatchEvent("pointermove", {
    pointerId: 2,
    clientX: box.x + 20,
    clientY: box.y + 104,
    screenX: box.x + 20,
    screenY: box.y + 104,
    isPrimary: true,
    buttons: 1,
  });
  await expect(app).toHaveAttribute("data-animation", "running-left");
  await pet.dispatchEvent("pointerup", {
    pointerId: 2,
    isPrimary: true,
    button: 0,
  });
});

test("pet owns the context menu gesture", async ({ page }) => {
  await openPet(page);
  await page.evaluate(() => {
    window.__LILI_INVOKES__ = [];
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        window.__LILI_INVOKES__.push({ name, args });
        return true;
      },
    };
  });
  const pet = page.locator(".pet-sprite");
  const box = await pet.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box.x + 80, box.y + 90, { button: "right" });
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__LILI_INVOKES__.filter((call) => call.name === "open_pet_context_menu"),
      ),
    )
    .toEqual([
      {
        name: "open_pet_context_menu",
        args: { screenX: box.x + 80, screenY: box.y + 90 },
      },
    ]);
  const result = await pet.evaluate((element) => {
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: 80,
      clientY: 90,
    });
    return {
      dispatchResult: element.dispatchEvent(event),
      defaultPrevented: event.defaultPrevented,
    };
  });
  expect(result).toEqual({ dispatchResult: false, defaultPrevented: true });

  const invokeCount = await page.evaluate(() => window.__LILI_INVOKES__.length);
  const backgroundResult = await page.locator("#lili-app").evaluate((element) => {
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
    });
    return {
      dispatchResult: element.dispatchEvent(event),
      defaultPrevented: event.defaultPrevented,
    };
  });
  expect(backgroundResult).toEqual({
    dispatchResult: false,
    defaultPrevented: true,
  });
  expect(await page.evaluate(() => window.__LILI_INVOKES__.length)).toBe(invokeCount);
});

test("left drag continues to use native window movement", async ({ page }) => {
  await openPet(page);
  await page.evaluate(() => {
    window.__LILI_INVOKES__ = [];
    window.__LILI_DRAG_STARTS__ = 0;
    document.addEventListener("dragstart", () => {
      window.__LILI_DRAG_STARTS__ += 1;
    });
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        window.__LILI_INVOKES__.push({ name, args });
        return true;
      },
    };
  });

  const pet = page.locator(".pet-sprite");
  const box = await pet.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box.x + 80, box.y + 90, { button: "right" });
  await page.mouse.move(box.x + 20, box.y + 104);
  await page.mouse.down();
  await page.mouse.move(box.x + 172, box.y + 104, { steps: 2 });
  await page.mouse.up();

  await expect
    .poll(() =>
      page.evaluate(() => window.__LILI_INVOKES__.map((call) => call.name)),
    )
    .toContain("commit_window_position");
  const calls = await page.evaluate(() => window.__LILI_INVOKES__);
  expect(calls.map((call) => call.name)).toEqual(
    expect.arrayContaining(["begin_window_drag", "move_window_to"]),
  );
  expect(await page.evaluate(() => window.__LILI_DRAG_STARTS__)).toBe(0);
});

test("pet window content cannot be selected", async ({ page }) => {
  await openPet(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification(
        "non-selectable",
        "completion",
        "Workspace",
        "Finished safely",
        now,
      ),
    ],
  });

  for (const selector of ["#lili-app", ".pet-sprite"]) {
    await expect
      .poll(() =>
        page.locator(selector).evaluate((element) => getComputedStyle(element).userSelect),
      )
      .toBe("none");
  }
  await expect(page.locator(".notification-card")).toHaveCount(0);

  await openNotifications(page);
  for (const selector of [
    "#lili-app",
    ".notification-card",
    ".notification-summary",
    ".notification-activate",
  ]) {
    await expect
      .poll(() =>
        page.locator(selector).evaluate((element) => getComputedStyle(element).userSelect),
      )
      .toBe("none");
  }
  await expect(page.locator(".pet-sprite")).toHaveCount(0);
});

test("look direction follows all four quadrants", async ({ page }) => {
  await openPet(page);
  await replacePresentation(page);
  const pet = page.locator(".pet-sprite");
  const atlas = page.locator(".pet-atlas");
  const quadrants = [
    [{ x: 160, y: 30 }, ["9", "2"]],
    [{ x: 160, y: 180 }, ["9", "6"]],
    [{ x: 30, y: 180 }, ["10", "2"]],
    [{ x: 30, y: 30 }, ["10", "6"]],
  ];
  for (const [position, [row, column]] of quadrants) {
    await pet.hover({ position });
    await expect(atlas).toHaveAttribute("data-frame-row", row);
    await expect(atlas).toHaveAttribute("data-frame-column", column);
  }
});

test("concurrent session notifications fill upward from the bottom", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = [
    notification("attention-old", "attention", "Alpha", "Needs input", now + 100),
    notification("failure-middle", "failure", "Beta", "Failed", now + 200),
    notification("completion-new", "completion", "Gamma", "Completed", now + 300),
  ];
  await replacePresentation(page, {
    lifecycle: "failed",
    unreadNotificationCount: notifications.length,
    notifications,
  });

  await expect(page.locator(".notification-card")).toHaveCount(3);
  const notificationIds = await page
    .locator(".notification-card")
    .evaluateAll((cards) => cards.map((card) => card.dataset.notificationId));
  expect(notificationIds).toEqual([
    "completion-new",
    "failure-middle",
    "attention-old",
  ]);
  const visualOrder = await page.locator(".notification-card").evaluateAll((cards) =>
    cards
      .map((card) => ({
        id: card.dataset.notificationId,
        top: card.getBoundingClientRect().top,
      }))
      .sort((left, right) => left.top - right.top)
      .map((card) => card.id),
  );
  expect(visualOrder).toEqual([
    "attention-old",
    "failure-middle",
    "completion-new",
  ]);
  const bottom = await page
    .locator("[data-notification-id='completion-new']")
    .evaluate((card) => card.getBoundingClientRect().bottom);
  const viewportHeight = await page.evaluate(() => window.innerHeight);
  expect(bottom).toBe(viewportHeight - 4);
  await expect(page.locator("#lili-app")).toHaveAttribute("data-unread-count", "3");
});

test("a newer same-priority notification occupies the bottom slot", async ({
  page,
}) => {
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("completion-old", "completion", "Alpha", "Older", now),
    ],
  });
  await replacePresentation(page, {
    unreadNotificationCount: 2,
    notifications: [
      notification("completion-new", "completion", "Beta", "Newer", now + 100),
      notification("completion-old", "completion", "Alpha", "Older", now),
    ],
  });

  const bottomMost = await page.locator(".notification-card").evaluateAll((cards) =>
    cards
      .map((card) => ({
        id: card.dataset.notificationId,
        bottom: card.getBoundingClientRect().bottom,
      }))
      .sort((left, right) => right.bottom - left.bottom)[0].id,
  );
  expect(bottomMost).toBe("completion-new");
});

test("below-pet placement keeps the newest card at the top edge", async ({
  page,
}) => {
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("below-pet", "completion", "Workspace", "Done", now),
    ],
  });
  await page.evaluate(() => {
    document.documentElement.dataset.notificationPlacement = "below";
  });

  const cardTop = await page
    .locator("[data-notification-id='below-pet']")
    .evaluate((card) => card.getBoundingClientRect().top);
  expect(cardTop).toBe(4);
});

test("notification surface reports a compact native window height", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__LILI_INVOKES__ = [];
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        window.__LILI_INVOKES__.push({ name, args });
        return true;
      },
    };
  });
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("compact-window", "completion", "Workspace", "Done", now),
    ],
  });

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__LILI_INVOKES__
          .filter((call) => call.name === "resize_notification_window")
          .at(-1)?.args?.height,
      ),
    )
    .toBeGreaterThanOrEqual(16);
  const height = await page.evaluate(() =>
    window.__LILI_INVOKES__
      .filter((call) => call.name === "resize_notification_window")
      .at(-1).args.height,
  );
  expect(height).toBeLessThan(158);
});

test("notification surface suppresses the browser context menu", async ({
  page,
}) => {
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("context-menu", "completion", "Workspace", "Done", now),
    ],
  });

  const result = await page.locator(".notification-card").evaluate((card) => {
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
    });
    return {
      dispatchResult: card.dispatchEvent(event),
      defaultPrevented: event.defaultPrevented,
    };
  });
  expect(result).toEqual({ dispatchResult: false, defaultPrevented: true });
});

test("keyboard shortcuts move focus between companion surfaces", async ({
  page,
}) => {
  await openPet(page);
  await page.evaluate(() => {
    window.__LILI_INVOKES__ = [];
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        window.__LILI_INVOKES__.push({ name, args });
        return true;
      },
    };
  });
  await page.locator(".pet-sprite").focus();
  await page.keyboard.press("Alt+n");
  await expect
    .poll(() => page.evaluate(() => window.__LILI_INVOKES__))
    .toContainEqual({ name: "focus_notification_window", args: undefined });

  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("keyboard-focus", "completion", "Workspace", "Done", now),
    ],
  });
  await page.evaluate(() => {
    window.__LILI_INVOKES__ = [];
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        window.__LILI_INVOKES__.push({ name, args });
        return true;
      },
    };
  });
  await page.locator(".notification-activate").focus();
  await page.keyboard.press("Escape");
  await expect
    .poll(() => page.evaluate(() => window.__LILI_INVOKES__))
    .toContainEqual({ name: "focus_pet_window", args: undefined });
});

test("action outcomes use bounded user-facing feedback", async ({ page }) => {
  await openPet(page);
  for (const [kind, message] of [
    ["success", "Action completed"],
    ["failure", "Action timed out"],
    ["busy", "Action is busy"],
  ]) {
    await replacePresentation(page, {
      actionFeedback: {
        actionId: `fixture-${kind}`,
        kind,
        message,
        occurredAtMs: now,
      },
    });
    const feedback = page.locator(".action-feedback");
    await expect(feedback).toHaveAttribute("data-action-result", kind);
    await expect(feedback).toContainText(message);
  }
});

test("reload recovers the latest snapshot", async ({ page }) => {
  await openNotifications(page);
  const notifications = [
    notification("reload-attention", "attention", "Workspace", "Needs input", now),
  ];
  const applied = await replacePresentation(page, {
    lifecycle: "waiting",
    unreadNotificationCount: 1,
    notifications,
  });

  await page.reload();
  await page.waitForFunction(() => window.__LILI_HYDRATED__ === true);
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-revision",
    String(applied.revision),
  );
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-unread-count",
    "1",
  );
  await expect(page.locator("[data-notification-id='reload-attention']")).toBeVisible();
});

test("reduced motion freezes loops and disables gaze", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openPet(page);
  await replacePresentation(page, { lifecycle: "activity_reminder" });
  const app = page.locator("#lili-app");
  const pet = page.locator(".pet-sprite");
  const atlas = page.locator(".pet-atlas");
  await expect(app).toHaveAttribute("data-reduced-motion", "true");
  await expect(app).toHaveAttribute("data-lifecycle", "activity-reminder");
  await expect(app).toHaveAttribute("data-animation", "running");
  const frame = [
    await atlas.getAttribute("data-frame-row"),
    await atlas.getAttribute("data-frame-column"),
  ];
  await page.waitForTimeout(400);
  await pet.hover({ position: { x: 160, y: 30 } });
  await page.waitForTimeout(100);
  await expect(atlas).toHaveAttribute("data-frame-row", frame[0]);
  await expect(atlas).toHaveAttribute("data-frame-column", frame[1]);
});
