/* Toolbox — landing page behaviour.
   Plain ES2020, no dependencies, no bundler. Everything degrades: with JS off you
   still get the full page and a download link pointing at the releases index. */

(() => {
  'use strict';

  const REPO = 'lazzyms/toolbox';
  const $  = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ───────────────────────── appearance ───────────────────────── */

  const root = document.documentElement;
  const stored = localStorage.getItem('toolbox-theme');
  const setTheme = (theme) => {
    root.dataset.theme = theme;
    $('#theme-toggle')?.setAttribute(
      'aria-label',
      `Switch to ${theme === 'dark' ? 'light' : 'dark'} appearance`
    );
    // Matches --background in styles.css: oklch(0.145 0 0) / oklch(1 0 0).
    $('meta[name="theme-color"]')?.setAttribute(
      'content',
      theme === 'dark' ? '#0a0a0a' : '#ffffff'
    );
  };

  // ?theme=light|dark forces an appearance, which makes the page easy to preview
  // and screenshot either way without touching system settings.
  const forced = new URLSearchParams(location.search).get('theme');
  setTheme(
    (forced === 'light' || forced === 'dark' ? forced : null) ||
    stored ||
    (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')
  );

  $('#theme-toggle')?.addEventListener('click', () => {
    const next = root.dataset.theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    localStorage.setItem('toolbox-theme', next);
  });

  // Follow the system only while the visitor hasn't expressed a preference.
  matchMedia('(prefers-color-scheme: light)').addEventListener('change', (e) => {
    if (!localStorage.getItem('toolbox-theme')) setTheme(e.matches ? 'light' : 'dark');
  });

  /* ───────────────────── direct .dmg download link ─────────────────────
     The DMG asset is named Toolbox-<version>.dmg, so there is no stable static
     URL for it. Resolve the real asset from the releases API and point the
     buttons straight at it; the markup's /releases/latest href is the fallback. */

  const formatBytes = (bytes) => {
    if (!bytes) return null;
    const mb = bytes / 1_048_576;
    return mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
  };

  async function resolveDownload() {
    const buttons = $$('[data-dl]');
    const note = $('#dl-note');
    if (!buttons.length) return;

    try {
      const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
        headers: { Accept: 'application/vnd.github+json' },
      });

      if (res.status === 404) {
        if (note) {
          note.innerHTML =
            'No release has been published yet — ' +
            '<a href="#build">build it from source</a> in the meantime.';
        }
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);

      const release = await res.json();
      const dmg = (release.assets || []).find((a) => a.name.toLowerCase().endsWith('.dmg'));
      if (!dmg) throw new Error('no dmg asset');

      const size = formatBytes(dmg.size);
      for (const btn of buttons) {
        btn.href = dmg.browser_download_url;      // direct download, no interstitial
        btn.setAttribute('download', dmg.name);
        const meta = $('.meta', btn);
        if (meta) meta.textContent = size ? `${release.tag_name} · ${size}` : release.tag_name;
      }
      if (note) {
        note.innerHTML =
          `<code>${dmg.name}</code> · Apple silicon &amp; Intel · ` +
          `<a href="https://github.com/${REPO}/releases">all releases</a>`;
      }
    } catch {
      // Rate-limited or offline: the static href already works, so say nothing loud.
      if (note) {
        note.innerHTML =
          `<a href="https://github.com/${REPO}/releases/latest">Open the latest release</a> ` +
          'to pick the DMG.';
      }
    }
  }

  resolveDownload();

  /* ───────────────────────── nav chrome ───────────────────────── */

  const nav = $('.nav');
  const onScroll = () => nav?.classList.toggle('stuck', window.scrollY > 8);
  onScroll();
  addEventListener('scroll', onScroll, { passive: true });

  const links = $$('.nav-links a');
  const sections = links
    .map((a) => $(a.getAttribute('href')))
    .filter(Boolean);

  if (sections.length && 'IntersectionObserver' in window) {
    const spy = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          for (const a of links) {
            a.setAttribute('aria-current', String(a.getAttribute('href') === `#${entry.target.id}`));
          }
        }
      },
      { rootMargin: '-45% 0px -50% 0px' }
    );
    sections.forEach((s) => spy.observe(s));
  }

  /* ───────────────────────── copy buttons ───────────────────────── */

  for (const btn of $$('.copy')) {
    btn.addEventListener('click', async () => {
      const source = $(btn.dataset.copy);
      if (!source) return;
      try {
        await navigator.clipboard.writeText(source.innerText.trim());
        const original = btn.textContent;
        btn.textContent = 'Copied';
        btn.classList.add('ok');
        setTimeout(() => { btn.textContent = original; btn.classList.remove('ok'); }, 1600);
      } catch {
        btn.textContent = 'Press ⌘C';
      }
    });
  }

  /* ───────────────────────── scroll reveal ───────────────────────── */

  /* The reveal is decoration, so it must never be the reason content is missing.
     Anything still hidden after a grace period is shown unconditionally, and the
     class is only ever added when we know an observer is there to remove it. */
  const revealable = $$('.section > .wrap > *, .closer .wrap > *');
  if (!reduceMotion && 'IntersectionObserver' in window) {
    const show = (el) => {
      el.classList.add('in');
      el.style.transitionDelay = '';
    };

    const io = new IntersectionObserver(
      (entries, obs) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          show(entry.target);
          obs.unobserve(entry.target);
        }
      },
      { rootMargin: '0px 0px -8% 0px' }
    );

    revealable.forEach((el, i) => {
      el.classList.add('reveal');
      el.style.transitionDelay = `${Math.min(i % 6, 5) * 45}ms`;
      io.observe(el);
    });

    // Failsafe: if anything is still hidden shortly after load — deep link, an
    // observer that never fired, a restored scroll position — just reveal it.
    const failsafe = () => setTimeout(() => {
      revealable.forEach((el) => { if (!el.classList.contains('in')) show(el); });
      io.disconnect();
    }, 1500);

    if (document.readyState === 'complete') failsafe();
    else addEventListener('load', failsafe, { once: true });
  }

  /* ═════════════════════════ the app demo ═════════════════════════
     A recreation of the real UI: the sidebar registry from Utility.swift, the
     drop zone, each tool's option rows, and the run bar with its batch progress. */

  const TOOLS = [
    {
      id: 'pdf-unlock',
      category: 'pdf',
      icon: 'i-lock-open',
      title: 'Remove PDF Password',
      blurb: 'Save an unlocked copy of a PDF you know the password for.',
      prompt: 'Drop password-protected PDFs here',
      run: 'Remove Password',
      suffix: '-unlocked',
      files: [
        { name: 'bank-statement.pdf', size: '2.4 MB' },
        { name: 'contract-signed.pdf', size: '860 KB' },
        { name: 'payslip-april.pdf',   size: '1.1 MB' },
      ],
      options: [
        { kind: 'password', label: 'Password' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-page-numbers',
      category: 'pdf',
      icon: 'i-doc-safe',
      title: 'Add PDF Page Numbers',
      blurb: 'Stamp page numbers onto a PDF.',
      prompt: 'Drop PDFs to number',
      run: 'Add Page Numbers',
      suffix: '-numbered',
      files: [
        { name: 'q3-handbook.pdf',     size: '6.2 MB' },
        { name: 'onboarding-deck.pdf', size: '3.4 MB' },
        { name: 'price-list.pdf',      size: '900 KB' },
      ],
      options: [
        { kind: 'segment', label: 'Position', choices: ['Bottom centre', 'Bottom right', 'Top right'], value: 0 },
        { kind: 'check',   label: 'Cover', text: 'Skip the first page' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-merge',
      category: 'pdf',
      icon: 'i-stack',
      title: 'Merge PDF',
      blurb: 'Combine several PDFs into one file.',
      prompt: 'Drop the PDFs to combine, in order',
      run: 'Merge',
      suffix: '-merged',
      files: [
        { name: 'chapter-1.pdf',  size: '1.2 MB' },
        { name: 'chapter-2.pdf',  size: '1.8 MB' },
        { name: 'appendix-a.pdf', size: '420 KB' },
      ],
      options: [
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-watermark',
      category: 'pdf',
      icon: 'i-stack',
      title: 'Watermark PDF',
      blurb: 'Stamp text or an image across pages — DRAFT, CONFIDENTIAL, a logo.',
      prompt: 'Drop PDFs to watermark',
      run: 'Add Watermark',
      suffix: '-watermarked',
      files: [
        { name: 'invoice-2026-08.pdf',  size: '88 KB' },
        { name: 'pitch-deck-draft.pdf', size: '5.1 MB' },
        { name: 'terms-of-service.pdf', size: '210 KB' },
      ],
      options: [
        { kind: 'segment', label: 'Source', choices: ['Text', 'Image'], value: 0 },
        { kind: 'slider',  label: 'Opacity', value: 40 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-crop',
      category: 'pdf',
      icon: 'i-resize',
      title: 'Crop PDF',
      blurb: 'Trim margins or cut to a region — losslessly.',
      prompt: 'Drop PDFs to crop',
      run: 'Crop',
      suffix: '-cropped',
      files: [
        { name: 'scanned-contract.pdf', size: '12.4 MB' },
        { name: 'lecture-notes.pdf',    size: '8.7 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Method', choices: ['Auto margins', 'Fixed box'], value: 0 },
        { kind: 'size',    label: 'Box size' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-protect',
      category: 'pdf',
      icon: 'i-shield',
      title: 'Protect PDF',
      blurb: 'Add a password so only you can open it.',
      prompt: 'Drop PDFs to protect',
      run: 'Add Password',
      suffix: '-protected',
      files: [
        { name: 'tax-return-2025.pdf', size: '640 KB' },
        { name: 'lease-agreement.pdf', size: '1.3 MB' },
        { name: 'medical-record.pdf',  size: '380 KB' },
      ],
      options: [
        { kind: 'password', label: 'Password' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'images-to-pdf',
      category: 'pdf',
      icon: 'i-stack',
      title: 'Images to PDF',
      blurb: 'Turn photos and scans into one PDF — HEIC included.',
      prompt: 'Drop photos and scans to combine',
      run: 'Create PDF',
      outExt: 'pdf',
      files: [
        { name: 'IMG_4821.heic',        size: '3.8 MB' },
        { name: 'scan-receipt.jpg',     size: '1.2 MB' },
        { name: 'whiteboard-photo.png', size: '2.6 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Page size', choices: ['Fit image', 'A4', 'Letter'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-to-images',
      category: 'pdf',
      icon: 'i-convert',
      title: 'PDF to Images',
      blurb: 'Render pages to JPEG or PNG at 72–300 dpi.',
      prompt: 'Drop PDFs to render as images',
      run: 'Render Pages',
      suffix: '-page-1',
      outExt: 'png',
      files: [
        { name: 'user-guide.pdf',     size: '9.8 MB' },
        { name: 'slide-printout.pdf', size: '2.2 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Format', choices: ['PNG', 'JPEG'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-to-text',
      category: 'pdf',
      icon: 'i-doc-safe',
      title: 'PDF to Text',
      blurb: 'Pull the text out as .txt or best-effort Markdown.',
      prompt: 'Drop PDFs to extract text from',
      run: 'Extract Text',
      suffix: '-text',
      outExt: 'txt',
      files: [
        { name: 'annual-report.pdf',  size: '4.6 MB' },
        { name: 'research-paper.pdf', size: '1.9 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Format', choices: ['Plain text', 'Markdown'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-split',
      category: 'pdf',
      icon: 'i-stack',
      title: 'Split PDF',
      blurb: 'Break one PDF into several — every page, by ranges, or fixed-size chunks.',
      prompt: 'Drop a PDF to split',
      run: 'Split',
      suffix: '-page-1',
      files: [
        { name: 'thesis-final.pdf',    size: '14.2 MB' },
        { name: 'course-handbook.pdf', size: '8.3 MB' },
        { name: 'zine-issue-4.pdf',    size: '22.5 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Mode', choices: ['Every page', 'Ranges', 'Chunks of 10'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-image-extract',
      category: 'pdf',
      icon: 'i-download',
      title: 'Extract Images from PDF',
      blurb: 'Pull embedded pictures out at their original resolution — JPEGs stay untouched.',
      prompt: 'Drop PDFs to pull pictures from',
      run: 'Extract Images',
      files: [
        { name: 'portfolio-2026.pdf', size: '22.7 MB' },
        { name: 'press-kit.pdf',      size: '6.4 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Format', choices: ['Original', 'JPEG'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-sign',
      category: 'pdf',
      icon: 'i-verified',
      title: 'Sign PDF',
      blurb: 'Stamp your signature image or typed name onto pages — visual only, not cryptographic.',
      prompt: 'Drop PDFs to sign',
      run: 'Sign',
      suffix: '-signed',
      files: [
        { name: 'nda-acme.pdf',     size: '310 KB' },
        { name: 'offer-letter.pdf', size: '190 KB' },
      ],
      options: [
        { kind: 'segment', label: 'Signature', choices: ['Image', 'Typed'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-ocr',
      category: 'pdf',
      icon: 'i-doc-safe',
      title: 'OCR PDF',
      blurb: 'Read text out of scans with on-device OCR — outputs a .txt file.',
      prompt: 'Drop scanned PDFs',
      run: 'Run OCR',
      suffix: '-ocr-text',
      outExt: 'txt',
      files: [
        { name: 'scanned-invoice.pdf',  size: '7.9 MB' },
        { name: 'grandmas-recipes.pdf', size: '11.3 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Language', choices: ['English', 'Automatic'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-remove-pages',
      category: 'pdf',
      icon: 'i-compress',
      title: 'Remove PDF Pages',
      blurb: 'Delete selected pages from a PDF — the rest stay, in order.',
      prompt: 'Drop a PDF to delete pages from',
      run: 'Remove Pages',
      suffix: '-trimmed',
      files: [
        { name: 'meeting-pack-full.pdf', size: '5.6 MB' },
        { name: 'onboarding-v7.pdf',     size: '3.1 MB' },
      ],
      options: [
        { kind: 'check', label: 'Selection', text: 'Invert — keep the selected pages instead' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-extract-pages',
      category: 'pdf',
      icon: 'i-download',
      title: 'Extract PDF Pages',
      blurb: 'Pull selected pages out into a new PDF — ranges like 1-3, 7.',
      prompt: 'Drop a PDF to pull pages from',
      run: 'Extract Pages',
      suffix: '-pages',
      files: [
        { name: 'course-reader.pdf', size: '18.9 MB' },
        { name: 'annual-filing.pdf', size: '9.4 MB' },
      ],
      options: [
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-organize',
      category: 'pdf',
      icon: 'i-stack',
      title: 'Organize PDF',
      blurb: 'Reorder, rotate and delete pages of one PDF from a thumbnail grid.',
      prompt: 'Drop a PDF to rearrange',
      run: 'Organize',
      suffix: '-organized',
      files: [
        { name: 'photo-album-draft.pdf', size: '16.1 MB' },
        { name: 'grant-application.pdf', size: '4.8 MB' },
      ],
      options: [
        { kind: 'destination' },
      ],
    },
    {
      id: 'pdf-compress',
      category: 'pdf',
      icon: 'i-compress',
      title: 'Compress PDF',
      blurb: 'Shrink scan-heavy PDFs by rasterising pages as JPEG — text becomes pixels.',
      prompt: 'Drop PDFs to shrink',
      run: 'Compress',
      suffix: '-compressed',
      showSavings: true,
      files: [
        { name: 'insurance-scan.pdf', size: '24.8 MB', after: '6.1 MB' },
        { name: 'archive-2004.pdf',   size: '31.5 MB', after: '7.4 MB' },
      ],
      options: [
        { kind: 'slider', label: 'Quality', value: 60 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'heic-convert',
      category: 'images',
      icon: 'i-convert',
      title: 'Convert Image Format',
      blurb: 'HEIC to PNG, JPEG and back — batch friendly.',
      prompt: 'Drop HEIC, PNG, JPEG, TIFF or RAW images here',
      run: 'Convert to PNG',
      convert: true,
      files: [
        { name: 'IMG_4821.heic', size: '3.8 MB' },
        { name: 'IMG_4822.heic', size: '4.1 MB' },
        { name: 'DSC_0194.dng',  size: '24.6 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Format', choices: ['PNG', 'JPEG', 'HEIC', 'TIFF'], value: 0 },
        { kind: 'slider',  label: 'Quality', value: 90 },
        { kind: 'check',   label: 'Metadata', text: 'Remove EXIF and location data' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'compress',
      category: 'images',
      icon: 'i-compress',
      title: 'Compress Images',
      blurb: 'Shrink files losslessly, or trade quality for size.',
      prompt: 'Drop images to make smaller',
      run: 'Compress',
      suffix: '-compressed',
      showSavings: true,
      files: [
        { name: 'screenshot-1.png', size: '1.9 MB', after: '612 KB' },
        { name: 'screenshot-2.png', size: '2.3 MB', after: '741 KB' },
        { name: 'hero-banner.jpg',  size: '4.4 MB', after: '1.2 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Mode', choices: ['Lossless', 'Lossy'], value: 1,
          hints: ['Re-encodes without touching a single pixel.',
                  'Trades detail for size using the quality slider.'] },
        { kind: 'slider', label: 'Quality', value: 80 },
        { kind: 'check',  label: 'Metadata', text: 'Remove EXIF and location data' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'resize',
      category: 'images',
      icon: 'i-resize',
      title: 'Resize Images',
      blurb: 'Scale by pixels, percentage or longest side.',
      prompt: 'Drop images to resize',
      run: 'Resize',
      suffix: '-resized',
      files: [
        { name: 'IMG_5001.jpeg', size: '4.2 MB' },
        { name: 'IMG_5002.jpeg', size: '3.9 MB' },
        { name: 'poster.png',    size: '8.1 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Method',
          choices: ['Fit', 'Longest side', 'Percentage', 'Exact'], value: 0,
          hints: [
            'Scales down to fit inside the box. Aspect ratio is preserved — leave a field empty to leave that side unconstrained.',
            'Handles portrait and landscape in one batch — whichever side is longer becomes this many pixels.',
            'Scales both sides by the same percentage of the original.',
            'Exact pixel size. Distorts unless the ratio happens to match.',
          ] },
        { kind: 'size', label: 'Max size' },
        { kind: 'check', label: 'Upscaling', text: 'Allow enlarging images smaller than the target' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'rotate',
      category: 'images',
      icon: 'i-rotate',
      title: 'Rotate & Flip Images',
      blurb: 'Quarter turns and mirrors, without resampling — batch friendly.',
      prompt: 'Drop images to rotate or flip',
      run: 'Rotate & Flip',
      suffix: '-rotated',
      files: [
        { name: 'IMG_5102.jpeg', size: '3.1 MB' },
        { name: 'IMG_5103.jpeg', size: '2.8 MB' },
        { name: 'panorama.jpg',  size: '5.4 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Turn', choices: ['90° CW', '90° CCW', '180°', 'Flip H', 'Flip V'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'crop',
      category: 'images',
      icon: 'i-resize',
      title: 'Crop Images',
      blurb: 'Cut to an anchored aspect ratio or a fixed pixel rectangle, in a batch.',
      prompt: 'Drop images to crop',
      run: 'Crop',
      suffix: '-cropped',
      files: [
        { name: 'profile-shot.jpg', size: '2.2 MB' },
        { name: 'banner-art.png',   size: '6.8 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Aspect', choices: ['Square', '4:3', '16:9', 'Original'], value: 0 },
        { kind: 'size',    label: 'Max size' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'icon-set',
      category: 'images',
      icon: 'i-beaker',
      title: 'Generate App Icons',
      blurb: 'Turn one image into a complete macOS, favicon, iOS or Android icon set.',
      prompt: 'Drop a square source image',
      run: 'Generate Icons',
      files: [
        { name: 'app-icon-source.png', size: '2.3 MB' },
        { name: 'logo-mark.png',       size: '480 KB' },
      ],
      options: [
        { kind: 'segment', label: 'Target', choices: ['macOS', 'Favicon', 'iOS', 'Android'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'gif-create',
      category: 'images',
      icon: 'i-stack',
      title: 'Create GIF',
      blurb: 'Animate a batch of still images into a looped GIF.',
      prompt: 'Drop still frames in sequence',
      run: 'Create GIF',
      suffix: '-animated',
      files: [
        { name: 'jump-frame-01.png', size: '820 KB' },
        { name: 'jump-frame-02.png', size: '790 KB' },
        { name: 'jump-frame-03.png', size: '810 KB' },
      ],
      options: [
        { kind: 'segment', label: 'Frame rate', choices: ['10 fps', '15 fps', '25 fps'], value: 1 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'gif-extract',
      category: 'images',
      icon: 'i-stack',
      title: 'Extract GIF Frames',
      blurb: 'Split an animated GIF into its individual frames.',
      prompt: 'Drop an animated GIF',
      run: 'Extract Frames',
      suffix: '-frame-1',
      outExt: 'png',
      files: [
        { name: 'cat-reaction.gif',     size: '3.4 MB' },
        { name: 'goal-celebration.gif', size: '8.7 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Step', choices: ['Every frame', 'Every 2nd', 'Every 3rd'], value: 0 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'image-watermark',
      category: 'images',
      icon: 'i-stack',
      title: 'Watermark Images',
      blurb: 'Stamp text or a logo across a batch of images.',
      prompt: 'Drop images to watermark',
      run: 'Watermark',
      suffix: '-watermarked',
      files: [
        { name: 'product-01.jpg', size: '1.4 MB' },
        { name: 'product-02.jpg', size: '1.6 MB' },
        { name: 'team-photo.png', size: '4.9 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Source', choices: ['Text', 'Logo'], value: 0 },
        { kind: 'slider',  label: 'Opacity', value: 50 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'image-metadata',
      category: 'images',
      icon: 'i-gauge',
      title: 'Image Metadata',
      blurb: 'See what a photo leaks, then strip EXIF and GPS without recompressing it.',
      prompt: 'Drop photos to inspect',
      run: 'Strip Metadata',
      suffix: '-stripped',
      files: [
        { name: 'beach-sunset.jpg', size: '4.2 MB' },
        { name: 'IMG_0999.heic',    size: '2.7 MB' },
      ],
      options: [
        { kind: 'check', label: 'EXIF', text: 'Strip camera, lens and settings data' },
        { kind: 'check', label: 'GPS',  text: 'Strip location coordinates' },
        { kind: 'destination' },
      ],
    },
    {
      id: 'image-tone',
      category: 'images',
      icon: 'i-gauge',
      title: 'Colour & Tone Adjustments',
      blurb: 'Batch brightness, contrast, saturation, exposure and one-tap presets.',
      prompt: 'Drop photos to adjust',
      run: 'Apply Adjustments',
      suffix: '-adjusted',
      files: [
        { name: 'bride-portrait.jpg', size: '5.2 MB' },
        { name: 'reception-hall.jpg', size: '4.8 MB' },
        { name: 'golden-hour.jpg',    size: '3.3 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Preset', choices: ['None', 'Vivid', 'Noir', 'Warm'], value: 0 },
        { kind: 'slider',  label: 'Saturation', value: 100 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'tiff-pages',
      category: 'images',
      icon: 'i-stack',
      title: 'Split & Combine TIFF',
      blurb: 'Break a multi-page TIFF into images, or bind a batch of images into one.',
      prompt: 'Drop TIFFs or a folder of images',
      run: 'Split TIFF',
      suffix: '-frame-1',
      outExt: 'png',
      files: [
        { name: 'microfilm-scan.tiff', size: '48.6 MB' },
        { name: 'xray-series.tiff',    size: '22.1 MB' },
      ],
      options: [
        { kind: 'segment', label: 'Mode', choices: ['Split', 'Combine'], value: 0,
          hints: ['Writes each page out as its own image.',
                  'Binds every dropped image into one multi-page TIFF.'] },
        { kind: 'destination' },
      ],
    },
    {
      id: 'image-blur-faces',
      category: 'images',
      icon: 'i-shield',
      title: 'Blur Faces',
      blurb: 'Detect faces on-device and blur them — photos never leave this Mac.',
      prompt: 'Drop photos containing people',
      run: 'Blur Faces',
      suffix: '-blurred',
      files: [
        { name: 'school-photo.jpg', size: '3.6 MB' },
        { name: 'street-scene.jpg', size: '5.1 MB' },
      ],
      options: [
        { kind: 'slider', label: 'Strength', value: 70 },
        { kind: 'destination' },
      ],
    },
    {
      id: 'image-remove-bg',
      category: 'images',
      icon: 'i-beaker',
      title: 'Remove Background',
      blurb: 'Lift the subject out of a photo into a transparent PNG — on-device with Vision.',
      prompt: 'Drop photos with a clear subject',
      run: 'Remove Background',
      suffix: '-cutout',
      outExt: 'png',
      files: [
        { name: 'headshot-white-bg.jpg', size: '1.8 MB' },
        { name: 'product-on-desk.jpg',   size: '2.9 MB' },
      ],
      options: [
        { kind: 'destination' },
      ],
    },
  ];

  const demo = $('#demo');
  if (!demo) return;

  const detail = $('.detail', demo);
  const state = { id: TOOLS[0].id, phase: 'idle', progress: 0, done: 0 };
  let timers = [];

  const clearTimers = () => { timers.forEach(clearTimeout); timers = []; };
  const current = () => TOOLS.find((t) => t.id === state.id);
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  /* sidebar */
  for (const list of $$('.side-list', demo)) {
    const category = list.dataset.category;
    list.innerHTML = TOOLS.filter((t) => t.category === category).map((t) => `
      <li>
        <button type="button" data-tool="${t.id}">
          <svg class="ico" aria-hidden="true"><use href="#${t.icon}"/></svg>
          <span>${esc(t.title)}</span>
        </button>
      </li>`).join('');
  }

  $$('[data-tool]', demo).forEach((btn) => {
    btn.addEventListener('click', () => {
      // Switching tools resets the pane, mirroring the .id(selected.id) reset in the app.
      clearTimers();
      Object.assign(state, { id: btn.dataset.tool, phase: 'idle', progress: 0, done: 0 });
      render();
    });
  });

  function optionMarkup(tool, opt, index) {
    switch (opt.kind) {
      case 'password':
        return `<div class="opt">
            <label for="d-pw">${opt.label}</label>
            <input class="input" id="d-pw" type="password" value="hunter2" style="width:8.75rem" readonly>
          </div>`;

      case 'segment': {
        const buttons = opt.choices.map((c, i) =>
          `<button type="button" data-opt="${index}" data-choice="${i}"
             aria-pressed="${i === opt.value}">${esc(c)}</button>`).join('');
        const hint = opt.hints?.[opt.value]
          ? `<p class="hint">${esc(opt.hints[opt.value])}</p>` : '';
        return `<div class="opt">
            <label>${opt.label}</label>
            <div class="tabs">${buttons}</div>
          </div>${hint}`;
      }

      case 'slider':
        return `<div class="opt">
            <label for="d-q${index}">${opt.label}</label>
            <input class="slider" id="d-q${index}" type="range" min="10" max="100"
                   value="${opt.value}" data-opt="${index}">
            <span class="val">${opt.value}%</span>
          </div>`;

      case 'size':
        return `<div class="opt">
            <label>${opt.label}</label>
            <input class="input" type="text" value="2000" aria-label="Width" readonly style="width:4rem">
            <span aria-hidden="true">×</span>
            <input class="input" type="text" value="2000" aria-label="Height" readonly style="width:4rem">
            <span class="val">px</span>
          </div>`;

      case 'check':
        return `<div class="opt">
            <label>${opt.label}</label>
            <label class="check"><input class="checkbox" type="checkbox"> ${esc(opt.text)}</label>
          </div>`;

      case 'destination':
        return `<div class="opt">
            <label>Save to</label>
            <div class="tabs">
              <button type="button" aria-pressed="true">Next to originals</button>
              <button type="button" aria-pressed="false">Choose folder…</button>
            </div>
          </div>`;

      default:
        return '';
    }
  }

  function fileMarkup(tool) {
    const format = tool.convert
      ? tool.options[0].choices[tool.options[0].value].toLowerCase().replace('jpeg', 'jpg')
      : null;

    return tool.files.map((file, i) => {
      const finished = i < state.done;
      const base = file.name.replace(/\.[^.]+$/, '');
      const ext = file.name.split('.').pop();
      const outName = format ? `${base}.${format}` : `${base}${tool.suffix || ''}.${tool.outExt || ext}`;
      const size = finished && tool.showSavings
        ? `${file.size} → ${file.after}`
        : file.size;

      return `<div class="filerow${finished ? ' done' : ''}" style="animation-delay:${i * 55}ms">
          <span class="state" aria-hidden="true">${finished
            ? '<svg class="ico" style="width:.75rem;height:.75rem"><use href="#i-check"/></svg>'
            : '·'}</span>
          <span class="name">${esc(finished ? outName : file.name)}</span>
          <span class="size">${esc(size)}</span>
        </div>`;
    }).join('');
  }

  function render() {
    const tool = current();
    const showFiles = state.phase !== 'idle';
    const running = state.phase === 'running';
    const total = tool.files.length;

    const runLabel = tool.convert
      ? `Convert to ${tool.options[0].choices[tool.options[0].value]}`
      : tool.run;

    const status = running
      ? `<div class="progress"><i style="width:${Math.round(state.progress * 100)}%"></i></div>
         <span class="status">Working…</span>`
      : state.phase === 'done'
        ? `<span class="status">${total} of ${total} succeeded</span>`
        : showFiles
          ? `<span class="status">${total} files ready</span>`
          : '';

    // Keep the sidebar highlight in step with the selected tool.
    for (const btn of $$('[data-tool]', demo)) {
      btn.setAttribute('aria-current', String(btn.dataset.tool === state.id));
    }

    detail.innerHTML = `
      <div class="detail-scroll">
        <div class="detail-head">
          <span class="card-icon"><svg class="ico" aria-hidden="true"><use href="#${tool.icon}"/></svg></span>
          <div>
            <h4>${esc(tool.title)}</h4>
            <p>${esc(tool.blurb)}</p>
          </div>
        </div>

        ${showFiles
          ? `<div class="files">${fileMarkup(tool)}</div>`
          : `<div class="dropzone">
               <span class="glyph" aria-hidden="true">⬇</span>
               <p>${esc(tool.prompt)}</p>
               <button class="btn btn-outline btn-sm" type="button" data-add>Choose Files…</button>
             </div>`}

        ${tool.options.map((o, i) => optionMarkup(tool, o, i)).join('')}
      </div>

      <div class="runbar">
        ${status}
        <button class="btn btn-default btn-sm" type="button" data-run ${running ? 'disabled' : ''}>
          ${esc(state.phase === 'done' ? 'Run again' : runLabel)}
        </button>
      </div>`;

    wire();
  }

  function wire() {
    const tool = current();

    $('[data-add]', detail)?.addEventListener('click', () => {
      state.phase = 'queued';
      state.done = 0;
      render();
    });

    $$('[data-choice]', detail).forEach((btn) => {
      btn.addEventListener('click', () => {
        tool.options[Number(btn.dataset.opt)].value = Number(btn.dataset.choice);
        render();
      });
    });

    $$('.slider', detail).forEach((slider) => {
      slider.addEventListener('input', () => {
        tool.options[Number(slider.dataset.opt)].value = Number(slider.value);
        slider.parentElement.querySelector('.val').textContent = `${slider.value}%`;
      });
    });

    $('[data-run]', detail)?.addEventListener('click', run);
  }

  function run() {
    clearTimers();
    const tool = current();
    const total = tool.files.length;

    // Nothing queued yet? Fill the queue first, the way dropping files would.
    if (state.phase === 'idle') {
      state.phase = 'queued';
      state.done = 0;
      render();
      timers.push(setTimeout(run, reduceMotion ? 0 : 260));
      return;
    }

    Object.assign(state, { phase: 'running', progress: 0, done: 0 });
    render();

    const step = reduceMotion ? 40 : 520;
    for (let i = 1; i <= total; i++) {
      timers.push(setTimeout(() => {
        state.done = i;
        state.progress = i / total;
        if (i === total) state.phase = 'done';
        render();
      }, step * i));
    }
  }

  render();

  // Drop-zone flourish: the border lights up as the demo scrolls into view once.
  if (!reduceMotion && 'IntersectionObserver' in window) {
    const once = new IntersectionObserver((entries, obs) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        obs.disconnect();
        const zone = $('.dropzone', detail);
        if (!zone) return;
        timers.push(setTimeout(() => zone.classList.add('hot'), 700));
        timers.push(setTimeout(() => zone.classList.remove('hot'), 1900));
      }
    }, { threshold: .4 });
    once.observe(demo);
  }
})();
