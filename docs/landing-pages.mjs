// Builds the task landing pages, sitemap.xml, robots.txt and llms.txt from one list.
// Usage: node docs/landing-pages.mjs
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const docs = new URL('./', import.meta.url);
export const site = 'https://toolbox.mauliksompura.co.in';
const storeUrl = 'https://apps.microsoft.com/detail/9N5R8W4GJVH4';
const socialImageAlt = 'Toolbox social preview showing macOS and Windows availability and local PDF and image tools';
const categories = {
  pdf: { name: 'PDF tools', file: 'pdf.html' },
  images: { name: 'Image tools', file: 'images.html' },
};

const privacyAnswer = 'No. Toolbox is a desktop app, so it reads and writes the file on your own computer. It never uploads your files.';
const originalAnswer = 'No. Toolbox saves a new file beside the original and never overwrites it.';
const platformAnswer = 'Toolbox runs on macOS (Apple silicon) and Windows. The Windows app comes from the Microsoft Store. It is free and MIT licensed.';

export const landingPages = [
  {
    file: 'remove-pdf-password.html', toolId: 'pdf-unlock', category: 'pdf', tool: 'Remove Password',
    title: 'Remove a PDF password offline | Toolbox',
    description: 'Remove the password from a PDF, Word, Excel or PowerPoint file you can open. Free desktop app for Mac and Windows. Files never leave your computer.',
    kicker: 'Remove PDF password', heading: 'Remove a PDF password,<br><em>without uploading it.</em>',
    lede: 'Enter the password you already know, and Toolbox saves an unlocked copy of the PDF or Office file. Nothing is uploaded.',
    steps: [
      'Open Toolbox and choose Remove Password.',
      'Add one or more PDF, DOC, DOCX, XLS, XLSX, PPT or PPTX files.',
      'Type the password you use to open each file.',
      'Toolbox saves an unlocked copy next to each original.',
    ],
    faqs: [
      ['Can Toolbox unlock a PDF if I forgot the password?', 'No. Remove Password needs the password you use to open the file. It cannot recover, guess or bypass a password.'],
      ['Which files can I unlock?', 'PDF files and password protected Word, Excel and PowerPoint files: DOC, DOCX, XLS, XLSX, PPT and PPTX. Unsupported encryption returns an error for that file only.'],
      ['Is my PDF uploaded to a server?', privacyAnswer],
      ['Does it change the original file?', originalAnswer],
    ],
    related: ['protect-pdf.html', 'compress-pdf.html', 'merge-pdf.html'],
  },
  {
    file: 'protect-pdf.html', toolId: 'pdf-protect', category: 'pdf', tool: 'Protect PDF',
    title: 'Password protect a PDF offline | Toolbox',
    description: 'Add a password to a PDF, DOCX or XLSX file on your own computer. Free desktop app for Mac and Windows, with no upload.',
    kicker: 'Protect PDF', heading: 'Lock a PDF<br><em>on your own machine.</em>',
    lede: 'Add an open password to PDFs, and to DOCX or XLSX files with Protect Office Files. Toolbox saves a protected copy.',
    steps: [
      'Open Toolbox and choose Protect PDF, or Protect Office Files for DOCX and XLSX.',
      'Add the files you want to lock.',
      'Type the password.',
      'Toolbox saves a password protected copy next to each original.',
    ],
    faqs: [
      ['Is the password sent anywhere?', 'No. Toolbox encrypts the file on your computer. Your password and your file stay on your device.'],
      ['Can I protect Word and Excel files too?', 'Yes. Protect Office Files adds a password to DOCX and XLSX files.'],
      ['Does it change the original file?', originalAnswer],
    ],
    related: ['remove-pdf-password.html', 'sign-pdf.html', 'edit-pdf.html'],
  },
  {
    file: 'compress-pdf.html', toolId: 'pdf-compress', category: 'pdf', tool: 'Compress PDF',
    title: 'Compress a PDF offline | Toolbox',
    description: 'Make PDF files smaller on your Mac or Windows PC. Compress several PDFs at once without uploading them. Free desktop app.',
    kicker: 'Compress PDF', heading: 'Smaller PDFs,<br><em>no upload.</em>',
    lede: 'Toolbox compresses PDF stream data and keeps every page the same size and layout.',
    steps: [
      'Open Toolbox and choose Compress PDF.',
      'Add one or more PDF files.',
      'Start the run.',
      'Toolbox saves a compressed copy next to each original.',
    ],
    faqs: [
      ['How much smaller will my PDF get?', 'It depends on the file. Toolbox compresses the PDF stream data. Files that are already compressed may shrink very little.'],
      ['Can I compress several PDFs at once?', 'Yes. Add as many PDFs as you like. If one file fails, the others still finish.'],
      ['Is my PDF uploaded?', privacyAnswer],
    ],
    related: ['compress-image.html', 'merge-pdf.html', 'split-pdf.html'],
  },
  {
    file: 'merge-pdf.html', toolId: 'pdf-merge', category: 'pdf', tool: 'Merge PDF',
    title: 'Merge PDF files offline | Toolbox',
    description: 'Combine several PDFs into one file in the order you choose. Free desktop app for Mac and Windows that never uploads your files.',
    kicker: 'Merge PDF', heading: 'Combine PDFs<br><em>in your order.</em>',
    lede: 'Put several PDFs together into one document. Set the order before you merge.',
    steps: [
      'Open Toolbox and choose Merge PDF.',
      'Add the PDFs you want to combine.',
      'Arrange them in the order you want.',
      'Toolbox saves one merged PDF.',
    ],
    faqs: [
      ['Can I choose the page order?', 'Yes. Toolbox merges the files in the order you set. To reorder single pages afterwards, use Organize PDF.'],
      ['Are my PDFs uploaded to merge them?', privacyAnswer],
      ['Does it change the original files?', 'No. Toolbox writes a new merged PDF and leaves the source files as they are.'],
    ],
    related: ['split-pdf.html', 'jpg-to-pdf.html', 'compress-pdf.html'],
  },
  {
    file: 'split-pdf.html', toolId: 'pdf-split', category: 'pdf', tool: 'Split PDF',
    title: 'Split a PDF offline | Toolbox',
    description: 'Split one PDF into single pages, page ranges or fixed-size chunks. Free desktop app for Mac and Windows with no upload.',
    kicker: 'Split PDF', heading: 'Split a PDF<br><em>into the parts you need.</em>',
    lede: 'Break a PDF into single pages, page ranges or fixed-size chunks. You can also extract or remove selected pages.',
    steps: [
      'Open Toolbox and choose Split PDF.',
      'Add the PDF.',
      'Choose single pages, ranges such as 1-3, 7, or a fixed chunk size.',
      'Toolbox saves the new PDFs next to the original.',
    ],
    faqs: [
      ['Can I pull out only a few pages?', 'Yes. Use Extract PDF Pages to save selected pages as a new PDF, or Remove PDF Pages to drop the pages you do not need.'],
      ['Is my PDF uploaded?', privacyAnswer],
      ['Does it change the original file?', originalAnswer],
    ],
    related: ['merge-pdf.html', 'pdf-to-jpg.html', 'edit-pdf.html'],
  },
  {
    file: 'pdf-to-jpg.html', toolId: 'pdf-to-images', category: 'pdf', tool: 'PDF to Images',
    title: 'Convert PDF to JPG or PNG offline | Toolbox',
    description: 'Turn PDF pages into JPG or PNG images at 72, 150 or 300 dpi. Free desktop app for Mac and Windows. Files stay on your computer.',
    kicker: 'PDF to JPG', heading: 'PDF pages<br><em>to images.</em>',
    lede: 'Render every page of a PDF to JPEG or PNG at 72, 150 or 300 dpi.',
    steps: [
      'Open Toolbox and choose PDF to Images.',
      'Add one or more PDFs.',
      'Pick JPEG or PNG and a resolution of 72, 150 or 300 dpi.',
      'Toolbox saves one image per page.',
    ],
    faqs: [
      ['Which image formats can I export?', 'JPEG and PNG.'],
      ['Can I get the original images out of a PDF instead?', 'Yes. Extract Images pulls embedded JPEG images out at their original resolution without recompressing them.'],
      ['Is my PDF uploaded?', privacyAnswer],
    ],
    related: ['jpg-to-pdf.html', 'convert-image.html', 'split-pdf.html'],
  },
  {
    file: 'jpg-to-pdf.html', toolId: 'images-to-pdf', category: 'pdf', tool: 'Images to PDF',
    title: 'Convert JPG, PNG or HEIC to PDF offline | Toolbox',
    description: 'Turn photos and scans into one PDF in the order you choose. Supports JPG, PNG and HEIC. Free desktop app for Mac and Windows.',
    kicker: 'JPG to PDF', heading: 'Photos and scans<br><em>into one PDF.</em>',
    lede: 'Combine images into a single PDF in the order you choose, including HEIC photos on supported systems.',
    steps: [
      'Open Toolbox and choose Images to PDF.',
      'Add your images.',
      'Arrange them in page order.',
      'Toolbox saves one PDF.',
    ],
    faqs: [
      ['Can I convert iPhone HEIC photos to PDF?', 'Yes, on systems that can decode HEIC images.'],
      ['Are my photos uploaded?', privacyAnswer],
      ['Does it change the original images?', 'No. Toolbox writes a new PDF and leaves the images as they are.'],
    ],
    related: ['pdf-to-jpg.html', 'merge-pdf.html', 'convert-image.html'],
  },
  {
    file: 'sign-pdf.html', toolId: 'pdf-sign', category: 'pdf', tool: 'Sign PDF',
    title: 'Sign a PDF offline | Toolbox',
    description: 'Place a typed name or a signature image on one or more PDF pages. Free desktop app for Mac and Windows with no upload.',
    kicker: 'Sign PDF', heading: 'Sign a PDF<br><em>without an upload.</em>',
    lede: 'Add a typed name, or a PNG or JPEG image of your signature, to one or more pages of a PDF.',
    steps: [
      'Open Toolbox and choose Sign PDF.',
      'Add the PDF.',
      'Type your name or choose a PNG or JPEG signature image, then pick the pages.',
      'Toolbox saves a signed copy next to the original.',
    ],
    faqs: [
      ['Is this a digital certificate signature?', 'No. Sign PDF places a visible signature mark on the page. It is not a cryptographic signature.'],
      ['Is my document uploaded?', privacyAnswer],
      ['Does it change the original file?', originalAnswer],
    ],
    related: ['edit-pdf.html', 'protect-pdf.html', 'merge-pdf.html'],
  },
  {
    file: 'edit-pdf.html', toolId: 'pdf-edit', category: 'pdf', tool: 'Edit PDF',
    title: 'Edit a PDF offline: add text and notes | Toolbox',
    description: 'Add text, highlights, rectangles and notes to PDF pages, or add a watermark or page numbers. Free desktop app for Mac and Windows.',
    kicker: 'Edit PDF', heading: 'Mark up a PDF<br><em>on your desktop.</em>',
    lede: 'Add text, highlights, rectangle shapes and notes to selected pages. Watermark, crop, organize and number pages with the other PDF tools.',
    steps: [
      'Open Toolbox and choose Edit PDF.',
      'Add the PDF.',
      'Add text, highlights, rectangles or notes to the pages you select.',
      'Toolbox saves an edited copy next to the original.',
    ],
    faqs: [
      ['Can I change the existing text in a PDF?', 'No. Edit PDF adds new text, highlights, shapes and notes on top of the page. It does not rewrite text that is already there.'],
      ['Can I add page numbers or a watermark?', 'Yes. Use Add PDF Page Numbers or Watermark PDF.'],
      ['Is my PDF uploaded?', privacyAnswer],
    ],
    related: ['sign-pdf.html', 'split-pdf.html', 'protect-pdf.html'],
  },
  {
    file: 'compress-image.html', toolId: 'compress', category: 'images', tool: 'Compress Images',
    title: 'Compress images offline: JPG, PNG, WebP | Toolbox',
    description: 'Make JPG, PNG and WebP images smaller in batches on your Mac or Windows PC. Lossless or lossy modes, no upload. Free desktop app.',
    kicker: 'Compress images', heading: 'Smaller images,<br><em>in batches.</em>',
    lede: 'Compress many images at once. Keep a lossless copy, or trade some quality for a smaller file.',
    steps: [
      'Open Toolbox and choose Compress Images.',
      'Add one or more images.',
      'Pick lossless, or a lossy mode for smaller files.',
      'Toolbox saves a compressed copy next to each original.',
    ],
    faqs: [
      ['Can I compress many images at once?', 'Yes. Add a whole batch. If one image fails, the rest still finish.'],
      ['Are my photos uploaded?', privacyAnswer],
      ['Does it change the original images?', originalAnswer],
    ],
    related: ['resize-image.html', 'convert-image.html', 'compress-pdf.html'],
  },
  {
    file: 'resize-image.html', toolId: 'resize', category: 'images', tool: 'Resize Images',
    title: 'Resize an image offline | Toolbox',
    description: 'Resize images by pixels, percentage or longest side and keep the aspect ratio. Free desktop app for Mac and Windows with no upload.',
    kicker: 'Resize images', heading: 'Resize an image<br><em>to the exact size.</em>',
    lede: 'Scale an image by pixels, by percentage, or by its longest side. Toolbox keeps the aspect ratio.',
    steps: [
      'Open Toolbox and choose Resize Images.',
      'Add the image.',
      'Enter a pixel size, a percentage, or a longest side.',
      'Toolbox saves the resized copy next to the original.',
    ],
    faqs: [
      ['Will my image be stretched?', 'No. Toolbox keeps the aspect ratio when it resizes.'],
      ['Can I also crop or rotate?', 'Yes. Use Crop Images or Rotate and Flip Images.'],
      ['Is my image uploaded?', privacyAnswer],
    ],
    related: ['compress-image.html', 'convert-image.html', 'jpg-to-pdf.html'],
  },
  {
    file: 'convert-image.html', toolId: 'heic-convert', category: 'images', tool: 'Convert Image Format',
    title: 'Convert HEIC to JPG, PNG or WebP offline | Toolbox',
    description: 'Convert images between PNG, JPEG, WebP and HEIC, including iPhone HEIC to JPG. Free desktop app for Mac and Windows with no upload.',
    kicker: 'Convert images', heading: 'HEIC, JPG, PNG, WebP.<br><em>Any direction.</em>',
    lede: 'Convert an image between PNG, JPEG, WebP and HEIC, including iPhone HEIC photos to JPG.',
    steps: [
      'Open Toolbox and choose Convert Image Format.',
      'Add the image.',
      'Pick PNG, JPEG, WebP or HEIC as the output.',
      'Toolbox saves the converted copy next to the original.',
    ],
    faqs: [
      ['Can I convert iPhone HEIC photos to JPG?', 'Yes. Choose JPEG as the output format.'],
      ['Which formats are supported?', 'PNG, JPEG, WebP and HEIC.'],
      ['Is my image uploaded?', privacyAnswer],
    ],
    related: ['resize-image.html', 'compress-image.html', 'jpg-to-pdf.html'],
  },
  {
    file: 'offline-pdf-tools.html', category: 'pdf', tool: 'Toolbox',
    title: 'Offline alternative to iLovePDF and Smallpdf | Toolbox',
    description: 'Free desktop alternative to online PDF and image sites such as iLovePDF and Smallpdf. Merge, split, compress, unlock and convert files without uploading them.',
    kicker: 'Offline PDF tools', heading: 'The PDF tools you use online,<br><em>without the upload.</em>',
    lede: 'Toolbox is a free desktop app with the everyday tasks of online PDF and image sites. Your files are processed on your computer.',
    steps: [
      'Download Toolbox for macOS, or get it from the Microsoft Store on Windows.',
      'Pick a tool: merge, split, compress, sign, unlock, convert or resize.',
      'Add your files. There is no account, upload or daily limit.',
      'Toolbox saves the results next to your originals.',
    ],
    faqs: [
      ['Is Toolbox related to iLovePDF or Smallpdf?', 'No. Toolbox is an independent, open source project. Those names belong to their owners.'],
      ['How is it different from online PDF sites?', 'Toolbox runs on your computer, so it never uploads your files and needs no account.'],
      ['Is it free?', platformAnswer],
      ['Does it change my original files?', originalAnswer],
    ],
    related: ['remove-pdf-password.html', 'compress-pdf.html', 'convert-image.html'],
  },
];

const html = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const jsonLd = data => JSON.stringify(data, null, 2).replaceAll('</', '<\\/');
const pageUrl = file => `${site}/${file}`;
const byFile = new Map(landingPages.map(page => [page.file, page]));

const header = `<a class="skip" href="#main">Skip to content</a>
<header class="nav"><div class="wrap nav-inner">
  <a class="brand" href="index.html"><img class="brand-mark" src="assets/app-icon.png" alt="" width="28" height="28"><span class="brand-name">Toolbox</span></a>
  <div class="tool-search nav-search" data-tool-search hidden><label for="tool-search">Search tools</label><svg class="ico" aria-hidden="true" viewBox="0 0 24 24"><circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="2"/><path d="m20 20-3.5-3.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg><input id="tool-search" type="search" placeholder="Search tools" autocomplete="off" spellcheck="false" aria-controls="tool-search-results"><ul id="tool-search-results" class="tool-search-results" hidden></ul><p class="tool-search-status" role="status" aria-live="polite"></p></div>
  <nav class="nav-links" aria-label="Sections"><a href="index.html#tools">Tools</a><a href="pdf.html">PDF tools</a><a href="images.html">Image tools</a><a href="index.html#faq">FAQ</a></nav>
  <div class="nav-actions"><button id="theme-toggle" class="btn btn-ghost btn-icon" type="button" aria-label="Switch to light appearance"><svg class="ico only-dark" aria-hidden="true" viewBox="0 0 24 24"><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M19 5l-1.5 1.5M6.5 17.5 5 19" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="12" r="4" fill="none" stroke="currentColor" stroke-width="2"/></svg><svg class="ico only-light" aria-hidden="true" viewBox="0 0 24 24"><path fill="currentColor" d="M20.2 15.3A8.5 8.5 0 0 1 8.7 3.8 8.5 8.5 0 1 0 20.2 15.3Z"/></svg></button><a class="btn btn-ghost btn-icon" href="https://github.com/lazzyms/toolbox" aria-label="Toolbox on GitHub"><svg class="ico" aria-hidden="true" viewBox="0 0 24 24"><path fill="currentColor" d="M12 .8a11.2 11.2 0 0 0-3.5 21.8c.5.1.7-.2.7-.6v-2.2c-3 .7-3.7-1.4-3.7-1.4-.5-1.3-1.2-1.6-1.2-1.6-1-.7 0-.7 0-.7 1.1.1 1.7 1.1 1.7 1.1 1 1.7 2.6 1.2 3.2.9.1-.7.4-1.2.7-1.5-2.4-.3-5-1.2-5-5.3 0-1.2.4-2.1 1.1-2.9-.1-.3-.5-1.4.1-2.8 0 0 .9-.3 3 1.1a10.4 10.4 0 0 1 5.5 0c2.1-1.4 3-1.1 3-1.1.6 1.4.2 2.5.1 2.8.7.8 1.1 1.7 1.1 2.9 0 4.1-2.6 5-5 5.3.4.4.8 1.1.8 2.2v3.2c0 .4.2.7.8.6A11.2 11.2 0 0 0 12 .8Z"/></svg></a></div>
</div></header>`;

const footer = `<footer class="footer"><div class="wrap footer-inner"><p>Toolbox — MIT licensed.</p><nav aria-label="Footer"><a href="index.html">Overview</a><a href="pdf.html">PDF tools</a><a href="images.html">Image tools</a><a href="privacy.html">Privacy</a><a href="terms.html">Terms</a><a href="https://github.com/lazzyms/toolbox">Source</a><a href="https://github.com/lazzyms/toolbox/issues">Issues</a></nav></div></footer>`;

const chevron = '<svg class="chevron" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>';

export const structuredData = (page, version) => {
  const category = categories[page.category];
  return {
    '@context': 'https://schema.org',
    '@graph': [
      {
        '@type': 'SoftwareApplication',
        name: 'Toolbox',
        url: `${site}/`,
        applicationCategory: 'UtilitiesApplication',
        operatingSystem: 'macOS, Windows',
        downloadUrl: [`https://github.com/lazzyms/toolbox/releases/download/tauri-v${version}/Toolbox-${version}-macos.dmg`, storeUrl],
        offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
        license: 'https://opensource.org/licenses/MIT',
      },
      {
        '@type': 'BreadcrumbList',
        itemListElement: [
          { '@type': 'ListItem', position: 1, name: 'Toolbox', item: `${site}/` },
          { '@type': 'ListItem', position: 2, name: category.name, item: pageUrl(category.file) },
          { '@type': 'ListItem', position: 3, name: page.kicker, item: pageUrl(page.file) },
        ],
      },
      {
        '@type': 'FAQPage',
        mainEntity: page.faqs.map(([name, text]) => ({ '@type': 'Question', name, acceptedAnswer: { '@type': 'Answer', text } })),
      },
    ],
  };
};

export const renderLandingPage = (page, version) => {
  const dmg = `https://github.com/lazzyms/toolbox/releases/download/tauri-v${version}/Toolbox-${version}-macos.dmg`;
  const download = `<div class="cta"><a class="btn btn-default btn-lg" data-dl data-download-platform="macos" href="${dmg}"><svg class="ico" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12m-5-5 5 5 5-5M5 21h14"/></svg>Download for macOS</a><a class="btn btn-outline btn-lg" data-dl data-download-platform="windows" href="${storeUrl}"><svg class="ico" aria-hidden="true" viewBox="0 0 24 24" fill="currentColor"><path d="M3 3h8.5v8.5H3zm9.5 0H21v8.5h-8.5zM3 12.5h8.5V21H3zm9.5 0H21V21h-8.5z"/></svg>Get it from Microsoft Store</a></div>`;
  const meta = [
    `<link rel="canonical" href="${pageUrl(page.file)}">`,
    '<meta property="og:type" content="website">',
    `<meta property="og:title" content="${html(page.title)}">`,
    `<meta property="og:description" content="${html(page.description)}">`,
    `<meta property="og:url" content="${pageUrl(page.file)}">`,
    `<meta property="og:image" content="${site}/og.jpg">`,
    '<meta property="og:image:type" content="image/jpeg">',
    '<meta property="og:image:width" content="1200">',
    '<meta property="og:image:height" content="630">',
    `<meta property="og:image:alt" content="${socialImageAlt}">`,
    '<meta name="twitter:card" content="summary_large_image">',
    `<meta name="twitter:title" content="${html(page.title)}">`,
    `<meta name="twitter:description" content="${html(page.description)}">`,
    `<meta name="twitter:image" content="${site}/og.jpg">`,
    `<meta name="twitter:image:alt" content="${socialImageAlt}">`,
  ].join('\n');
  const steps = page.steps.map(step => `      <li>${html(step)}</li>`).join('\n');
  const faqs = page.faqs.map(([question, answer]) => `      <details>
        <summary>${html(question)}${chevron}</summary>
        <div class="accordion-content"><p>${html(answer)}</p></div>
      </details>`).join('\n');
  const related = page.related.map(file => `<li><a href="${file}">${html(byFile.get(file).kicker)}</a></li>`).join('');
  return `<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${html(page.title)}</title>
<meta name="description" content="${html(page.description)}">
<link rel="icon" href="assets/favicon-32.png" type="image/png" sizes="32x32">
<link rel="apple-touch-icon" href="assets/apple-touch-icon.png">
<link rel="stylesheet" href="assets/styles.css">
${meta}
<meta name="theme-color" content="#08090b">
<script type="application/ld+json">
${jsonLd(structuredData(page, version))}
</script>
</head>
<body>
${header}
<main id="main">
  <section class="category-hero"><div class="wrap category-hero-inner"><a class="back-link" href="${categories[page.category].file}">← ${categories[page.category].name}</a><p class="kicker">${html(page.kicker)}</p><h1>${page.heading}</h1><p class="lede">${html(page.lede)}</p>${download}<p class="cta-note">Free · No upload, no account</p></div></section>
  <section class="section"><div class="wrap narrow">
    <h2>How to ${html(page.kicker.charAt(0).toLowerCase() + page.kicker.slice(1))} with Toolbox</h2>
    <ol class="task-steps">
${steps}
    </ol>
    <h2 class="task-faq-title">Questions</h2>
    <div class="accordion">
${faqs}
    </div>
    <h2 class="task-related-title">Related</h2>
    <ul class="task-links">${related}</ul>
  </div></section>
</main>
${footer}
<script src="assets/app.js"></script>
</body>
</html>
`;
};

export const sitePages = () => readdirSync(docs).filter(name => name.endsWith('.html')).sort();

export const renderSitemap = pages => `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${pages.map(file => `  <url><loc>${file === 'index.html' ? `${site}/` : pageUrl(file)}</loc></url>`).join('\n')}
</urlset>
`;

export const robots = `# Search engines and AI assistants are welcome to read and cite this site.
User-agent: *
Allow: /

Sitemap: ${site}/sitemap.xml
`;

export const renderLlms = () => `# Toolbox

> Toolbox is a free, open source (MIT) desktop app for macOS (Apple silicon) and Windows with local PDF, Office and image tools. It processes files on the user's own computer: it never uploads files, needs no account, and saves results beside the originals without overwriting them.

Download: ${site}/ (macOS DMG from GitHub releases; Windows from the Microsoft Store: ${storeUrl})
Source: https://github.com/lazzyms/toolbox

## Tasks

${landingPages.map(page => `- [${page.kicker}](${pageUrl(page.file)}): ${page.description}`).join('\n')}

## Tool lists

- [PDF and document tools](${site}/pdf.html): remove a known password, protect, merge, split, compress, sign, edit, watermark, crop, organize, add page numbers, convert PDF to images or text, images to PDF.
- [Image tools](${site}/images.html): convert between PNG, JPEG, WebP and HEIC, compress, resize, crop, rotate, watermark, strip metadata, create GIFs, generate app icons.

## Limits

- Remove Password needs the password the user already knows. It cannot recover or bypass passwords.
- Sign PDF places a visible signature, not a cryptographic one.
- OCR, background removal and face blurring are not available in the current build.

## Policies

- [Privacy](${site}/privacy.html)
- [Terms](${site}/terms.html)
`;

// Turns each tool card on the category pages into a link to that tool's guide, and gives every card an anchor.
const byToolId = new Map(landingPages.filter(page => page.toolId).map(page => [page.toolId, page]));
const toolCard = /<(?:article|a) class="feature-card"[^>]*data-tool-id="([^"]+)"[^>]*>([\s\S]*?)<\/(?:article|a)>/g;
export const linkToolCards = source => source.replace(toolCard, (_, id, body) => {
  const guide = byToolId.get(id);
  return guide
    ? `<a class="feature-card" id="tool-${id}" href="${guide.file}" data-tool-id="${id}">${body}</a>`
    : `<article class="feature-card" id="tool-${id}" data-tool-id="${id}">${body}</article>`;
});

// The site search index: one entry per tool card, pointing at its guide or its card.
const text = value => value.replace(/<[^>]+>/g, '').replaceAll('&amp;', '&').trim();
export const toolIndex = pages => JSON.stringify(Object.entries(pages).flatMap(([file, source]) => [...source.matchAll(toolCard)].map(([, id, body]) => ({
  name: text(body.match(/<h2>([\s\S]*?)<\/h2>/)[1]),
  label: text(body.match(/<p class="feature-label">([\s\S]*?)<\/p>/)[1]),
  description: text(body.match(/<p>([\s\S]*?)<\/p>$/)[1]),
  href: byToolId.get(id)?.file ?? `${file}#tool-${id}`,
}))), null, 1) + '\n';

export const buildSite = version => {
  const files = new Map(landingPages.map(page => [page.file, renderLandingPage(page, version)]));
  for (const file of ['pdf.html', 'images.html']) files.set(file, linkToolCards(readFileSync(new URL(file, docs), 'utf8')));
  files.set('assets/tools.json', toolIndex({ 'pdf.html': files.get('pdf.html'), 'images.html': files.get('images.html') }));
  const pages = [...new Set([...sitePages(), ...files.keys()])].filter(file => file.endsWith('.html')).sort();
  files.set('sitemap.xml', renderSitemap(pages));
  files.set('robots.txt', robots);
  files.set('llms.txt', renderLlms());
  return files;
};

export const releaseVersion = () => JSON.parse(readFileSync(new URL('../tauri-latest.json', docs), 'utf8')).version;

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  for (const [file, source] of buildSite(releaseVersion())) writeFileSync(new URL(file, docs), source);
}
