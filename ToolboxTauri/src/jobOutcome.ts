import type {
  AtomicToolId,
  ErrorKind,
  JobOutcome,
  OutcomePointer,
  OutcomeWarning,
  SkipCode,
  SkipReason,
  ToolError,
  ToolResult,
  WarningCode,
} from "./contracts";
import { UtilityRegistry } from "./registry";

const errorKinds: readonly ErrorKind[] = ["invalidInput", "unavailable", "limitExceeded", "processing"];

const skipCodes: readonly SkipCode[] = [
  "notProtected",
  "alreadyProtected",
  "permissionsPasswordOnly",
  "noSavings",
  "alreadyInTargetFormat",
  "rangeEmptyAfterClamping",
  "textAlreadyPresent",
  "noEffectiveChange",
  "unsupportedFormat",
  "cancelled",
];

const warningDetailKeys: Record<WarningCode, readonly string[] | null> = {
  signatureRemoved: null,
  fontSubstituted: ["requestedFamily", "actualFamily"],
  renamedToAvoidReplacing: ["chosenName", "finalName"],
  precisionReduced: null,
};

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

const parsePointer = (value: unknown, path: string): OutcomePointer | null => {
  if (value === null || value === undefined) return null;
  const pointer = record(value, path);
  if (pointer.kind === "tool") {
    const toolId = text(pointer.toolId, `${path}.toolId`);
    return toolIds.has(toolId) ? { kind: "tool", toolId: toolId as AtomicToolId } : fail(`${path}.toolId`, "a registered tool id");
  }
  if (pointer.kind === "pack") return { kind: "pack", packId: text(pointer.packId, `${path}.packId`) };
  return fail(`${path}.kind`, "tool or pack");
};

const parseToolError = (value: unknown, path: string): ToolError => {
  const error = record(value, path);
  return {
    kind: member(error.kind, errorKinds, `${path}.kind`),
    message: text(error.message, `${path}.message`),
    pointer: parsePointer(error.pointer, `${path}.pointer`),
  };
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
  const code = member(warning.code, Object.keys(warningDetailKeys) as WarningCode[], `${path}.code`);
  const message = text(warning.message, `${path}.message`);
  const detailKeys = warningDetailKeys[code];
  if (!detailKeys) return { code, message } as OutcomeWarning;
  const details = record(warning.details, `${path}.details`);
  return {
    code,
    message,
    details: Object.fromEntries(detailKeys.map((key) => [key, text(details[key], `${path}.details.${key}`)])),
  } as OutcomeWarning;
};

export const parseJobOutcome = (value: unknown, path = "outcome"): JobOutcome => {
  const outcome = record(value, path);
  const base = {
    inputPath: text(outcome.inputPath, `${path}.inputPath`),
    detail: text(outcome.detail, `${path}.detail`),
    warnings: Array.isArray(outcome.warnings)
      ? outcome.warnings.map((warning, index) => parseWarning(warning, `${path}.warnings[${index}]`))
      : fail(`${path}.warnings`, "an array"),
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

export const failedOutcome = (inputPath: string, kind: ErrorKind, message: string): JobOutcome => ({
  inputPath,
  state: "failed",
  outputPaths: [],
  detail: "",
  failure: { kind, message, pointer: null },
  reason: null,
  warnings: [],
});
