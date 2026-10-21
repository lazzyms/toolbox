import type { Page } from "@playwright/test";
import { UtilityRegistry } from "../../src/registry";

/**
 * Opens a registered tool the way a person finds it: by searching its title on the home screen.
 * Tools folded into an editor show as that editor's entry, which still opens the searched tool.
 */
export const openToolBySearch = async (page: Page, title: string) => {
    const tool = UtilityRegistry.find((item) => item.title === title);
    if (!tool) throw new Error(`No registered tool is titled ${title}`);
    await page.getByRole("textbox", { name: "Search tools" }).fill(title);
    await page.locator(`[data-command-result="${tool.id}"]`).click();
};
