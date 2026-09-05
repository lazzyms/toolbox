import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { UtilityRegistry } from "../../src/registry";

const fixturePath = path.resolve("src-tauri/icons/icon.png");
const fixtureName = path.basename(fixturePath);

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
                    return { pages: [{ index: 0, x: 0, y: 0, width: 612, height: 792, preview: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='612' height='792'%3E%3Crect width='100%25' height='100%25' fill='white'/%3E%3C/svg%3E" }] };
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

test("crop stays disabled until a crop rectangle is drawn", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", {
        name: "Crop PDF: Hide content outside a selected page rectangle.",
    }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByText(fixtureName, { exact: true })).toBeVisible();

    const cropAction = page.getByRole("main").getByRole("button", { name: "Crop", exact: true });
    await expect(cropAction).toBeDisabled();

    const preview = page.getByLabel("Preview of page 1");
    const box = await preview.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.8, box.y + box.height * 0.8);
    await page.mouse.up();

    await expect(cropAction).toBeEnabled();
});

test("pdf editor renders page previews", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", {
        name: "Sign PDF: Place a visible typed signature on a PDF page.",
    }).click();
    await page.getByRole("button", { name: "Choose files to process" }).click();
    await expect(page.getByRole("img", { name: "Preview of page 1" })).toBeVisible();
    await expect(page.locator('aside[aria-label="PDF page thumbnails"] img[alt="Thumbnail of page 1"]')).toBeVisible();
});

test("vision tools explain unavailable resources before file selection", async ({ page }) => {
    await page.goto("/");
    for (const id of ["pdf-ocr", "image-blur-faces", "image-remove-bg"]) {
        const utility = UtilityRegistry.find((item) => item.id === id);
        expect(utility).toBeDefined();
        if (!utility) continue;
        await page.getByRole("button", { name: `${utility.title}: ${utility.blurb}` }).click();
        await expect(page.getByText("Unavailable in this build.", { exact: true })).toBeVisible();
        await expect(page.getByRole("button", { name: "Choose files to process" })).toHaveCount(0);
    }
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
            test.skip(utility.status !== "implemented", "Tool is not available in this build.");
            await exerciseFeature(page, utility);
        });
    }
});
