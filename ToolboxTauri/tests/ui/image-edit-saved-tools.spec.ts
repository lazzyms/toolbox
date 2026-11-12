import { expect, test } from "@playwright/test";
import { RetiredToolIds } from "../../src/registry/retiredToolIds";
import { UtilityRegistry } from "../../src/registry";

// The six Edit Image tools kept their ids when their own commands were retired, so saved
// favorites and recents need no migration: each saved id must still open its stack step.
const savedTools = [
    { id: "crop", step: "Crop" },
    { id: "resize", step: "Resize" },
    { id: "rotate", step: "Rotate and Flip" },
    { id: "image-tone", step: "Colour and Tone" },
    { id: "image-watermark", step: "Watermark" },
    { id: "image-metadata", step: "Remove Image Metadata" },
] as const;
const savedIds = savedTools.map(({ id }) => id);
const retiredCommands = ["crop_images", "resize_images", "rotate_images", "adjust_image_tone", "watermark_images", "image_metadata"];

type SavedWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[]>;
};

const svg = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='480'/%3E";

for (const theme of ["light", "dark"] as const) {
    test(`saved Edit Image favorites and recents open their own step in the ${theme} theme`, async ({ page }) => {
        await page.addInitScript(({ blank, theme, saved }) => {
            const win = window as unknown as SavedWindow;
            win.__invocations = [];
            win.__dialogResults = [];
            localStorage.setItem("toolbox-theme", theme);
            // Seed once, as a person upgrading would have them; later loads keep what the app wrote.
            if (localStorage.getItem("toolbox-tool-id-migration") === null) {
                localStorage.setItem("toolbox-favorites", JSON.stringify(saved));
                localStorage.setItem("toolbox-recent", JSON.stringify([...saved].reverse()));
            }
            window.__TAURI_INTERNALS__ = {
                metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
                invoke: async (command: string, args: unknown) => {
                    win.__invocations.push({ command, args });
                    if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                    if (command === "get_last_save_folder") return "/Users/test/Documents";
                    if (command === "check_output_folder") return "ready";
                    if (command === "get_quick_actions") return [];
                    if (command === "plugin:app|version") return "9.4.2";
                    if (command === "plugin:path|resolve_directory") return "/Users/test";
                    if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                    if (command === "inspect_image_preview" || command === "inspect_image_edit_preview") return { width: 640, height: 480, dataUrl: blank };
                    if (command === "inspect_image_metadata") return [];
                    return null;
                },
                transformCallback: () => 1,
                unregisterCallback: () => undefined,
            };
            window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
        }, { blank: svg, theme, saved: savedIds });

        const steps = page.getByRole("list", { name: "Steps, in the order they run" });
        for (const { id, step } of savedTools) {
            const title = UtilityRegistry.find((tool) => tool.id === id)!.title;
            await page.goto("/");
            await expect(page.locator("body")).toHaveAttribute("data-theme", theme);
            const chips = page.getByRole("group", { name: "Favorite tools" });
            await expect(chips.getByRole("button")).toHaveText(savedTools.map(({ id }) => UtilityRegistry.find((tool) => tool.id === id)!.title));
            await chips.getByRole("button", { name: title, exact: true }).click();

            await page.evaluate(() => { (window as unknown as SavedWindow).__dialogResults = [["/photos/a.png"]]; });
            await page.getByRole("button", { name: "Choose files to process" }).click();
            await expect(steps.getByRole("button", { name: `Step 1 of 1, ${step}, showing its options` })).toHaveAttribute("aria-pressed", "true");

            // Opening from a saved id records that same id as the most recent tool, and nothing
            // reaches the retired per-tool commands.
            const opened = await page.evaluate(() => ({
                recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "[]") as string[],
                commands: (window as unknown as SavedWindow).__invocations.map(({ command }) => command),
            }));
            expect(opened.recent[0]).toBe(id);
            expect(opened.commands.filter((command) => retiredCommands.includes(command))).toEqual([]);
        }

        const stored = await page.evaluate(() => ({
            favorites: JSON.parse(localStorage.getItem("toolbox-favorites") ?? "null") as string[],
            recent: JSON.parse(localStorage.getItem("toolbox-recent") ?? "null") as string[],
            marker: localStorage.getItem("toolbox-tool-id-migration"),
        }));
        expect(stored.favorites).toEqual(savedIds);
        expect([...stored.recent].sort()).toEqual([...savedIds].sort());
        expect(stored.marker).toBe(String(RetiredToolIds.version));
        for (const id of stored.recent) {
            expect(UtilityRegistry.find((tool) => tool.id === id)?.command, `${id} still runs through the stack`).toBe("export_image_edit_stack");
        }
    });
}
