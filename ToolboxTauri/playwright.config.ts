import { defineConfig, devices } from "@playwright/test";

const port = process.env.TOOLBOX_PLAYWRIGHT_PORT || "1420";
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
    testDir: "./tests/ui",
    fullyParallel: true,
    reporter: "list",
    use: {
        baseURL,
        ...devices["Desktop Chrome"],
    },
    webServer: {
        command: `npm run dev -- --host 127.0.0.1 --port ${port}`,
        url: baseURL,
        reuseExistingServer: false,
        timeout: 120_000,
    },
});
