import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const projectDirectory = path.resolve(scriptDirectory, "..");
const sourceDirectory = path.join(projectDirectory, "src");
const uiDirectory = path.join(sourceDirectory, "components/ui");
const configPath = path.join(projectDirectory, "components.json");
const themePath = path.join(sourceDirectory, "styles/theme.css");
const requiredComponents = [
  "badge", "button", "card", "checkbox", "input", "label", "native-select",
  "dialog", "slider", "switch", "textarea", "toggle-group",
];
const cardSurfaceClasses = [
  "tool-card", "tool-workspace-card", "workspace-control-panel", "workspace-source-bar",
  "workspace-command-rail", "conversion-preview", "media-frame-order", "image-editor-preview",
  "image-editor-controls", "image-editor-result-preview", "image-editor-history",
  "file-dropzone", "file-selection-list", "result-card", "settings-info", "support-info",
  "scene-empty", "scene-thumbnails", "conversion-page-card",
];
const buttonClasses = [
  "rail-link", "quick-tool-link", "settings-link", "favorite-button",
  "workspace-action-button", "workspace-inline-button", "file-selection-item",
  "workspace-command", "scene-thumbnail", "workspace-source-open", "workspace-source-clear",
  "pdf-editor-open-files",
];
const componentClassSelectors = {
  card: [...cardSurfaceClasses, "tool-empty-state"],
  button: buttonClasses,
  input: ["search-box__input"],
  label: ["workspace-check", "conversion-page-choice"],
  badge: ["privacy-pill"],
  dialog: ["settings-dialog"],
};
const forbiddenProperties = {
  card: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|transition|transform)$/,
  button: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|height|min-height|font-size|font-weight|line-height|transition|transform)$/,
  input: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|height|min-height|font-size)$/,
  badge: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
  dialog: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
  label: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|font-size|font-weight|line-height)$/,
  select: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|height|min-height|font-size)$/,
  textarea: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?|height|min-height|font-size)$/,
  slider: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
  checkbox: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
  switch: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
  toggle: /^(?:background(?:-.+)?|border(?:-.+)?|box-shadow|color|outline(?:-.+)?|padding(?:-.+)?)$/,
};
const componentSlots = new Map([
  ["card", "card"],
  ["button", "button"],
  ["input", "input"],
  ["native-select", "select"],
  ["select", "select"],
  ["textarea", "textarea"],
  ["label", "label"],
  ["badge", "badge"],
  ["dialog-content", "dialog"],
  ["slider", "slider"],
  ["checkbox", "checkbox"],
  ["switch", "switch"],
  ["toggle", "toggle"],
  ["toggle-group", "toggle"],
]);
const extensions = new Set([".css", ".js", ".jsx", ".ts", ".tsx"]);
const issues = [];

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (filePath === uiDirectory) return [];
      return sourceFiles(filePath);
    }
    return extensions.has(path.extname(entry.name)) ? [filePath] : [];
  });
}

function report(message) {
  issues.push(message);
}

function cssRules(css) {
  const rules = [];
  const parents = [];
  let start = 0;
  let quote = "";
  let comment = false;

  for (let index = 0; index < css.length; index += 1) {
    const current = css[index];
    const next = css[index + 1];
    if (comment) {
      if (current === "*" && next === "/") {
        comment = false;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (current === "\\") index += 1;
      else if (current === quote) quote = "";
      continue;
    }
    if (current === "/" && next === "*") {
      comment = true;
      index += 1;
      continue;
    }
    if (current === "\"" || current === "'") {
      quote = current;
      continue;
    }
    if (current === "{") {
      const selector = css.slice(start, index).trim();
      parents.push(selector.startsWith("@") ? null : selector);
      start = index + 1;
    } else if (current === "}") {
      const selector = parents.pop();
      if (selector) rules.push({ selector, declarations: css.slice(start, index) });
      start = index + 1;
    } else if (current === ";" && parents.length === 0) {
      start = index + 1;
    }
  }

  return rules;
}

function componentKind(selector) {
  const subject = selector.trim().split(/[\s>+~]+/).at(-1) ?? "";
  if (/^input\[type\s*=\s*["']?(?:file|color)/i.test(subject)) return null;
  const slot = subject.match(/\[data-slot\s*=\s*["']([^"']+)["']\]/)?.[1];
  if (slot && componentSlots.has(slot)) return componentSlots.get(slot);
  if (/^button(?:[.:#\[]|$)/.test(subject)) return "button";
  if (/^input(?:[.:#\[]|$)/.test(subject)) return "input";
  if (/^select(?:[.:#\[]|$)/.test(subject)) return "select";
  if (/^textarea(?:[.:#\[]|$)/.test(subject)) return "textarea";

  for (const [kind, classNames] of Object.entries(componentClassSelectors)) {
    if (classNames.some((className) => new RegExp(`(^|[^\\w-])\\.${className}(?![\\w-])`).test(subject))) {
      return kind;
    }
  }
  return null;
}

if (!existsSync(configPath)) {
  report("components.json is missing.");
} else {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  if (config.style !== "new-york") report('components.json must use style "new-york".');
  if (config.tailwind?.baseColor !== "neutral") report('components.json must use baseColor "neutral".');
  if (config.tailwind?.cssVariables !== true) report("shadcn CSS variables must remain enabled.");
}

for (const component of requiredComponents) {
  if (!existsSync(path.join(uiDirectory, `${component}.tsx`))) {
    report(`Required shadcn component is missing: src/components/ui/${component}.tsx`);
  }
}

if (existsSync(path.join(sourceDirectory, "design-system"))) {
  report("Remove the retired src/design-system component system.");
}

const theme = existsSync(themePath) ? readFileSync(themePath, "utf8") : "";
if (!theme) report("src/styles/theme.css is missing.");
for (const token of ["background", "primary", "accent", "border"]) {
  const declaration = new RegExp(`--${token}\\s*:\\s*oklch\\(([^)]+)\\)`, "g");
  const matches = [...theme.matchAll(declaration)];
  if (matches.length < 2) {
    report(`Theme must define neutral --${token} tokens for dark and light modes.`);
    continue;
  }
  for (const match of matches) {
    const chroma = match[1].trim().split(/\s+/)[1]?.replace(/\/$/, "");
    if (Number(chroma) !== 0) report(`Theme token --${token} has non-neutral chroma: ${match[0]}`);
  }
}

const contractsPath = path.join(sourceDirectory, "contracts.ts");
const registryPath = path.join(sourceDirectory, "registry/index.ts");
const contracts = existsSync(contractsPath) ? readFileSync(contractsPath, "utf8") : "";
const registry = existsSync(registryPath) ? readFileSync(registryPath, "utf8") : "";
if (/\btint\s*:/.test(contracts) || /\btint\s*:/.test(registry) || /--color-tool-/.test(theme)) {
  report("Remove the retired per-tool tint palette and tint fields.");
}

const appFiles = sourceFiles(sourceDirectory);
for (const filePath of appFiles) {
  const source = readFileSync(filePath, "utf8");
  const relativePath = path.relative(projectDirectory, filePath);
  if (/from\s+["'][^"']*design-system|import\s+["'][^"']*design-system/.test(source)) {
    report(`${relativePath} imports from the retired design system.`);
  }

  if (path.extname(filePath) === ".css") {
    for (const rule of cssRules(source)) {
      for (const selector of rule.selector.split(",")) {
        const kind = componentKind(selector);
        const forbidden = forbiddenProperties[kind];
        if (!forbidden) continue;
        const properties = new Set();
        for (const declaration of rule.declarations.split(";")) {
          const [property] = declaration.split(":", 1);
          const normalizedProperty = property?.trim().toLowerCase();
          if (normalizedProperty && forbidden.test(normalizedProperty)) properties.add(normalizedProperty);
        }
        if (properties.size > 0) {
          report(`${relativePath} re-styles a shadcn ${kind} through ${[...properties].join(", ")}: ${selector.trim()}`);
        }
      }
    }
  }
  if (/\bds-[\w-]+/.test(source)) report(`${relativePath} still uses a retired ds-* class.`);

  for (const match of source.matchAll(/<(button|select|textarea|input|dialog)\b[^>]*>/g)) {
    const element = match[1].toLowerCase();
    const attributes = match[0];
    if (element === "input" && /\btype\s*=\s*(["'])(file|color)\1/i.test(attributes)) continue;
    const line = source.slice(0, match.index).split("\n").length;
    report(`${relativePath}:${line} uses a native <${element}> instead of its shadcn component.`);
  }

  for (const match of source.matchAll(/<(div|section|article|span)\b[^>]*\bonClick\s*=/gi)) {
    const line = source.slice(0, match.index).split("\n").length;
    report(`${relativePath}:${line} puts an app action on a generic layout element instead of a shadcn control.`);
  }

  for (const match of source.matchAll(/<(div|section|article|aside|main)\b([^>]*)>/gs)) {
    const classNames = [...match[2].matchAll(/\bclassName\s*=\s*\{?\s*(["'`])([\s\S]*?)\1/g)]
      .flatMap((classMatch) => classMatch[2].split(/\s+/));
    const surfaceClass = cardSurfaceClasses.find((className) => classNames.includes(className));
    if (surfaceClass) {
      const line = source.slice(0, match.index).split("\n").length;
      report(`${relativePath}:${line} renders the ${surfaceClass} surface on <${match[1]}> instead of shadcn Card.`);
    }
  }

  for (const match of source.matchAll(/<(div|section|article|aside|main)\b[^>]*\brole\s*=\s*(["'])(dialog|alertdialog)\2/gi)) {
    const line = source.slice(0, match.index).split("\n").length;
    report(`${relativePath}:${line} builds a dialog with raw ARIA markup instead of shadcn Dialog.`);
  }
}

if (issues.length) {
  console.error("shadcn component contract failed:");
  for (const issue of issues) console.error(`  ${issue}`);
  process.exitCode = 1;
} else {
  console.log(`Verified neutral shadcn components and blocked legacy tints, component CSS re-skinning, native controls, raw dialogs, click-only layout actions, and non-Card reusable surfaces across ${appFiles.length} app source files.`);
}
