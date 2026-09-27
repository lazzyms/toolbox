import { expect, test } from "@playwright/test";

test("design-system primitives expose accessible controls and working states", async ({ page }) => {
  await page.goto("/tests/fixtures/design-system.html");
  await expect(page.getByRole("heading", { name: "Design system preview" })).toBeVisible();

  const save = page.getByRole("button", { name: "Save changes" });
  const disabledButton = page.getByRole("button", { name: "Disabled action" });
  await expect(disabledButton).toBeDisabled();
  await page.keyboard.press("Tab");
  await expect(save).toBeFocused();
  await expect(save).toHaveCSS("outline-style", "solid");
  const restingButtonColor = await save.evaluate((node) => getComputedStyle(node).backgroundColor);
  await save.hover();
  await expect.poll(() => save.evaluate((node) => getComputedStyle(node).backgroundColor))
    .not.toBe(restingButtonColor);
  const hoverButtonColor = await save.evaluate((node) => getComputedStyle(node).backgroundColor);
  await page.mouse.down();
  await expect.poll(() => save.evaluate((node) => getComputedStyle(node).backgroundColor))
    .not.toBe(hoverButtonColor);
  await page.mouse.up();
  await expect(page.locator("footer[role='status']")).toContainText("Saved");
  const dangerButton = page.getByRole("button", { name: "Delete item" });
  const dangerColor = await dangerButton.evaluate((node) => getComputedStyle(node).backgroundColor);
  await dangerButton.hover();
  await expect.poll(() => dangerButton.evaluate((node) => getComputedStyle(node).backgroundColor))
    .not.toBe(dangerColor);

  const close = page.getByRole("button", { name: "Close preview" });
  await expect(page.getByRole("button", { name: "Disabled icon" })).toBeDisabled();
  const closeColor = await close.evaluate((node) => getComputedStyle(node).color);
  await close.hover();
  await expect.poll(() => close.evaluate((node) => getComputedStyle(node).color))
    .not.toBe(closeColor);
  await close.click();
  await expect(page.locator("footer[role='status']")).toContainText("Draft");

  const quality = page.getByRole("slider", { name: "Quality", exact: true });
  await expect(quality).toHaveValue("70");
  const sliderAccent = await quality.evaluate((node) => getComputedStyle(node).accentColor);
  await quality.hover();
  await expect.poll(() => quality.evaluate((node) => getComputedStyle(node).accentColor))
    .not.toBe(sliderAccent);
  await page.keyboard.press("Tab");
  await expect(quality).toBeFocused();
  await expect(quality).toHaveCSS("outline-style", "solid");
  await quality.press("ArrowRight");
  await expect(quality).toHaveValue("71");
  await expect(page.getByRole("slider", { name: "Disabled quality" })).toBeDisabled();

  const actionSwatch = page.getByRole("group", { name: "Annotation color" })
    .getByRole("button", { name: "Action blue" });
  const dangerSwatch = page.getByRole("button", { name: "Danger red" });
  await page.keyboard.press("Tab");
  await expect(actionSwatch).toBeFocused();
  await expect(actionSwatch).toHaveCSS("outline-style", "solid");
  await page.keyboard.press("Tab");
  await expect(dangerSwatch).toBeFocused();
  await expect(dangerSwatch).toHaveCSS("outline-style", "solid");
  await dangerSwatch.click();
  await expect(dangerSwatch).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("group", { name: "Disabled colors" }).getByRole("button"))
    .toBeDisabled();

  const metadata = page.getByRole("switch", { name: "Keep metadata" });
  const toggleBorder = await metadata.locator(".ds-toggle__track")
    .evaluate((node) => getComputedStyle(node).borderColor);
  await metadata.hover();
  await expect.poll(() => metadata.locator(".ds-toggle__track")
    .evaluate((node) => getComputedStyle(node).borderColor)).not.toBe(toggleBorder);
  await metadata.click();
  await expect(metadata).toHaveAttribute("aria-checked", "true");
  await expect(page.getByRole("switch", { name: "Disabled metadata" })).toBeDisabled();

  const landscape = page.getByRole("radio", { name: "Landscape" });
  await landscape.click();
  await expect(landscape).toHaveAttribute("aria-checked", "true");
  await landscape.press("ArrowRight");
  await expect(page.getByRole("radio", { name: "Portrait" })).toHaveAttribute("aria-checked", "true");
  await expect(page.getByRole("radio", { name: "Grid" })).toBeDisabled();

  const projectName = page.getByRole("textbox", { name: "Project name" });
  const fieldBorder = await projectName.locator("..").evaluate((node) => getComputedStyle(node).borderColor);
  await projectName.hover();
  await expect.poll(() => projectName.locator("..").evaluate((node) => getComputedStyle(node).borderColor))
    .not.toBe(fieldBorder);
  await projectName.fill("Launch kit");
  await expect(page.getByRole("textbox", { name: "Project name" })).toHaveValue("Launch kit");
  await expect(page.getByRole("textbox", { name: "Disabled field" })).toBeDisabled();
  await page.getByRole("spinbutton", { name: "Copies", exact: true }).fill("3");
  await expect(page.getByRole("spinbutton", { name: "Copies", exact: true })).toHaveValue("3");
  await expect(page.getByRole("spinbutton", { name: "Disabled copies" })).toBeDisabled();
  await page.getByRole("combobox", { name: "Format", exact: true }).selectOption("webp");
  await expect(page.getByRole("combobox", { name: "Format", exact: true })).toHaveValue("webp");
  await expect(page.getByRole("combobox", { name: "Disabled format" })).toBeDisabled();

  const help = page.getByRole("button", { name: "Help" });
  await help.focus();
  await expect(page.getByRole("tooltip")).toBeVisible();
  await expect(page.getByRole("button", { name: "Undo" }).locator(".." )).toHaveAttribute("role", "toolbar");
  const card = page.getByRole("article", { name: "Example card" });
  await expect(card).toContainText("Reusable card");
  await page.getByRole("button", { name: "Choose file" }).click();
  await expect(page.locator("footer[role='status']")).toContainText("File selected");

  await expect(page.getByRole("img", { name: "Example thumbnail" })).toBeVisible();
  await expect(page.getByText("Ready", { exact: true })).toBeVisible();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();
});

test("toolbar has one tab stop and arrow navigation skips disabled buttons", async ({ page }) => {
  await page.goto("/tests/fixtures/design-system.html");
  const toolbar = page.getByRole("toolbar", { name: "Edit controls" });
  const undo = toolbar.getByRole("button", { name: "Undo" });
  const redo = toolbar.getByRole("button", { name: "Redo" });
  const copy = toolbar.getByRole("button", { name: "Copy" });

  await expect(undo).toHaveAttribute("tabindex", "0");
  await expect(redo).toBeDisabled();
  await expect(copy).toHaveAttribute("tabindex", "-1");
  await undo.focus();
  await undo.press("ArrowRight");
  await expect(copy).toBeFocused();
  await expect(undo).toHaveAttribute("tabindex", "-1");
  await expect(copy).toHaveAttribute("tabindex", "0");
  await copy.press("ArrowLeft");
  await expect(undo).toBeFocused();
});

test("normal text tokens reach 4.5 to 1 contrast across semantic surfaces in both palettes", async ({ page }) => {
  await page.goto("/tests/fixtures/design-system.html");
  const contrastFor = (theme: "dark" | "light") => page.evaluate((nextTheme) => {
    document.body.dataset.theme = nextTheme;
    const channels = (color: string): number[] => {
      const value = color.trim();
      if (value.startsWith("#")) {
        const hex = value.slice(1);
        if (hex.length === 3) return [...hex].map((channel) => parseInt(channel + channel, 16));
        return hex.match(/.{2}/g)?.slice(0, 3).map((channel) => parseInt(channel, 16)) ?? [];
      }
      return value.match(/[\d.]+/g)?.slice(0, 3).map(Number) ?? [];
    };
    const luminance = (color: string) => {
      const [red, green, blue] = channels(color).map((channel) => {
        const normalized = channel / 255;
        return normalized <= 0.04045
          ? normalized / 12.92
          : ((normalized + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    };
    const styles = getComputedStyle(document.body);
    return ["--color-text-secondary", "--color-text-muted"].flatMap((token) => {
      const foreground = luminance(styles.getPropertyValue(token));
      return ["--color-canvas", "--color-rail", "--color-panel", "--color-card", "--color-overlay"]
        .map((surface) => {
        const background = luminance(styles.getPropertyValue(surface));
        return {
          token,
          surface,
          ratio: (Math.max(foreground, background) + 0.05) /
            (Math.min(foreground, background) + 0.05),
        };
        });
    });
  }, theme);

  for (const theme of ["dark", "light"] as const) {
    for (const { token, surface, ratio } of await contrastFor(theme)) {
      expect(ratio, `${theme} ${token} on ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  }
});
