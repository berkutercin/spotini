// Generates spotini's logo files and social images from code: `npm run brand`.
// Text is converted to outlines with the bundled JetBrains Mono, so the SVGs
// don't depend on any installed font.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import opentype from "opentype.js";
import { Resvg } from "@resvg/resvg-js";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");
const fontsDir = path.join(root, "node_modules/@fontsource/jetbrains-mono/files");

export const COLORS = {
  charcoal: "#161616",
  surface: "#1f1e1c",
  line: "#2c2a27",
  ink: "#e8e2d6",
  muted: "#a09a8f",
  amber: "#f5a524",
  paper: "#f4efe6",
  paperInk: "#1b1a18",
  // Amber darkened for light backgrounds (at least 3:1 against paper).
  amberOnPaper: "#c27a0e",
};

const loadFont = (weight) => {
  const file = fs.readFileSync(path.join(fontsDir, `jetbrains-mono-latin-${weight}-normal.woff`));
  return opentype.parse(file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength));
};
const bold = loadFont(700);
const regular = loadFont(400);

// ---------- The mark (direction A: prompt + bars) ----------

/** Shapes in a 512x512 box. `accent` colours the chevron and first two bars, `ink` the last bar. */
const markShapes = ({ accent, ink }) => `
  <path d="M104 172 L188 256 L104 340" fill="none" stroke="${accent}" stroke-width="50" stroke-linecap="round" stroke-linejoin="round"/>
  <rect x="236" y="232" width="50" height="108" rx="25" fill="${accent}"/>
  <rect x="310" y="152" width="50" height="188" rx="25" fill="${accent}"/>
  <rect x="384" y="204" width="50" height="136" rx="25" fill="${ink}"/>`;

/** Tight bounds of the mark inside the 512 box (chevron caps included). */
const MARK_BOX = { x: 79, y: 147, w: 355, h: 218 };

const svgDoc = (width, height, body, viewBox = `0 0 ${width} ${height}`) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="${viewBox}">${body}</svg>\n`;

const iconSvg = (size = 512) =>
  svgDoc(
    size,
    size,
    `<rect x="16" y="16" width="480" height="480" rx="112" fill="${COLORS.charcoal}"/>
     <rect x="16" y="16" width="480" height="480" rx="112" fill="none" stroke="${COLORS.line}" stroke-width="8"/>
     ${markShapes({ accent: COLORS.amber, ink: COLORS.ink })}`,
    "0 0 512 512",
  );

/** The mark without its tile, scaled into a group at (x, y) with the given height. */
const markGroup = (colors, x, y, height) => {
  const s = height / MARK_BOX.h;
  return `<g transform="translate(${x} ${y}) scale(${s}) translate(${-MARK_BOX.x} ${-MARK_BOX.y})">${markShapes(colors)}</g>`;
};

// ---------- Text ----------

const TRACKING = -0.04; // em
// JetBrains Mono's contextual ligatures garble some runs when outlined; plain glyphs only.
const TEXT_OPTIONS = { letterSpacing: TRACKING, features: { liga: false, rlig: false, calt: false } };

/** SVG path data from opentype commands (opentype's own toPathData emits NaN for some glyph positions). */
function pathData(path) {
  const n = (v) => v.toFixed(2);
  return path.commands
    .map((c) => {
      switch (c.type) {
        case "M":
        case "L":
          return `${c.type}${n(c.x)} ${n(c.y)}`;
        case "Q":
          return `Q${n(c.x1)} ${n(c.y1)} ${n(c.x)} ${n(c.y)}`;
        case "C":
          return `C${n(c.x1)} ${n(c.y1)} ${n(c.x2)} ${n(c.y2)} ${n(c.x)} ${n(c.y)}`;
        default:
          return "Z";
      }
    })
    .join("");
}

/** Outlined text as a path; returns its data and advance width. */
function textPath(font, text, x, baseline, size, fill) {
  const p = font.getPath(text, x, baseline, size, TEXT_OPTIONS);
  const advance = font.getAdvanceWidth(text, size, TEXT_OPTIONS);
  return { svg: `<path d="${pathData(p)}" fill="${fill}"/>`, advance };
}

/** "spotini" + block cursor: bold lowercase. `size` is the font size; returns svg + width. */
function wordmark(x, baseline, size, { ink, cursor }) {
  const word = textPath(bold, "spotini", x, baseline, size, ink);
  const top = baseline - size * 0.74;
  const cursorX = x + word.advance + size * 0.06;
  const cursorW = size * 0.3;
  // The dots of the i's rise above the cursor; boxes start at whichever is higher.
  const inkTop = Math.min(top, bold.getPath("spotini", x, baseline, size, TEXT_OPTIONS).getBoundingBox().y1);
  const svg = `${word.svg}<rect x="${cursorX.toFixed(2)}" y="${top.toFixed(2)}" width="${cursorW.toFixed(2)}" height="${(size * 0.82).toFixed(2)}" fill="${cursor}"/>`;
  return { svg, width: cursorX + cursorW - x, top, inkTop, bottom: top + size * 0.82 };
}

// ---------- Variants ----------

const THEMES = {
  dark: { bg: COLORS.charcoal, accent: COLORS.amber, ink: COLORS.ink, word: COLORS.ink, cursor: COLORS.amber },
  light: { bg: COLORS.paper, accent: COLORS.amberOnPaper, ink: COLORS.paperInk, word: COLORS.paperInk, cursor: COLORS.amberOnPaper },
  "mono-dark": { bg: COLORS.charcoal, accent: COLORS.ink, ink: COLORS.ink, word: COLORS.ink, cursor: COLORS.ink },
  "mono-light": { bg: COLORS.paper, accent: COLORS.paperInk, ink: COLORS.paperInk, word: COLORS.paperInk, cursor: COLORS.paperInk },
};

// Logo files have transparent backgrounds; the theme name says which background they're for.
function markSvg(theme) {
  const t = THEMES[theme];
  return svgDoc(MARK_BOX.w, MARK_BOX.h, markGroup(t, 0, 0, MARK_BOX.h));
}

function wordmarkSvg(theme) {
  const t = THEMES[theme];
  const size = 100;
  const w = wordmark(0, 0, size, { ink: t.word, cursor: t.cursor });
  // Re-run positioned so the box starts at 0,0.
  const placed = wordmark(0, -w.inkTop, size, { ink: t.word, cursor: t.cursor });
  return svgDoc(Math.ceil(w.width), Math.ceil(w.bottom - w.inkTop + size * 0.24), placed.svg);
}

/** Mark + wordmark side by side; the mark is as tall as the cursor block. */
function lockupSvg(theme) {
  const t = THEMES[theme];
  const size = 100;
  const probe = wordmark(0, 0, size, t);
  const markH = probe.bottom - probe.top;
  const markW = (MARK_BOX.w / MARK_BOX.h) * markH;
  const gap = size * 0.36;
  const baseline = -probe.inkTop;
  const markY = probe.top - probe.inkTop;
  const word = wordmark(markW + gap, baseline, size, { ink: t.word, cursor: t.cursor });
  const width = Math.ceil(markW + gap + word.width);
  const height = Math.ceil(markY + markH + size * 0.24); // room for descender of "p"
  return svgDoc(width, height, markGroup(t, 0, markY, markH) + word.svg);
}

// ---------- Social image and favicon ----------

const TAGLINE = "A tiny, keyboard-first player for Spotify.";

function ogImageSvg() {
  const W = 1280;
  const H = 640;
  const size = 120;
  const probe = wordmark(0, 0, size, THEMES.dark);
  const markH = probe.bottom - probe.top;
  const markW = (MARK_BOX.w / MARK_BOX.h) * markH;
  const gap = size * 0.36;
  const lockW = markW + gap + probe.width;
  const x0 = (W - lockW) / 2;
  const top = 214;
  const word = wordmark(x0 + markW + gap, top - probe.top, size, { ink: COLORS.ink, cursor: COLORS.amber });

  const tagSize = 34;
  const tagW = regular.getAdvanceWidth(TAGLINE, tagSize, TEXT_OPTIONS);
  const tag = textPath(regular, TAGLINE, (W - tagW) / 2, top + markH + 92, tagSize, COLORS.muted);

  // Separators are drawn as dots: the font's "·" outlines to NaN coordinates.
  const footParts = ["github.com/berkutercin/spotini", "unofficial", "needs Spotify Premium"];
  const footSize = 22;
  const sepW = footSize * 1.4;
  const partWidths = footParts.map((t) => regular.getAdvanceWidth(t, footSize, TEXT_OPTIONS));
  let fx = (W - partWidths.reduce((a, b) => a + b, 0) - sepW * (footParts.length - 1)) / 2;
  const footY = H - 56;
  const footer = footParts
    .map((t, i) => {
      const part = textPath(regular, t, fx, footY, footSize, "#5f5b54").svg;
      fx += partWidths[i];
      const dot = i < footParts.length - 1 ? `<circle cx="${(fx + sepW / 2).toFixed(1)}" cy="${footY - footSize * 0.3}" r="2.5" fill="#5f5b54"/>` : "";
      fx += sepW;
      return part + dot;
    })
    .join("");

  return svgDoc(
    W,
    H,
    `<defs>
       <radialGradient id="glow" cx="50%" cy="38%" r="60%">
         <stop offset="0" stop-color="${COLORS.amber}" stop-opacity="0.16"/>
         <stop offset="1" stop-color="${COLORS.amber}" stop-opacity="0"/>
       </radialGradient>
     </defs>
     <rect width="${W}" height="${H}" fill="${COLORS.charcoal}"/>
     <rect width="${W}" height="${H}" fill="url(#glow)"/>
     <rect x="24" y="24" width="${W - 48}" height="${H - 48}" rx="18" fill="none" stroke="${COLORS.line}" stroke-width="2"/>
     ${markGroup(THEMES.dark, x0, top, markH)}
     ${word.svg}
     ${tag.svg}
     ${footer}`,
  );
}

/** Favicon: the tile, with slightly thicker strokes to hold up at 16 px. */
const faviconSvg = () =>
  svgDoc(
    64,
    64,
    `<rect x="0" y="0" width="512" height="512" rx="112" fill="${COLORS.charcoal}"/>
     <g transform="translate(256 256) scale(1.08) translate(-256 -256)">${markShapes({ accent: COLORS.amber, ink: COLORS.ink })}</g>`,
    "0 0 512 512",
  );

// ---------- Output ----------

const write = (rel, content) => {
  const file = path.join(here, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  return file;
};

const png = (svg, width) => new Resvg(svg, { fitTo: { mode: "width", value: width } }).render().asPng();

/** Packs PNG images into an .ico file. */
function ico(pngs) {
  const header = Buffer.alloc(6 + 16 * pngs.length);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(pngs.length, 4);
  let offset = header.length;
  pngs.forEach((data, i) => {
    const size = data.readUInt32BE(16); // the PNG's width; height is the same
    const entry = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, entry);
    header.writeUInt8(size >= 256 ? 0 : size, entry + 1);
    header.writeUInt16LE(1, entry + 4); // colour planes
    header.writeUInt16LE(32, entry + 6); // bits per pixel
    header.writeUInt32LE(data.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += data.length;
  });
  return Buffer.concat([header, ...pngs]);
}

const outputs = [];
const svgAndPng = (name, svg, widths) => {
  outputs.push(write(`logo/${name}.svg`, svg));
  for (const w of widths) outputs.push(write(`logo/png/${name}${widths.length > 1 ? `-${w}` : ""}.png`, png(svg, w)));
};

svgAndPng("icon", iconSvg(), [16, 32, 64, 128, 256, 512, 1024]);
for (const theme of Object.keys(THEMES)) {
  svgAndPng(`mark-${theme}`, markSvg(theme), [512]);
  svgAndPng(`wordmark-${theme}`, wordmarkSvg(theme), [1200]);
  svgAndPng(`lockup-${theme}`, lockupSvg(theme), [1600]);
}

const og = ogImageSvg();
outputs.push(write("social/og-image.svg", og));
outputs.push(write("social/og-image.png", png(og, 1280)));

const fav = faviconSvg();
outputs.push(write("favicon/favicon.svg", fav));
for (const w of [16, 32, 180]) outputs.push(write(`favicon/favicon-${w}.png`, png(fav, w)));

// The app's icon (window, taskbar, tray and installer): an .ico holding PNGs at the sizes Windows asks for.
outputs.push(write("../app/icon.ico", ico([16, 20, 24, 32, 40, 48, 64, 256].map((w) => png(iconSvg(), w)))));

console.log(`wrote ${outputs.length} files`);
