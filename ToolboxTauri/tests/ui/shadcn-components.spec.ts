import { expect, test } from "@playwright/test";

test("desktop shell uses neutral shadcn controls in both themes", async ({ page }, testInfo) => {
  await page.goto("/");

  const body = page.locator("body");
  const buttons = page.locator("button");
  await expect(buttons.first()).toHaveAttribute("data-slot", "button");
  await expect(page.locator('[class*="ds-"]')).toHaveCount(0);
  await expect(page.locator("button:not([data-slot])")).toHaveCount(0);

  for (const theme of ["light", "dark"] as const) {
    await body.evaluate((element, nextTheme) => {
      element.setAttribute("data-theme", nextTheme);
    }, theme);
    await page.evaluate(async () => {
      const transitions = document.getAnimations().filter((animation) =>
        animation.playState === "running" && Number.isFinite(animation.effect?.getComputedTiming().endTime),
      );
      await Promise.all(transitions.map((animation) => animation.finished.catch(() => undefined)));
    });

    const palette = await page.evaluate(() => {
      const styles = getComputedStyle(document.body);
      return ["--background", "--primary", "--accent", "--border"].map((token) => ({
        token,
        value: styles.getPropertyValue(token).trim(),
      }));
    });

    for (const { token, value } of palette) {
      const neutralOklch = value.match(/oklch\(\s*[\d.]+\s+([\d.]+)\s+[\d.]+(?:\s*\/\s*[\d.]+%?)?\s*\)/);
      expect(neutralOklch, `${theme} ${token} uses neutral OKLCH tokens`).not.toBeNull();
      expect(Number(neutralOklch?.[1]), `${theme} ${token} chroma`).toBe(0);
    }

    const cardSurface = await page.locator(".tool-card").first().evaluate((element) => ({
      background: getComputedStyle(element).backgroundColor,
      token: getComputedStyle(document.body).getPropertyValue("--card").trim(),
      borderToken: getComputedStyle(document.body).getPropertyValue("--border").trim(),
    }));
    const normalizedBorderToken = cardSurface.borderToken.replace(/\/\s*([\d.]+)%/, (_, percent) => `/ ${Number(percent) / 100}`);
    await expect.poll(async () =>
      page.locator(".tool-card").first().evaluate((element) => getComputedStyle(element).backgroundColor),
    ).toBe(cardSurface.token);
    await expect.poll(async () =>
      page.locator(".tool-card").first().evaluate((element) => getComputedStyle(element).borderColor),
    ).toBe(normalizedBorderToken);

    await page.screenshot({ path: testInfo.outputPath(`toolbox-${theme}.png`), fullPage: false });
  }

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings" });
  await expect(settings).toBeVisible();
  await expect(settings.locator('[class*="ds-"]')).toHaveCount(0);
  await expect(settings.locator("button:not([data-slot])")).toHaveCount(0);
});

test("feature panels and previews use shadcn Card surfaces", async ({ page }) => {
  await page.goto("/");

  const surfacesByTool = [
    { tool: "Protect PDF", selectors: [".workspace-control-panel", ".workspace-command-rail"] },
    { tool: "Create GIF", selectors: [".workspace-control-panel", ".workspace-command-rail"] },
    { tool: "PDF to Images", selectors: [".workspace-control-panel", ".workspace-command-rail", ".conversion-preview"] },
    { tool: "Resize Images", selectors: [".workspace-command-rail", ".image-editor-controls", ".image-editor-preview"] },
    { tool: "Edit PDF", selectors: [".scene-empty"] },
  ];

  for (const { tool, selectors } of surfacesByTool) {
    await page.goto("/");
    await page.getByRole("button", { name: `Open ${tool}`, exact: true }).click();
    for (const selector of selectors) {
      await expect(page.locator(selector)).toHaveAttribute("data-slot", "card");
    }
  }

  await page.goto("/");
  await page.getByRole("button", { name: "Open Edit PDF", exact: true }).click();
  const emptyScene = page.locator(".scene-empty");
  await expect(emptyScene.locator('[data-slot="card-header"]')).toHaveCount(1);
  await expect(emptyScene.getByRole("heading", { name: "Open a PDF to edit" })).toBeVisible();
  await expect(emptyScene.locator('[data-slot="card-description"]')).toHaveCount(1);
});

test("shared cards keep generated anatomy and variants own selected appearance", async ({ page }) => {
  await page.goto("/");

  const toolCard = page.locator(".tool-card").first();
  await expect.poll(() => toolCard.evaluate((element) => getComputedStyle(element).borderRadius)).toBe("14px");
  await expect(toolCard.locator('[data-slot="card-header"]')).toHaveCount(1);
  await expect(toolCard.locator('[data-slot="card-footer"]')).toHaveCount(1);

  await page.getByRole("button", { name: "Open Protect PDF", exact: true }).click();

  const sourceBar = page.locator(".workspace-source-bar");
  await expect.poll(() => sourceBar.evaluate((element) => getComputedStyle(element).borderRadius)).toBe("14px");
  await expect(sourceBar.locator('[data-slot="card-content"]')).toHaveCount(1);

  const commandRail = page.locator(".workspace-command-rail");
  await expect(commandRail.locator('[data-slot="card-header"]')).toHaveCount(1);
  await expect(commandRail.locator('[data-slot="card-content"]')).toHaveCount(1);
  await expect(commandRail.locator('.workspace-command[aria-pressed="true"]')).toHaveAttribute("data-variant", "secondary");
});
