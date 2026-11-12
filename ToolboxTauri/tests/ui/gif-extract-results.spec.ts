import { expect, test, type Page } from "@playwright/test";
import type { JobOutcome } from "../../src/contracts";
import { openToolBySearch } from "./open-tool";

// Extract GIF Frames runs a batch of GIFs and shows each GIF's frames and timings file as one grouped row.

type ExtractWindow = Window & {
    __invocations: Array<{ command: string; args: unknown }>;
    __dialogResults: Array<string | string[] | null>;
};

const spinFrames = ["/local/gifs/spin-frame-001.png", "/local/gifs/spin-frame-002.png", "/local/gifs/spin-frame-003.png"];

const outcomes: JobOutcome[] = [
    {
        inputPath: "/local/gifs/spin.gif",
        outputPaths: [...spinFrames, "/local/gifs/spin-timings.json"],
        detail: "3 PNGs, 320 × 320",
        state: "succeeded",
        failure: null,
        reason: null,
        warnings: [],
    },
    {
        inputPath: "/local/gifs/party.gif",
        outputPaths: [],
        detail: "",
        state: "failed",
        failure: { kind: "invalidInput", message: "Frame 31 is damaged, so the GIF can't be read past it. No frames were saved for this GIF.", pointer: null },
        reason: null,
        warnings: [],
    },
    {
        inputPath: "/local/gifs/logo-still.gif",
        outputPaths: ["/local/gifs/logo-still-frame-001.png", "/local/gifs/logo-still-timings.json"],
        detail: "1 PNG, 512 × 512",
        state: "succeeded",
        failure: null,
        reason: null,
        warnings: [],
    },
];

const start = async (page: Page) => {
    await page.addInitScript((outcomes) => {
        const win = window as unknown as ExtractWindow;
        win.__invocations = [];
        win.__dialogResults = [];
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
        window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
            invoke: async (command: string, args: unknown) => {
                win.__invocations.push({ command, args });
                if (command === "get_quick_actions") return [];
                if (command === "get_output_settings") return { mode: "alongsideInput", folder: "/Users/test/Documents/Toolbox" };
                if (command === "check_output_folder") return "ready";
                if (command === "plugin:dialog|open") return win.__dialogResults.shift() ?? null;
                if (command === "plugin:path|resolve_directory") return "/Users/test";
                if (command === "extract_gif_frames") return outcomes;
                return null;
            },
            transformCallback: () => 1,
            unregisterCallback: () => undefined,
        };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    }, outcomes);
    await page.goto("/");
    await openToolBySearch(page, "Extract GIF Frames");
};

const choose = async (page: Page, files: string[]) => {
    await page.evaluate((files) => { (window as unknown as ExtractWindow).__dialogResults.push(files); }, files);
    await page.getByRole("button", { name: "Choose files to process" }).click();
};

const invocationsOf = (page: Page, command: string) => page.evaluate((command) =>
    (window as unknown as ExtractWindow).__invocations.filter((invocation) => invocation.command === command).map(({ args }) => args),
command);

const gifs = ["/local/gifs/spin.gif", "/local/gifs/party.gif", "/local/gifs/logo-still.gif"];

const run = async (page: Page) => {
    await choose(page, gifs);
    await page.locator(".workspace-primary-action").click();
    await expect(page.getByRole("list", { name: "Results" })).toBeVisible();
};

test.describe("Extract GIF Frames", () => {
    test("describes the frames and timings file each GIF makes, with example names", async ({ page }) => {
        await start(page);
        const outputs = page.getByRole("region", { name: "You get, for each GIF" });
        await expect(outputs).toContainText("Every frame as a full PNG, numbered so it sorts in order.");
        await expect(outputs).toContainText("each PNG looks like the frame on screen");
        await expect(outputs).toContainText("One timings file that lists each frame file and its delay in milliseconds.");

        await choose(page, gifs);
        const example = page.getByTestId("run-destination-example");
        await expect(example).toContainText("spin-frame-001.png");
        await expect(example).toContainText("spin-timings.json");
    });

    test("sends every GIF path in one request", async ({ page }) => {
        await start(page);
        await run(page);
        expect(await invocationsOf(page, "extract_gif_frames")).toEqual([{ runId: expect.any(String), request: { paths: gifs, outputLocation: "alongsideInput" } }]);
    });

    test("groups each GIF's outputs with Open and Reveal, and shows a failed GIF beside the others", async ({ page }) => {
        await start(page);
        await run(page);

        const summary = page.locator(".result-summary");
        await expect(summary).toContainText("2 extracted, 1 failed");
        await expect(summary).toContainText("4 frames saved.");

        const results = page.getByRole("list", { name: "Results" });
        const spin = results.getByRole("listitem").filter({ hasText: "spin.gif" });
        await expect(spin).toContainText("3 frames and a timings file");
        await expect(spin).toContainText("Extracted");
        const still = results.getByRole("listitem").filter({ hasText: "logo-still.gif" });
        await expect(still).toContainText("1 frame and a timings file. This GIF isn't animated.");
        const party = results.getByRole("listitem").filter({ hasText: "party.gif" });
        await expect(party).toContainText("Frame 31 is damaged, so the GIF can't be read past it. No frames were saved for this GIF.");
        await expect(party).toContainText("Failed");
        await expect(party.getByRole("button")).toHaveCount(0);

        // A GIF's own row opens and reveals its timings file.
        await spin.getByRole("button", { name: "Open spin-timings.json" }).click();
        await spin.getByRole("button", { name: "Show in Finder: spin-timings.json" }).click();
        await expect.poll(() => invocationsOf(page, "open_output_path")).toEqual([{ path: "/local/gifs/spin-timings.json" }]);
        await expect.poll(() => invocationsOf(page, "reveal_output_path")).toEqual([{ path: "/local/gifs/spin-timings.json" }]);

        // Expanding shows the timings file and the numbered frames as their own rows.
        await expect(results.getByText("spin-frame-001.png to -003.png")).toHaveCount(0);
        const toggle = page.getByRole("button", { name: "Show outputs of spin.gif" });
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await toggle.click();
        await expect(page.getByRole("button", { name: "Hide outputs of spin.gif" })).toHaveAttribute("aria-expanded", "true");

        const timings = results.getByRole("listitem").filter({ hasText: "Each frame file and its delay" });
        await expect(timings).toContainText("spin-timings.json");
        const frames = results.getByRole("listitem").filter({ hasText: "spin-frame-001.png to -003.png" });
        await expect(frames).toContainText("3 PNGs, 320 × 320");
        await expect(frames.getByRole("button", { name: /^Open/ })).toHaveCount(0);
        await frames.getByRole("button", { name: "Show in Finder: spin-frame-001.png to -003.png" }).click();
        await expect.poll(async () => (await invocationsOf(page, "reveal_output_path")).at(-1)).toEqual({ path: spinFrames[0] });
        await timings.getByRole("button", { name: "Open spin-timings.json" }).click();
        await expect.poll(async () => (await invocationsOf(page, "open_output_path")).length).toBe(2);

        // The failed GIF has nothing to expand, and only the expanded GIF shows its parts.
        await expect(page.getByRole("button", { name: "Show outputs of party.gif" })).toHaveCount(0);
        await expect(results.getByText("logo-still-frame-001.png")).toHaveCount(0);
        await page.getByRole("button", { name: "Hide outputs of spin.gif" }).click();
        await expect(frames).toHaveCount(0);
    });
});
