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

test("pet context menu text cannot be selected", async ({ page }) => {
  await page.goto("/context-menu");
  const menu = page.locator("menu");
  const button = menu.locator("button").first();
  await expect(menu).toBeVisible();
  await expect(menu).toHaveCSS("user-select", "none");
  await expect(button).toHaveCSS("user-select", "none");

  const bounds = await button.boundingBox();
  expect(bounds).not.toBeNull();
  await page.mouse.move(bounds.x + 8, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width - 8, bounds.y + bounds.height / 2, {
    steps: 4,
  });
  await page.mouse.up();

  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");
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

test("notifications use icon-only controls with accessible names", async ({
  page,
}) => {
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("icon-controls", "attention", "Workspace", "Input required", now),
    ],
  });

  const card = page.locator(".notification-card");
  await expect(card.locator(".notification-status")).toHaveAttribute("role", "img");
  await expect(card.locator(".notification-status-icon")).toHaveCount(1);
  await expect(card.locator(".notification-kind")).toHaveCount(0);
  await expect(card.locator(".notification-disclosure")).toHaveCount(0);

  for (const [selector, label] of [
    [".notification-activate", "Open Attention notification for Workspace"],
    [".notification-dismiss", "Dismiss Attention notification for Workspace"],
  ]) {
    const button = card.locator(selector);
    await expect(button).toHaveAttribute("aria-label", label);
    await expect(button.locator(".notification-action-icon")).toHaveCount(1);
    expect(await button.textContent()).toBe("");
  }
});

test("notification glass surfaces stay opaque and follow the system color scheme", async ({
  page,
}) => {
  await openNotifications(page);
  await replacePresentation(page, {
    unreadNotificationCount: 1,
    notifications: [
      notification("theme-preview", "completion", "Workspace", "Finished safely", now),
    ],
  });

  const card = page.locator(".notification-card");
  await expect(card.locator(".notification-status")).toHaveCount(0);
  const readTheme = () =>
    card.evaluate((element) => {
      const style = getComputedStyle(element);
      const colorValues = style.backgroundColor.match(/[\d.]+/g) ?? [];
      return {
        backgroundColor: style.backgroundColor,
        backgroundAlpha: colorValues.length >= 4 ? Number(colorValues[3]) : 1,
        foreground: style.color,
        surface: style.getPropertyValue("--notification-surface").trim(),
      };
    });

  await page.emulateMedia({ colorScheme: "light" });
  const light = await readTheme();
  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await readTheme();

  expect(light.surface).toBe("#f2f6fc");
  expect(dark.surface).toBe("#292e38");
  expect(light.backgroundAlpha).toBeGreaterThan(0.9);
  expect(dark.backgroundAlpha).toBeGreaterThan(0.9);
  expect(light.backgroundAlpha).toBeLessThan(1);
  expect(dark.backgroundAlpha).toBeLessThan(1);
  expect(light.foreground).not.toBe(dark.foreground);
});

test("multiple notifications keep two cards visible while scrolling", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = [
    notification("accordion-oldest", "completion", "Alpha", "Oldest notification", now),
    notification("accordion-middle", "failure", "Beta", "Middle notification", now + 100),
    notification("accordion-newest", "attention", "Gamma", "Newest notification", now + 200),
    notification("accordion-latest", "completion", "Delta", "Latest notification", now + 300),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  const scrollStyle = await stack.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      backgroundColor: style.backgroundColor,
      overflow: style.overflow,
      overflowY: style.overflowY,
      scrollWidth: element.scrollWidth,
      clientWidth: element.clientWidth,
      scrollHeight: element.scrollHeight,
      clientHeight: element.clientHeight,
    };
  });
  expect(scrollStyle.backgroundColor).toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
  expect(scrollStyle.overflow).toBe("visible");
  expect(scrollStyle.overflowY).toBe("visible");
  expect(scrollStyle.scrollWidth).toBeLessThanOrEqual(scrollStyle.clientWidth);
  expect(scrollStyle.scrollHeight).toBeLessThanOrEqual(scrollStyle.clientHeight);
  await expect(stack).toHaveAttribute("data-notification-visible-count", "2");
  await expect(stack).toHaveClass(/notification-stack-more-top/);
  await expect(stack).not.toHaveClass(/notification-stack-more-bottom/);

  const visibleCardIds = () =>
    stack.evaluate((element) => {
      const stackBounds = element.getBoundingClientRect();
      const safeTop = stackBounds.top + 12;
      const safeBottom = stackBounds.bottom - 12;
      return [...element.querySelectorAll(".notification-card.notification-card-current")]
        .map((card) => ({
          id: card.dataset.notificationId,
          bounds: card.getBoundingClientRect(),
        }))
        .filter(
          ({ bounds }) =>
            bounds.top >= safeTop - 1 &&
            bounds.bottom <= safeBottom + 1,
        )
        .sort((left, right) => left.bounds.top - right.bounds.top)
        .map(({ id }) => id);
    });

  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-newest", "accordion-latest"]);
  await stack.hover();
  await page.mouse.wheel(0, -20);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-newest", "accordion-latest"]);
  await page.waitForTimeout(180);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-newest", "accordion-latest"]);
  await page.mouse.wheel(0, -120);
  await expect
    .poll(() => stack.evaluate((element) =>
      [...element.querySelectorAll(".notification-card")].reduce((roles, card) => {
        const role = ["top-behind", "top", "bottom", "bottom-behind"]
          .find((value) => card.classList.contains(`notification-card-${value}`)) ?? "hidden";
        roles[card.dataset.notificationId] = {
          role,
          foreground: card.classList.contains("notification-card-foreground"),
        };
        return roles;
      }, {}),
    ))
    .toEqual({
      "accordion-oldest": { role: "top-behind", foreground: false },
      "accordion-middle": { role: "top", foreground: false },
      "accordion-newest": { role: "bottom", foreground: true },
      "accordion-latest": { role: "bottom-behind", foreground: false },
    });
  await expect(stack).toHaveClass(/notification-stack-more-top/);
  await expect(stack).toHaveClass(/notification-stack-more-bottom/);
  await page.waitForTimeout(220);
  const transitionFrame = await stack.evaluate((element) => {
    const readCard = (id) => {
      const card = element.querySelector(`[data-notification-id='${id}']`);
      const bounds = card.getBoundingClientRect();
      const style = getComputedStyle(card);
      return {
        top: bounds.top,
        bottom: bounds.bottom,
        opacity: Number(style.opacity),
        zIndex: style.zIndex,
      };
    };
    return {
      older: readCard("accordion-oldest"),
      shared: readCard("accordion-newest"),
      outgoing: readCard("accordion-latest"),
    };
  });
  expect(transitionFrame.shared.zIndex).toBe("3");
  expect(transitionFrame.older.opacity).toBeGreaterThan(0);
  expect(transitionFrame.older.opacity).toBeLessThan(1);
  expect(transitionFrame.outgoing.opacity).toBeGreaterThan(0);
  expect(transitionFrame.outgoing.opacity).toBeLessThan(1);
  expect(transitionFrame.shared.bottom).toBeGreaterThan(transitionFrame.older.top);
  expect(transitionFrame.older.bottom).toBeGreaterThan(transitionFrame.shared.top);
  expect(transitionFrame.shared.bottom).toBeGreaterThan(transitionFrame.outgoing.top);
  expect(transitionFrame.outgoing.bottom).toBeGreaterThan(transitionFrame.shared.top);
  await page.mouse.wheel(0, -120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-middle", "accordion-newest"]);
  await page.waitForTimeout(320);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-oldest", "accordion-middle"]);
  await page.waitForTimeout(500);
  await page.mouse.wheel(0, -120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-oldest", "accordion-middle"]);
  await expect(stack).not.toHaveClass(/notification-stack-more-top/);
  await expect(stack).toHaveClass(/notification-stack-more-bottom/);
  await page.waitForTimeout(500);
  await page.mouse.wheel(0, 120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-middle", "accordion-newest"]);
  await page.waitForTimeout(500);
  await page.mouse.wheel(0, 120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["accordion-newest", "accordion-latest"]);
});

test("line-mode wheel input advances the notification carousel", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = [
    notification("line-oldest", "completion", "Alpha", "Oldest", now),
    notification("line-middle", "failure", "Beta", "Middle", now + 100),
    notification("line-newest", "attention", "Gamma", "Newest", now + 200),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  const visibleCardIds = () =>
    stack.evaluate((element) =>
      [...element.querySelectorAll(".notification-card.notification-card-current")]
        .sort((left, right) => left.getBoundingClientRect().top - right.getBoundingClientRect().top)
        .map((card) => card.dataset.notificationId),
    );

  await expect
    .poll(visibleCardIds)
    .toEqual(["line-middle", "line-newest"]);
  const defaultPrevented = await stack.evaluate((element) => {
    const event = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      deltaMode: WheelEvent.DOM_DELTA_LINE,
      deltaY: -3,
    });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(defaultPrevented).toBe(true);
  await expect
    .poll(visibleCardIds)
    .toEqual(["line-oldest", "line-middle"]);
});

test("notification reduced-motion changes settle queued transitions", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openNotifications(page);
  const notifications = [
    notification("motion-oldest", "completion", "Alpha", "Oldest", now),
    notification("motion-middle", "failure", "Beta", "Middle", now + 100),
    notification("motion-newest", "attention", "Gamma", "Newest", now + 200),
    notification("motion-latest", "completion", "Delta", "Latest", now + 300),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  const visibleCardIds = () =>
    stack.evaluate((element) =>
      [...element.querySelectorAll(".notification-card.notification-card-current")]
        .sort((left, right) => left.getBoundingClientRect().top - right.getBoundingClientRect().top)
        .map((card) => card.dataset.notificationId),
    );

  await expect
    .poll(visibleCardIds)
    .toEqual(["motion-newest", "motion-latest"]);
  await stack.hover();
  await page.mouse.wheel(0, -120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["motion-middle", "motion-newest"]);
  await page.waitForTimeout(120);
  await page.mouse.wheel(0, -120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["motion-middle", "motion-newest"]);

  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-reduced-motion",
    "true",
  );
  await expect
    .poll(visibleCardIds)
    .toEqual(["motion-oldest", "motion-middle"]);
});

test("presentation reduced motion disables notification transitions", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openNotifications(page);
  const notifications = [
    notification("presentation-motion", "completion", "Workspace", "Done", now),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const card = page.locator(".notification-card");
  await replacePresentation(page, {
    reducedMotion: true,
    unreadNotificationCount: notifications.length,
    notifications,
  });
  await expect(page.locator("#lili-app")).toHaveAttribute(
    "data-reduced-motion",
    "true",
  );
  await expect
    .poll(() => card.evaluate((element) => getComputedStyle(element).transitionDuration))
    .toBe("0s");
});

test("notification carousel remains switchable after the list shrinks", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = [
    notification("shrink-oldest", "completion", "Alpha", "Oldest", now),
    notification("shrink-middle", "failure", "Beta", "Middle", now + 100),
    notification("shrink-newest", "attention", "Gamma", "Newest", now + 200),
    notification("shrink-latest", "completion", "Delta", "Latest", now + 300),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  const visibleCardIds = () =>
    stack.evaluate((element) =>
      [...element.querySelectorAll(".notification-card.notification-card-current")]
        .filter((card) =>
          ["notification-card-top", "notification-card-bottom"].some((role) =>
            card.classList.contains(role),
          ),
        )
        .sort((left, right) => left.getBoundingClientRect().top - right.getBoundingClientRect().top)
        .map((card) => card.dataset.notificationId),
    );

  await expect
    .poll(visibleCardIds)
    .toEqual(["shrink-newest", "shrink-latest"]);
  await replacePresentation(page, {
    unreadNotificationCount: 3,
    notifications: notifications.slice(0, 3),
  });
  await expect
    .poll(visibleCardIds)
    .toEqual(["shrink-middle", "shrink-newest"]);
  await stack.hover();
  await page.mouse.wheel(0, -120);
  await expect
    .poll(visibleCardIds)
    .toEqual(["shrink-oldest", "shrink-middle"]);
});

test("focusing a visible notification does not advance the carousel", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = [
    notification("focus-oldest", "completion", "Alpha", "Oldest", now),
    notification("focus-middle", "failure", "Beta", "Middle", now + 100),
    notification("focus-newest", "attention", "Gamma", "Newest", now + 200),
  ];
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  const visibleCardIds = () =>
    stack.evaluate((element) =>
      [...element.querySelectorAll(".notification-card.notification-card-current")]
        .sort((left, right) => left.getBoundingClientRect().top - right.getBoundingClientRect().top)
        .map((card) => card.dataset.notificationId),
    );

  const currentIds = ["focus-middle", "focus-newest"];
  await expect.poll(visibleCardIds).toEqual(currentIds);
  for (const id of currentIds) {
    const card = page.locator(`[data-notification-id='${id}']`);
    await card.locator(".notification-activate").focus();
    await expect.poll(visibleCardIds).toEqual(currentIds);
    await card.locator(".notification-dismiss").focus();
    await expect.poll(visibleCardIds).toEqual(currentIds);
  }
});

test("each notification can be focused without viewport clipping", async ({
  page,
}) => {
  await openNotifications(page);
  const notifications = Array.from({ length: 6 }, (_, index) =>
    notification(
      `focus-notification-${index}`,
      index % 2 === 0 ? "completion" : "attention",
      `Workspace-${index}`,
      `Notification ${index} is fully readable`,
      now + index,
    ),
  );
  await replacePresentation(page, {
    unreadNotificationCount: notifications.length,
    notifications,
  });

  const stack = page.locator(".notification-stack");
  await expect(stack).toHaveAttribute("data-notification-visible-count", "2");
  const tabStops = await stack
    .locator(".notification-controls button")
    .evaluateAll((buttons) => buttons.map((button) => button.tabIndex));
  expect(tabStops).toHaveLength(12);
  expect(tabStops.every((tabIndex) => tabIndex === 0)).toBe(true);

  for (const current of notifications) {
    const card = page.locator(`[data-notification-id='${current.activationId}']`);
    await card.locator(".notification-activate").focus();
    await expect
      .poll(() => card.locator(".notification-summary").isVisible())
      .toBe(true);
    await expect
      .poll(async () => {
        const bounds = await Promise.all([stack.boundingBox(), card.boundingBox()]);
        return Boolean(
          bounds[0] &&
            bounds[1] &&
            bounds[1].y >= bounds[0].y - 1 &&
            bounds[1].y + bounds[1].height <= bounds[0].y + bounds[0].height + 1,
        );
      })
      .toBe(true);

    const bounds = await Promise.all([stack.boundingBox(), card.boundingBox()]);
    expect(bounds[0]).not.toBeNull();
    expect(bounds[1]).not.toBeNull();
    expect(bounds[1].y).toBeGreaterThanOrEqual(bounds[0].y - 1);
    expect(bounds[1].y + bounds[1].height).toBeLessThanOrEqual(
      bounds[0].y + bounds[0].height + 1,
    );
  }
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
  const visualOrder = await page
    .locator(".notification-card.notification-card-current")
    .evaluateAll((cards) =>
    cards
      .map((card) => ({
        id: card.dataset.notificationId,
        top: card.getBoundingClientRect().top,
      }))
      .sort((left, right) => left.top - right.top)
      .map((card) => card.id),
  );
  expect(visualOrder).toEqual(["failure-middle", "completion-new"]);
  const viewportHeight = await page.evaluate(() => window.innerHeight);
  await expect
    .poll(() =>
      page
        .locator("[data-notification-id='completion-new']")
        .evaluate((card) => card.getBoundingClientRect().bottom),
    )
    .toBe(viewportHeight - 16);
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

  const stack = page.locator(".notification-stack");
  await expect(stack).toHaveAttribute("data-notification-visible-count", "2");
  const bottomMost = () => page
    .locator(".notification-card.notification-card-current")
    .evaluateAll((cards) =>
      cards
        .map((card) => ({
          id: card.dataset.notificationId,
          bottom: card.getBoundingClientRect().bottom,
        }))
        .sort((left, right) => right.bottom - left.bottom)[0].id,
    );
  await expect.poll(bottomMost).toBe("completion-new");
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
  expect(cardTop).toBe(16);
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
