(() => {
  'use strict';

  const $ = (sel, root = document) => root.querySelector(sel);
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
    $('meta[name="theme-color"]')?.setAttribute(
      'content',
      theme === 'dark' ? '#0a0a0a' : '#ffffff'
    );
    for (const image of $$('[data-theme-src-dark][data-theme-src-light]')) {
      const source = image.dataset[`themeSrc${theme[0].toUpperCase()}${theme.slice(1)}`];
      if (source && image.getAttribute('src') !== source) image.setAttribute('src', source);
    }
  };

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

  matchMedia('(prefers-color-scheme: light)').addEventListener('change', (e) => {
    if (!localStorage.getItem('toolbox-theme')) setTheme(e.matches ? 'light' : 'dark');
  });

  /* ───────────────────────── downloads ───────────────────────── */

  // These are versioned asset URLs, so every download starts the installer directly.
  // Keep them in one place when cutting a new release.
  const downloads = {
    macos: {
      href: 'https://github.com/lazzyms/toolbox/releases/download/tauri-v0.2.0/Toolbox-0.2.0-macos.dmg',
      label: 'Download for macOS',
      detail: 'Apple silicon · DMG',
    },
    'windows-x64': {
      href: 'https://github.com/lazzyms/toolbox/releases/download/tauri-v0.2.0/Toolbox-windows-x86_64-setup.exe',
      label: 'Download for Windows x64',
      detail: 'Intel or AMD · EXE installer',
    },
    'windows-arm64': {
      href: 'https://github.com/lazzyms/toolbox/releases/download/tauri-v0.2.0/Toolbox-windows-arm64-setup.exe',
      label: 'Download for Windows ARM64',
      detail: 'ARM Windows · EXE installer',
    },
  };

  const browserPlatform = [
    navigator.userAgentData?.platform,
    navigator.platform,
    navigator.userAgent,
  ].filter(Boolean).join(' ').toLowerCase();
  const detectedPlatform = browserPlatform.includes('win')
    ? (/(arm64|aarch64)/.test(browserPlatform) ? 'windows-arm64' : 'windows-x64')
    : browserPlatform.includes('mac') ? 'macos' : null;

  if (detectedPlatform) {
    const target = downloads[detectedPlatform];
    for (const link of $$('[data-download-auto]')) {
      link.href = target.href;
      link.setAttribute('aria-label', target.label);
      $('[data-download-label]', link)?.replaceChildren(target.label);
    }
    const recommended = $(`[data-download-platform="${detectedPlatform}"]`);
    recommended?.setAttribute('data-recommended', 'true');
    recommended?.setAttribute('aria-label', `${target.label} (${target.detail}, recommended)`);
    $('#dl-note')?.replaceChildren(`Recommended for this device: ${target.detail}. Other installers are listed below.`);
  }

  /* ───────────────────────── nav chrome ───────────────────────── */

  const nav = $('.nav');
  const onScroll = () => nav?.classList.toggle('stuck', window.scrollY > 8);
  onScroll();
  addEventListener('scroll', onScroll, { passive: true });

  const links = $$('.nav-links a[href^="#"]');
  const sections = links.map((a) => $(a.getAttribute('href'))).filter(Boolean);
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
    sections.forEach((section) => spy.observe(section));
  }

  /* ───────────────────────── scroll reveal ───────────────────────── */

  const revealable = $$('.section > .wrap > *, .closer .wrap > *, .screenshots > *');
  if (!reduceMotion && 'IntersectionObserver' in window) {
    const show = (el) => {
      el.classList.add('in');
      el.style.transitionDelay = '';
    };
    const io = new IntersectionObserver(
      (entries, observer) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          show(entry.target);
          observer.unobserve(entry.target);
        }
      },
      { rootMargin: '0px 0px -8% 0px' }
    );
    revealable.forEach((el, index) => {
      el.classList.add('reveal');
      el.style.transitionDelay = `${Math.min(index % 6, 5) * 45}ms`;
      io.observe(el);
    });
    const failsafe = () => setTimeout(() => {
      revealable.forEach((el) => { if (!el.classList.contains('in')) show(el); });
      io.disconnect();
    }, 1500);
    if (document.readyState === 'complete') failsafe();
    else addEventListener('load', failsafe, { once: true });
  }
})();
