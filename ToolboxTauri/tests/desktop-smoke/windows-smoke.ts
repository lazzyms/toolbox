import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { basename, join, relative, resolve } from "node:path";
import { Builder, By, until, type WebDriver, type WebElement } from "selenium-webdriver";
import { ToolWorkspaceRegistry } from "../../src/registry";
import type { WorkspaceId } from "../../src/contracts";

type WorkspaceEvidence =
  | { workspaceId: WorkspaceId; title: string; status: "not-run" }
  | { workspaceId: WorkspaceId; title: string; status: "passed"; screenshot: string }
  | { workspaceId: WorkspaceId; title: string; status: "failed"; error: string; screenshot?: string };

type DesktopSmokeReport = {
  schemaVersion: 1;
  platform: "windows";
  executable: string;
  startedAt: string;
  finishedAt?: string;
  status: "running" | "passed" | "failed";
  homeScreenshot?: string;
  workspaces: WorkspaceEvidence[];
  setupError?: string;
};

const outputDirectory = resolve(process.env.RUNNER_TEMP ?? process.env.TEMP ?? ".", "toolbox-desktop-smoke");
const reportPath = join(outputDirectory, "desktop-smoke-report.json");
const executable = resolve("src-tauri/target/debug/toolbox.exe");
const report: DesktopSmokeReport = {
  schemaVersion: 1,
  platform: "windows",
  executable,
  startedAt: new Date().toISOString(),
  status: "running",
  workspaces: ToolWorkspaceRegistry.map(({ id, title }) => ({ workspaceId: id, title, status: "not-run" })),
};

let driverProcess: ChildProcess | undefined;
let driver: WebDriver | undefined;
let driverStartError: Error | undefined;

async function persistReport(): Promise<void> {
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
}

async function captureScreenshot(name: string): Promise<string> {
  if (!driver) throw new Error("WebDriver session is unavailable for screenshot capture");
  const path = join(outputDirectory, `${name}.png`);
  await writeFile(path, Buffer.from(await driver.takeScreenshot(), "base64"));
  return relative(outputDirectory, path).replaceAll("\\", "/");
}

function stopDriverProcess(): void {
  if (!driverProcess?.pid || driverProcess.exitCode !== null) return;
  if (process.platform === "win32") {
    try {
      execFileSync("taskkill", ["/pid", String(driverProcess.pid), "/t", "/f"], { stdio: "ignore" });
    } catch {
      driverProcess.kill();
    }
  } else {
    driverProcess.kill("SIGTERM");
  }
}

async function findWorkspaceButton(title: string): Promise<WebElement> {
  if (!driver) throw new Error("WebDriver session is unavailable");
  const button = await driver.wait(
    until.elementLocated(By.css(`button[aria-label="Open ${title}"]`)),
    10_000,
    `Could not find the accessible Open ${title} control`,
  );
  await driver.wait(until.elementIsVisible(button), 10_000);
  return button;
}

async function waitForDriverServer(): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError = "tauri-driver did not respond";
  while (Date.now() < deadline) {
    if (driverStartError) throw new Error(`Could not start tauri-driver: ${driverStartError.message}`);
    if (driverProcess && driverProcess.exitCode !== null) {
      throw new Error(`tauri-driver exited before it became ready with code ${driverProcess.exitCode}`);
    }
    try {
      const response = await fetch("http://127.0.0.1:4444/status", { signal: AbortSignal.timeout(1_000) });
      if (response.ok) return;
      lastError = `tauri-driver returned HTTP ${response.status}`;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 250));
  }
  throw new Error(`tauri-driver did not become ready within 30 seconds: ${lastError}`);
}

async function run(): Promise<void> {
  await persistReport();
  if (process.platform !== "win32") throw new Error("The desktop smoke runner must run on Windows");
  if (basename(executable).toLowerCase() !== "toolbox.exe") throw new Error(`Unexpected app executable: ${executable}`);

  driverProcess = spawn("tauri-driver", [], { stdio: "inherit", windowsHide: true });
  driverProcess.once("error", (error) => {
    driverStartError = error;
    report.setupError = `Could not start tauri-driver: ${error.message}`;
  });

  await waitForDriverServer();
  driver = await new Builder()
    .usingServer("http://127.0.0.1:4444")
    .withCapabilities({
      browserName: "wry",
      "tauri:options": { application: executable },
    })
    .build();

  await driver.wait(until.elementLocated(By.css("main.command-center")), 30_000, "Toolbox home did not open");
  report.homeScreenshot = await captureScreenshot("home");

  for (const workspace of ToolWorkspaceRegistry) {
    const evidenceIndex = report.workspaces.findIndex((item) => item.workspaceId === workspace.id);
    try {
      const openButton = await findWorkspaceButton(workspace.title);
      await openButton.click();
      const heading = await driver.wait(until.elementLocated(By.css("h1.workspace-title")), 10_000);
      await driver.wait(until.elementIsVisible(heading), 10_000);
      const actualTitle = (await heading.getText()).trim();
      if (actualTitle !== workspace.title) throw new Error(`Expected workspace heading “${workspace.title}”, received “${actualTitle}”`);

      const content = await driver.wait(until.elementLocated(By.css(".tool-workspace .tool-view")), 10_000);
      await driver.wait(until.elementIsVisible(content), 10_000);
      if (!(await driver.executeScript<boolean>("return arguments[0].children.length > 0", content))) {
        throw new Error(`The ${workspace.title} workspace content did not render`);
      }
      const screenshot = await captureScreenshot(`workspace-${workspace.id}`);
      report.workspaces[evidenceIndex] = { workspaceId: workspace.id, title: workspace.title, status: "passed", screenshot };
      await persistReport();

      const backButton = await driver.wait(
        until.elementLocated(By.xpath('//button[normalize-space(.)="← All tools"]')),
        10_000,
        "Could not find the All tools navigation control",
      );
      await backButton.click();
      const library = await driver.wait(until.elementLocated(By.css(".tool-library-section")), 10_000);
      await driver.wait(until.elementIsVisible(library), 10_000);
    } catch (error) {
      let screenshot: string | undefined;
      try {
        screenshot = await captureScreenshot(`workspace-${workspace.id}-failed`);
      } catch {
        screenshot = undefined;
      }
      report.workspaces[evidenceIndex] = {
        workspaceId: workspace.id,
        title: workspace.title,
        status: "failed",
        error: error instanceof Error ? error.message : String(error),
        ...(screenshot ? { screenshot } : {}),
      };
      await persistReport();
      break;
    }
  }

  if (report.workspaces.some((item) => item.status !== "passed")) {
    throw new Error("One or more workspace checks did not pass");
  }
}

try {
  await run();
} catch (error) {
  report.status = "failed";
  if (!report.workspaces.some((item) => item.status === "failed")) {
    report.setupError = error instanceof Error ? error.message : String(error);
  }
} finally {
  if (driver) {
    try {
      await driver.quit();
    } catch (error) {
      report.status = "failed";
      report.setupError ??= `Could not close WebDriver session: ${error instanceof Error ? error.message : String(error)}`;
    }
  }
  stopDriverProcess();
  report.finishedAt = new Date().toISOString();
  if (report.status !== "failed" && report.workspaces.every((item) => item.status === "passed")) report.status = "passed";
  if (report.status === "running") report.status = "failed";
  try {
    await persistReport();
  } catch (error) {
    console.error(`Could not persist desktop smoke report at ${reportPath}:`, error);
    process.exitCode = 1;
  }
}

if (report.status !== "passed") process.exitCode = 1;
console.log(`Desktop smoke report: ${reportPath}`);
