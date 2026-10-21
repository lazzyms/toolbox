import rawRetiredToolIds from "../../shared/retired-tool-ids.json" with { type: "json" };
import type { AtomicToolId } from "../contracts";
import { UtilityRegistry } from "./index";

export interface RetiredToolIdTable {
  version: number;
  replacements: ReadonlyMap<string, AtomicToolId | "removed">;
}

export const registeredToolIds: ReadonlySet<string> = new Set(UtilityRegistry.map(({ id }) => id));

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export const parseRetiredToolIds = (value: unknown): RetiredToolIdTable => {
  if (!isRecord(value) || Object.keys(value).sort().join() !== "retired,version") {
    throw new Error("The retired tool id table must contain only version and retired.");
  }
  const { version, retired } = value;
  if (typeof version !== "number" || !Number.isInteger(version) || version < 0) {
    throw new Error("The retired tool id table needs a non-negative integer version.");
  }
  if (!isRecord(retired)) throw new Error("Retired tool ids must map each id to a replacement or removed.");
  const replacements = new Map<string, AtomicToolId | "removed">();
  for (const [retiredId, replacement] of Object.entries(retired)) {
    if (registeredToolIds.has(retiredId)) throw new Error(`${retiredId} is still a registered tool.`);
    if (replacement !== "removed" && (typeof replacement !== "string" || !registeredToolIds.has(replacement))) {
      throw new Error(`${retiredId} must map to a registered tool id or removed.`);
    }
    replacements.set(retiredId, replacement as AtomicToolId | "removed");
  }
  return { version, replacements };
};

export const RetiredToolIds = parseRetiredToolIds(rawRetiredToolIds);
