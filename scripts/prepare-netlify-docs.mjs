// Builds the public website from docs/ into .netlify/publish for Netlify.
// Usage: node scripts/prepare-netlify-docs.mjs [output-directory]
import {
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { site as hostedSite } from "../docs/landing-pages.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const defaultSourceRoot = path.join(repositoryRoot, "docs");
export const defaultOutputRoot = path.join(repositoryRoot, ".netlify", "publish");

// docs/ also holds internal Markdown, scripts and research, so only these are public.
const rootFiles = ["og.jpg", "robots.txt", "sitemap.xml", "llms.txt"];
const socialUrl = /<(?:link rel="canonical" href|meta (?:property|name)="(?:og:url|og:image|twitter:image)" content)="([^"]+)"/g;
const redirectRules = ["/docs / 301", "/docs/ / 301", "/docs/* /:splat 301", ""].join("\n");

const isWithin = (parent, child) => {
  const relativePath = path.relative(parent, child);
  return relativePath === "" || (!relativePath.startsWith("..") && !path.isAbsolute(relativePath));
};

function listRegularFiles(root, directory = root) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolutePath = path.join(directory, entry.name);
    const relativePath = path.relative(root, absolutePath).split(path.sep).join("/");
    if (entry.isSymbolicLink()) throw new Error(`The public site contains a symbolic link: ${relativePath}`);
    if (entry.isDirectory()) return listRegularFiles(root, absolutePath);
    if (entry.isFile()) return [relativePath];
    throw new Error(`The public site contains an unsupported file: ${relativePath}`);
  }).sort();
}

function requireRegularFile(sourceRoot, relativePath) {
  const info = lstatSync(path.join(sourceRoot, relativePath), { throwIfNoEntry: false });
  if (!info?.isFile()) throw new Error(`Expected a regular public file at docs/${relativePath}.`);
}

function publishedPath(url, site) {
  const parsed = new URL(url);
  if (parsed.origin !== site) return null;
  const pathname = decodeURIComponent(parsed.pathname).replace(/^\//, "");
  return pathname === "" || pathname.endsWith("/") ? `${pathname}index.html` : pathname;
}

function verifyLocalReferences(stagingRoot, fileName, html) {
  const pageDirectory = path.dirname(path.join(stagingRoot, fileName));
  for (const [, reference] of html.matchAll(/\b(?:href|src|poster)=["']([^"']+)["']/gi)) {
    if (/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(reference)) continue;
    const pathname = decodeURIComponent(reference.replaceAll("&amp;", "&").split(/[?#]/, 1)[0]);
    if (!pathname) continue;

    const target = path.resolve(pageDirectory, pathname);
    if (!isWithin(stagingRoot, target)) {
      throw new Error(`${fileName} references a path outside the site: ${reference}`);
    }
    const info = statSync(target, { throwIfNoEntry: false });
    const file = info?.isDirectory() ? statSync(path.join(target, "index.html"), { throwIfNoEntry: false }) : info;
    if (!file?.isFile()) throw new Error(`${fileName} references a missing local file: ${reference}`);
  }
}

function verifySiteUrls(fileName, html, site) {
  const urls = [...html.matchAll(socialUrl)].map((match) => match[1]);
  if (!html.includes('<link rel="canonical"')) throw new Error(`${fileName} needs a canonical URL.`);
  for (const url of urls) {
    if (new URL(url).origin !== site) throw new Error(`${fileName} points its canonical or social URL away from ${site}: ${url}`);
  }
}

function verifyListedPages(stagingRoot, fileName, published, site) {
  const source = readFileSync(path.join(stagingRoot, fileName), "utf8");
  for (const [url] of source.matchAll(/https?:\/\/[^\s<>()"']+/g)) {
    const target = publishedPath(url, site);
    if (target !== null && !published.has(target)) {
      throw new Error(`${fileName} lists a page that is not published: ${url}`);
    }
  }
}

export function prepareSite({
  sourceRoot = defaultSourceRoot,
  outputRoot = defaultOutputRoot,
  site = hostedSite,
} = {}) {
  sourceRoot = path.resolve(sourceRoot);
  outputRoot = path.resolve(outputRoot);
  if (isWithin(sourceRoot, outputRoot) || isWithin(outputRoot, sourceRoot)) {
    throw new Error(`The output directory must be outside docs/ and must not contain it: ${outputRoot}`);
  }

  const assetInfo = lstatSync(path.join(sourceRoot, "assets"), { throwIfNoEntry: false });
  if (!assetInfo?.isDirectory()) throw new Error("docs/assets must be a real directory.");
  const pages = readdirSync(sourceRoot).filter((name) => name.endsWith(".html")).sort();
  for (const file of [...pages, ...rootFiles]) requireRegularFile(sourceRoot, file);
  const assets = listRegularFiles(sourceRoot, path.join(sourceRoot, "assets"));

  const stagingRoot = `${outputRoot}.partial`;
  rmSync(stagingRoot, { recursive: true, force: true });
  try {
    mkdirSync(stagingRoot, { recursive: true });
    for (const file of [...pages, ...rootFiles, ...assets]) {
      mkdirSync(path.dirname(path.join(stagingRoot, file)), { recursive: true });
      cpSync(path.join(sourceRoot, file), path.join(stagingRoot, file));
    }
    writeFileSync(path.join(stagingRoot, "_redirects"), redirectRules);

    const published = new Set(listRegularFiles(stagingRoot));
    for (const page of pages) {
      const html = readFileSync(path.join(stagingRoot, page), "utf8");
      verifySiteUrls(page, html, site);
      verifyLocalReferences(stagingRoot, page, html);
    }
    for (const file of ["sitemap.xml", "llms.txt", "robots.txt"]) verifyListedPages(stagingRoot, file, published, site);

    rmSync(outputRoot, { recursive: true, force: true });
    renameSync(stagingRoot, outputRoot);
    return [...published];
  } finally {
    if (existsSync(stagingRoot)) rmSync(stagingRoot, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [outputRoot] = process.argv.slice(2);
  const files = prepareSite(outputRoot ? { outputRoot } : {});
  console.log(`Prepared ${files.length} public files in ${path.relative(process.cwd(), outputRoot ?? defaultOutputRoot) || "."}.`);
}
