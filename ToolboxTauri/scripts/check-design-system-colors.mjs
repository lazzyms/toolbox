import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const sourceDirectory = path.resolve(scriptDirectory, "../src");
const tokenFile = path.join(sourceDirectory, "design-system/tokens.css");
const extensions = new Set([".css", ".js", ".jsx", ".ts", ".tsx", ".svg"]);
const colorValue = /#[\da-f]{3,8}\b|\b(?:rgba?|hsla?|oklab|oklch|lab|lch)\s*\(/gi;
const namedColors = new Set(
  "aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen".split(" "),
);
const colorDeclaration =
  /\b(?:color|background(?:-color)?|border(?:-[a-z-]+)?|outline(?:-[a-z-]+)?|fill|stroke|stop-color|box-shadow|text-shadow)\s*[:=]\s*([^;{}]+)/gi;

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filePath = path.join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(filePath);
    return extensions.has(path.extname(entry.name)) ? [filePath] : [];
  });
}

function lineAt(source, index) {
  return source.slice(0, index).split("\n").length;
}

const issues = [];
const files = sourceFiles(sourceDirectory).filter((filePath) => filePath !== tokenFile);

for (const filePath of files) {
  const source = readFileSync(filePath, "utf8");
  const declarations = [...source.matchAll(colorDeclaration)];
  for (const match of source.matchAll(colorValue)) {
    issues.push({ filePath, line: lineAt(source, match.index), value: match[0] });
  }
  for (const declaration of declarations) {
    const values = declaration[1].match(/\b[a-z]+\b/gi) ?? [];
    const literal = values.find((value) => namedColors.has(value.toLowerCase()));
    if (literal) {
      issues.push({
        filePath,
        line: lineAt(source, declaration.index),
        value: literal,
      });
    }
  }
}

if (issues.length) {
  console.error("Move color literals into src/design-system/tokens.css:");
  for (const issue of issues) {
    console.error(
      `  ${path.relative(path.dirname(sourceDirectory), issue.filePath)}:${issue.line} ${issue.value}`,
    );
  }
  process.exitCode = 1;
} else {
  console.log(
    `Checked ${files.length} source files. Color values live in src/design-system/tokens.css.`,
  );
}
