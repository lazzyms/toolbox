import rawCapabilities from "../../shared/tool-capabilities.json" with { type: "json" };
import type { FormatFamily, NativeAvailability, ToolCapability, ToolCommand } from "../contracts";

export interface SharedCapability extends ToolCapability {
  command: string;
}

const requiredCapabilityKeys = [
  "command",
  "inputCardinality",
  "supportsPageSelection",
  "supportsPreview",
  "nativeAvailability",
  "outputSuffix",
  "manyToOne",
];
const capabilityKeys = new Set([...requiredCapabilityKeys, "acceptedExtensions", "formatFamilies"]);
const familyKeys = new Set(["id", "extensions", "nativeAvailability", "basicFidelity"]);

const optionalCapabilityKeys = new Set(["outputFormats"]);

const isExtensionList = (value: unknown): value is string[] =>
  Array.isArray(value) &&
  value.length > 0 &&
  value.every((extension) => typeof extension === "string" && /^\.[a-z0-9]+$/.test(extension)) &&
  new Set(value).size === value.length;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isInputCardinality = (
  value: unknown,
): value is ToolCapability["inputCardinality"] =>
  value === "single" || value === "multiple" || value === "ordered" || value === "none";

const isNativeAvailability = (value: unknown): value is NativeAvailability =>
  value === "available" || value === "unavailable";

const parseFormatFamilies = (command: string, value: unknown): FormatFamily[] => {
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error(`${command} has no format families.`);
  }
  const ids = new Set<string>();
  return value.map((family) => {
    if (!isRecord(family) || Object.keys(family).some((key) => !familyKeys.has(key))) {
      throw new Error(`${command} has an invalid format family.`);
    }
    const { id, extensions, nativeAvailability, basicFidelity = false } = family;
    if (typeof id !== "string" || !/^[a-z0-9]+(-[a-z0-9]+)*$/.test(id) || ids.has(id)) {
      throw new Error(`${command} has an invalid or duplicate format family ${String(id)}.`);
    }
    ids.add(id);
    if (!Array.isArray(extensions) || extensions.length === 0) {
      throw new Error(`${command} format family ${id} has no extensions.`);
    }
    if (!isNativeAvailability(nativeAvailability) || typeof basicFidelity !== "boolean") {
      throw new Error(`${command} format family ${id} has invalid flags.`);
    }
    return { id, extensions, nativeAvailability, basicFidelity };
  });
};

const parseCapability = (value: unknown): SharedCapability => {
  if (!isRecord(value)) throw new Error("Each shared capability must be an object.");
  if (
    requiredCapabilityKeys.some((key) => !(key in value)) ||
    Object.keys(value).some((key) => !capabilityKeys.has(key) && !optionalCapabilityKeys.has(key))
  ) {
    throw new Error("Each shared capability must contain only the contract fields.");
  }
  const {
    command,
    inputCardinality,
    supportsPageSelection,
    supportsPreview,
    nativeAvailability,
    outputSuffix,
    manyToOne,
    outputFormats,
  } = value;
  if (typeof command !== "string" || command.length === 0) {
    throw new Error("Each shared capability needs a command.");
  }
  if (!isInputCardinality(inputCardinality)) {
    throw new Error(`${command} has an invalid input cardinality.`);
  }
  if ((value.acceptedExtensions == null) === (value.formatFamilies == null)) {
    throw new Error(`${command} must declare either accepted extensions or format families.`);
  }
  const formatFamilies =
    value.formatFamilies == null ? undefined : parseFormatFamilies(command, value.formatFamilies);
  const acceptedExtensions =
    formatFamilies?.flatMap((family) => family.extensions) ?? value.acceptedExtensions;
  if (inputCardinality === "none" && Array.isArray(acceptedExtensions) && acceptedExtensions.length > 0) {
    throw new Error(`${command} takes no input files, so it cannot declare accepted extensions.`);
  }
  if (
    !Array.isArray(acceptedExtensions) ||
    (inputCardinality !== "none" && acceptedExtensions.length === 0) ||
    !acceptedExtensions.every(
      (extension) =>
        typeof extension === "string" &&
        /^\.[a-z0-9]+$/.test(extension),
    ) ||
    new Set(acceptedExtensions).size !== acceptedExtensions.length
  ) {
    throw new Error(`${command} has invalid accepted extensions.`);
  }
  if (typeof supportsPageSelection !== "boolean" || typeof supportsPreview !== "boolean") {
    throw new Error(`${command} has invalid capability flags.`);
  }
  if (!isNativeAvailability(nativeAvailability)) {
    throw new Error(`${command} has invalid native availability.`);
  }
  if (typeof outputSuffix !== "string" || !/^(-[a-z0-9]+)*$/.test(outputSuffix)) {
    throw new Error(`${command} needs exactly one valid output suffix.`);
  }
  if (typeof manyToOne !== "boolean") {
    throw new Error(`${command} has an invalid many-to-one flag.`);
  }
  if (outputFormats !== undefined && !isExtensionList(outputFormats)) {
    throw new Error(`${command} has invalid output formats.`);
  }
  return {
    command,
    acceptedExtensions,
    ...(formatFamilies && { formatFamilies }),
    inputCardinality,
    supportsPageSelection,
    supportsPreview,
    nativeAvailability,
    outputSuffix,
    manyToOne,
    ...(outputFormats ? { outputFormats } : {}),
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
    ...(capability.formatFamilies && { formatFamilies: capability.formatFamilies }),
    inputCardinality: capability.inputCardinality,
    supportsPageSelection: capability.supportsPageSelection,
    supportsPreview: capability.supportsPreview,
    nativeAvailability: capability.nativeAvailability,
    outputSuffix: capability.outputSuffix,
    manyToOne: capability.manyToOne,
    ...(capability.outputFormats ? { outputFormats: capability.outputFormats } : {}),
  });
}

export const capabilityFor = (command: ToolCommand): ToolCapability => {
  const capability = capabilitiesByCommand.get(command);
  if (!capability) throw new Error(`Missing shared capability for ${command}.`);
  return capability;
};
