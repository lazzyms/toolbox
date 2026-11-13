import type {
  AtomicToolId,
  ErrorKind,
  FontSource,
  FontStyle,
  FontUse,
  JobOutcome,
  LimitKind,
  OutcomePointer,
  OutcomeWarning,
  PackNeed,
  SkipCode,
  SkipReason,
  ToolError,
  ToolResult,
  WarningCode,
} from "./contracts";
import { jpegColours } from "./contracts";
import { UtilityRegistry } from "./registry";

const errorKinds: readonly ErrorKind[] = ["invalidInput", "unavailable", "limitExceeded", "processing", "wrongPassword", "cancelled"];

const packNeeds: readonly PackNeed[] = ["notInstalled", "damaged", "incompatible"];

const fontStyles: readonly FontStyle[] = ["regular", "bold", "italic", "boldItalic"];

const fontSources: readonly FontSource[] = ["embedded", "system", "bundled fallback", "bundled default"];

const limitKinds: readonly LimitKind[] = ["fileSize", "pageCount", "time", "decompression", "textLength"];

const skipCodes: readonly SkipCode[] = [
  "notProtected",
  "alreadyProtected",
  "noSavings",
  "alreadyInTargetFormat",
  "rangeEmptyAfterClamping",
  "textAlreadyPresent",
  "noEffectiveChange",
  "unsupportedFormat",
  "needsOcr",
  "cancelled",
];

const toolIds = new Set<string>(UtilityRegistry.map((utility) => utility.id));

type Json = Record<string, unknown>;

const fail = (path: string, expected: string): never => {
  throw new Error(`Invalid job outcome: ${path} must be ${expected}.`);
};

const record = (value: unknown, path: string): Json =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Json) : fail(path, "an object");

const text = (value: unknown, path: string): string => (typeof value === "string" ? value : fail(path, "a string"));

const member = <T extends string>(value: unknown, allowed: readonly T[], path: string): T =>
  allowed.includes(value as T) ? (value as T) : fail(path, `one of ${allowed.join(", ")}`);

const texts = (value: unknown, path: string): string[] =>
  Array.isArray(value) ? value.map((item, index) => text(item, `${path}[${index}]`)) : fail(path, "an array");

type DetailParser = (value: unknown, path: string) => unknown;

const count = (value: unknown, path: string): number =>
  Number.isSafeInteger(value) && (value as number) >= 0 ? (value as number) : fail(path, "a whole number");

const counts = (value: unknown, path: string): number[] =>
  Array.isArray(value) ? value.map((item, index) => count(item, `${path}[${index}]`)) : fail(path, "an array");

const colour = (value: unknown, path: string) => member(value, jpegColours, path);

const imagePlace = { page: count, image: count };

const warningDetails: Record<WarningCode, Record<string, DetailParser> | null> = {
  signatureRemoved: null,
  fontSubstituted: { requestedFamily: text, actualFamily: text },
  renamedToAvoidReplacing: { chosenName: text, finalName: text },
  precisionReduced: null,
  bookmarksNotCarried: { into: text },
  tagsNotCarried: { into: text },
  omittedContent: null,
  approximateLayout: null,
  imageUnsupported: { ...imagePlace, feature: text },
  imageDamaged: imagePlace,
  imageTooLarge: { ...imagePlace, width: count, height: count },
  jpegColourKept: { colour, pages: counts, images: count },
  pagesWithoutText: { pages: counts },
  transparencyFlattened: null,
};

const byteSizes = (outcome: Json, path: string): { originalBytes?: number; outputBytes?: number } => {
  const sizes: { originalBytes?: number; outputBytes?: number } = {};
  for (const key of ["originalBytes", "outputBytes"] as const) {
    const value = outcome[key];
    if (value === null || value === undefined) continue;
    sizes[key] = Number.isSafeInteger(value) && (value as number) >= 0 ? (value as number) : fail(`${path}.${key}`, "a non-negative whole number of bytes");
  }
  return sizes;
};

const parsePointer = (value: unknown, path: string): OutcomePointer | null => {
  if (value === null || value === undefined) return null;
  const pointer = record(value, path);
  if (pointer.kind === "tool") {
    const toolId = text(pointer.toolId, `${path}.toolId`);
    return toolIds.has(toolId) ? { kind: "tool", toolId: toolId as AtomicToolId } : fail(`${path}.toolId`, "a registered tool id");
  }
  if (pointer.kind === "pack") {
    return {
      kind: "pack",
      packId: text(pointer.packId, `${path}.packId`),
      name: text(pointer.name, `${path}.name`),
      reason: member(pointer.reason, packNeeds, `${path}.reason`),
    };
  }
  if (pointer.kind === "outputFolder") return { kind: "outputFolder" };
  return fail(`${path}.kind`, "tool, pack or outputFolder");
};

const parseToolError = (value: unknown, path: string): ToolError => {
  const error = record(value, path);
  const kind = member(error.kind, errorKinds, `${path}.kind`);
  const base = { message: text(error.message, `${path}.message`), pointer: parsePointer(error.pointer, `${path}.pointer`) };
  if (kind !== "limitExceeded") return { ...base, kind };
  const details = record(error.details, `${path}.details`);
  return { ...base, kind, details: { limit: member(details.limit, limitKinds, `${path}.details.limit`) } };
};

const parseSkipReason = (value: unknown, path: string): SkipReason => {
  const reason = record(value, path);
  return {
    code: member(reason.code, skipCodes, `${path}.code`),
    message: text(reason.message, `${path}.message`),
    pointer: parsePointer(reason.pointer, `${path}.pointer`),
  };
};

const parseWarning = (value: unknown, path: string): OutcomeWarning => {
  const warning = record(value, path);
  const code = member(warning.code, Object.keys(warningDetails) as WarningCode[], `${path}.code`);
  const message = text(warning.message, `${path}.message`);
  const parsers = warningDetails[code];
  if (!parsers) return { code, message } as OutcomeWarning;
  const details = record(warning.details, `${path}.details`);
  return {
    code,
    message,
    details: Object.fromEntries(Object.entries(parsers).map(([key, parse]) => [key, parse(details[key], `${path}.details.${key}`)])),
  } as OutcomeWarning;
};

const parseFontUse = (value: unknown, path: string): FontUse => {
  const font = record(value, path);
  return {
    requestedFamily: text(font.requestedFamily, `${path}.requestedFamily`),
    requestedStyle: member(font.requestedStyle, fontStyles, `${path}.requestedStyle`),
    fontUsed: text(font.fontUsed, `${path}.fontUsed`),
    styleUsed: member(font.styleUsed, fontStyles, `${path}.styleUsed`),
    source: member(font.source, fontSources, `${path}.source`),
  };
};

export const parseJobOutcome = (value: unknown, path = "outcome"): JobOutcome => {
  const outcome = record(value, path);
  const base = {
    inputPath: text(outcome.inputPath, `${path}.inputPath`),
    detail: text(outcome.detail, `${path}.detail`),
    warnings: Array.isArray(outcome.warnings)
      ? outcome.warnings.map((warning, index) => parseWarning(warning, `${path}.warnings[${index}]`))
      : fail(`${path}.warnings`, "an array"),
    ...(outcome.fonts === undefined
      ? {}
      : {
          fonts: Array.isArray(outcome.fonts)
            ? outcome.fonts.map((font, index) => parseFontUse(font, `${path}.fonts[${index}]`))
            : fail(`${path}.fonts`, "an array"),
        }),
    ...byteSizes(outcome, path),
  };
  const outputPaths = texts(outcome.outputPaths, `${path}.outputPaths`);
  const hasFailure = outcome.failure !== null && outcome.failure !== undefined;
  const hasReason = outcome.reason !== null && outcome.reason !== undefined;
  switch (outcome.state) {
    case "succeeded":
      if (hasFailure || hasReason) fail(path, "a succeeded outcome without a failure or reason");
      return { ...base, state: "succeeded", outputPaths, failure: null, reason: null };
    case "skipped":
      if (hasFailure || outputPaths.length > 0) fail(path, "a skipped outcome without a failure or outputs");
      return { ...base, state: "skipped", outputPaths: [], failure: null, reason: parseSkipReason(outcome.reason, `${path}.reason`) };
    case "failed":
      if (hasReason) fail(path, "a failed outcome without a skip reason");
      return { ...base, state: "failed", outputPaths, failure: parseToolError(outcome.failure, `${path}.failure`), reason: null };
    default:
      return fail(`${path}.state`, "succeeded, skipped or failed");
  }
};

export const parseToolResult = (value: unknown): ToolResult =>
  Array.isArray(value) ? value.map((outcome, index) => parseJobOutcome(outcome, `outcome[${index}]`)) : fail("result", "an array");

export const failedOutcome = (inputPath: string, kind: Exclude<ErrorKind, "limitExceeded">, message: string): JobOutcome => ({
  inputPath,
  state: "failed",
  outputPaths: [],
  detail: "",
  failure: { kind, message, pointer: null },
  reason: null,
  warnings: [],
});
