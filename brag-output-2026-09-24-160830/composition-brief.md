# HyperFrames Composition Brief — Toolbox

Make a 20.2-second, 1920×1080 landscape launch video using the accompanying brag plan. This is a polished desktop product film, not a generic SaaS promo. Keep all text readable, use local assets only, and do not add voiceover.

## Story and source material
- Show the current Toolbox tool-library screen in composition/assets/toolbox-home.png.
- Show the current Edit PDF workspace in composition/assets/toolbox-edit-selected.png. sample.pdf is a fictional UI placeholder added for this capture; no file is processed or shown.
- Tell the actual workflow: choose a local file, make a PDF edit, save a separate copy.
- Use the exact supported privacy copy: “Private by default. Files stay on this device.” Toolbox’s README states files are processed on-device and never uploaded.
- Avoid invented product counts, claims, or URLs.

## Art direction
Use the source app’s dark desktop palette: background #171717, surface #202020, border #3a3a3a, text #ededed, muted #999999, and action blue #60a5fa. Use system UI sans-serif fonts. Keep the composition spacious, precise, and native to a desktop utility. Frame the real UI screens; do not recreate their content as a website mockup.

## Beat plan
1. 0.00–3.65s: “Your files / stay yours.” Introduce the Toolbox mark and PDF, image, document tiles.
2. 3.30–8.20s: actual tool-library capture, paired with “PDFs. Images. Documents.”
3. 7.85–14.35s: actual Edit PDF capture; readable steps at 9.02s, 10.02s, and 11.02s: “Choose a file”, “Make an edit”, “Save a copy”.
4. 14.35–20.20s: privacy payoff, with the Toolbox wordmark landing at 17.02s.

The 0.35s scene overlaps are intentional, with transparent scene backgrounds and soft content fades. The root background remains constant.

## Audio
- Use only the local composition/assets/music/music-bed.mp3, composition/assets/sfx/ui-click.ogg, and composition/assets/sfx/reveal.ogg.
- The music is a 20.2-second local excerpt with fades already applied. Keep it restrained at data-volume 0.3.
- The underlying track’s bundled cue preset is 120.19 BPM. Keep the three step reveals on the supplied beat grid and the wordmark at 17.02s.
- Use sparse click accents at the step reveals and the soft reveal cue at the wordmark.
- composition/assets/audio-data.js contains pre-extracted per-frame band data. Let the actual Toolbox screen frame’s blue rim light respond subtly to low-frequency energy; do not pulse or scale text.
- Music attribution is in attribution.txt; retain it in the output.

## Build contract
- Build one standalone composition/index.html with a single paused GSAP timeline registered as window.__timelines[toolbox-brag].
- Use the local GSAP file at composition/assets/vendor/gsap.min.js; no remote fonts, scripts, media, network calls, or voice services.
- Preserve all assets and text exactly as described. Do not add claims or show OS chrome.
- Run npx hyperframes check before preview. Do not render until the user has reviewed the preview and approved rendering.
