import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import path from "node:path";
import { UtilityRegistry } from "../../src/registry";

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const fixtureName = path.basename(fixturePath);
const appVersion = (JSON.parse(readFileSync(path.resolve("package.json"), "utf8")) as { version: string }).version;

type TestWindow = Window & {
    __toolboxInvocations?: Array<{ command: string; args: unknown }>;
};

test.beforeEach(async ({ page }) => {
    await page.addInitScript(({ fixturePath }) => {
        const invocations: Array<{ command: string; args: unknown }> = [];
        (window as TestWindow).__toolboxInvocations = invocations;

        window.__TAURI_INTERNALS__ = {
            metadata: {
                currentWindow: { label: "main" },
                currentWebview: { label: "main", windowLabel: "main" },
            },
            invoke: async (command, args) => {
                invocations.push({ command, args });

                if (command === "plugin:dialog|open") return fixturePath;
                if (command === "inspect_pdf") {
                    return { pages: [{ index: 0, width: 612, height: 792 }] };
                }
                if (command === "inspect_image_metadata") return ["fixture image"];
                if (command.startsWith("plugin:")) return null;

                return [{
                    inputPath: fixturePath,
                    outputPaths: [`${fixturePath}.output`],
                    detail: "Test output",
                    failure: null,
                }];
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
            unregisterListener: () => undefined,
        };
    }, { fixturePath });
});

test("every registered feature opens its detail pane", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Ready to process" })).toBeVisible();

    for (const utility of UtilityRegistry) {
        const navigationButton = page.getByRole("button", {
            name: `${utility.title}: ${utility.blurb}`,
        });

        await expect(navigationButton).toBeVisible();
        await navigationButton.click();
        await expect(page.getByRole("heading", { name: utility.title, exact: true })).toBeVisible();
        await expect(page.getByRole("status")).toHaveText(`${utility.title} selected.`);
    }
});

test("sidebar navigation has no decorative icons", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator('aside[aria-label="Toolbox navigation"] [aria-hidden="true"]')).toHaveCount(0);
});

test("sidebar and detail pane scroll independently", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator('nav[aria-label="Utilities"]')).toHaveCSS("overflow-y", "auto");
    await expect(page.locator('main[aria-label="Tool detail"]')).toHaveCSS("overflow-y", "auto");
});

test("sidebar does not show the engine badge", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("Unified Native Engine", { exact: true })).toHaveCount(0);
});

test("settings shows app info and the donation QR code", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings" }).click();

    const settings = page.getByRole("dialog", { name: "Settings" });
    await expect(settings).toBeVisible();
    await expect(settings.getByText("App info", { exact: true })).toBeVisible();
    await expect(settings.getByText("Toolbox", { exact: true })).toBeVisible();
    await expect(settings.getByText(`Version ${appVersion}`, { exact: true })).toBeVisible();
    await expect(settings.getByRole("img", { name: "Buy Me a Coffee donation QR code" })).toBeVisible();

    await settings.getByRole("button", { name: "Close settings" }).click();
    await expect(settings).toBeHidden();
});

const exerciseFeature = async (page: Page, utility: (typeof UtilityRegistry)[number]) => {
    await page.goto("/");
    await page.getByRole("button", {
        name: `${utility.title}: ${utility.blurb}`,
    }).click();

    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(fixtureName, { exact: true })).toBeVisible();

    if (utility.id === "pdf-unlock" || utility.id === "pdf-protect") {
        await page.locator('input[type="password"]').fill("test-password");
    }

    const action = page.locator("main button").filter({ hasText: utility.shortTitle }).last();
    await expect(action).toBeEnabled();
    await action.click();

    await expect(page.getByText("Test output", { exact: true })).toBeVisible();
    const invocations = await page.evaluate(() => (window as TestWindow).__toolboxInvocations ?? []);
    expect(invocations.some(({ command }) => command === utility.command)).toBe(true);
};

test.describe("registered feature actions", () => {
    expect(UtilityRegistry).toHaveLength(31);

    for (const utility of UtilityRegistry) {
        test(`${utility.id} accepts the fixture and runs`, async ({ page }) => {
            await exerciseFeature(page, utility);
        });
    }
});
