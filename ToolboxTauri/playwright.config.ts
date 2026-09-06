import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
    testDir: "./tests/ui",
    fullyParallel: true,
    reporter: "list",
    use: {
        baseURL: "http://127.0.0.1:1420",
        ...devices["Desktop Chrome"],
    },
    webServer: {
        command: "npm run dev -- --host 127.0.0.1",
        url: "http://127.0.0.1:1420",
        reuseExistingServer: false,
        timeout: 120_000,
    },
});
