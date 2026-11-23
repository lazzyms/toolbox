import { readFileSync } from "node:fs";
import ts from "typescript";
import { tsImport } from "tsx/esm/api";

/** The tool ids `AtomicToolId` declares in src/contracts.ts, read from its type declaration. */
export function declaredToolIds() {
  const source = readFileSync(new URL("../src/contracts.ts", import.meta.url), "utf8");
  const file = ts.createSourceFile("contracts.ts", source, ts.ScriptTarget.Latest);
  const alias = file.statements.find((node) => ts.isTypeAliasDeclaration(node) && node.name.text === "AtomicToolId");
  if (!alias) throw new Error("src/contracts.ts no longer declares AtomicToolId");
  const members = ts.isUnionTypeNode(alias.type) ? alias.type.types : [alias.type];
  return members.map((member) => {
    if (!ts.isLiteralTypeNode(member) || !ts.isStringLiteral(member.literal)) throw new Error("AtomicToolId must be a union of string literals");
    return member.literal.text;
  });
}

/** The registry the app runs, imported rather than parsed from source. */
export async function loadRegistry() {
  const { UtilityRegistry, ToolWorkspaceRegistry } = await tsImport("../src/registry/index.ts", import.meta.url);
  return { tools: UtilityRegistry, workspaces: ToolWorkspaceRegistry };
}

export const duplicates = (values) => [...new Set(values.filter((value, index) => values.indexOf(value) !== index))];
