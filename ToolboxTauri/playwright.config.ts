import { defineConfig, devices } from "@playwright/test";

const port = process.env.TOOLBOX_PLAYWRIGHT_PORT || "1420";
const baseURL = `http://127.0.0.1:${port}`;
const productionPreview = process.env.TOOLBOX_UI_PRODUCTION === "1";
const flagPlatform = process.env.TOOLBOX_EXPECTED_PLATFORM;
const outputDir = flagPlatform === "macos" || flagPlatform === "windows"
    ? `test-results/tool-flags-${flagPlatform}`
    : "test-results";

export default defineConfig({
    testDir: "./tests/ui",
    outputDir,
    fullyParallel: true,
    reporter: "list",
    use: {
        baseURL,
        ...devices["Desktop Chrome"],
    },
    webServer: {
        command: productionPreview
            ? `npm run preview -- --host 127.0.0.1 --port ${port}`
            : `npm run dev -- --host 127.0.0.1 --port ${port}`,
        url: baseURL,
        reuseExistingServer: false,
        timeout: 120_000,
    },
});
