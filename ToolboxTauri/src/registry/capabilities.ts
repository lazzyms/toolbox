import rawCapabilities from "../../shared/tool-capabilities.json" with { type: "json" };
import type { ToolCapability, ToolCommand } from "../contracts";

export interface SharedCapability extends ToolCapability {
  command: string;
}

const capabilityKeys = new Set([
  "command",
  "acceptedExtensions",
  "inputCardinality",
  "supportsPageSelection",
  "supportsPreview",
  "nativeAvailability",
]);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isInputCardinality = (
  value: unknown,
): value is ToolCapability["inputCardinality"] =>
  value === "single" || value === "multiple" || value === "ordered";

const parseCapability = (value: unknown): SharedCapability => {
  if (!isRecord(value)) throw new Error("Each shared capability must be an object.");
  const keys = Object.keys(value);
  if (keys.length !== capabilityKeys.size || keys.some((key) => !capabilityKeys.has(key))) {
    throw new Error("Each shared capability must contain only the contract fields.");
  }
  const {
    command,
    acceptedExtensions,
    inputCardinality,
    supportsPageSelection,
    supportsPreview,
    nativeAvailability,
  } = value;
  if (typeof command !== "string" || command.length === 0) {
    throw new Error("Each shared capability needs a command.");
  }
  if (
    !Array.isArray(acceptedExtensions) ||
    acceptedExtensions.length === 0 ||
    !acceptedExtensions.every(
      (extension) =>
        typeof extension === "string" &&
        /^\.[a-z0-9]+$/.test(extension),
    ) ||
    new Set(acceptedExtensions).size !== acceptedExtensions.length
  ) {
    throw new Error(`${command} has invalid accepted extensions.`);
  }
  if (!isInputCardinality(inputCardinality)) {
    throw new Error(`${command} has an invalid input cardinality.`);
  }
  if (typeof supportsPageSelection !== "boolean" || typeof supportsPreview !== "boolean") {
    throw new Error(`${command} has invalid capability flags.`);
  }
  if (nativeAvailability !== "available" && nativeAvailability !== "unavailable") {
    throw new Error(`${command} has invalid native availability.`);
  }
  return {
    command,
    acceptedExtensions,
    inputCardinality,
    supportsPageSelection,
    supportsPreview,
    nativeAvailability,
  };
};

export const parseCapabilityRegistry = (value: unknown): SharedCapability[] => {
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error("The shared capability registry must be a non-empty array.");
  }
  const capabilities = value.map(parseCapability);
  const commands = new Set<string>();
  for (const capability of capabilities) {
    if (commands.has(capability.command)) {
      throw new Error(`Duplicate shared capability command ${capability.command}.`);
    }
    commands.add(capability.command);
  }
  return capabilities;
};

export const SharedCapabilityRegistry = parseCapabilityRegistry(rawCapabilities);

const capabilitiesByCommand = new Map<string, ToolCapability>();
for (const capability of SharedCapabilityRegistry) {
  capabilitiesByCommand.set(capability.command, {
    acceptedExtensions: capability.acceptedExtensions,
    inputCardinality: capability.inputCardinality,
    supportsPageSelection: capability.supportsPageSelection,
    supportsPreview: capability.supportsPreview,
    nativeAvailability: capability.nativeAvailability,
  });
}

export const capabilityFor = (command: ToolCommand): ToolCapability => {
  const capability = capabilitiesByCommand.get(command);
  if (!capability) throw new Error(`Missing shared capability for ${command}.`);
  return capability;
};
