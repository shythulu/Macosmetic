#!/usr/bin/env node
//
// Generates the Darkberry COSMIC themes in res/themes/cosmic from a Darkberry checkout:
// every flavour (Wisp, Fen, Mire, Blackwater) in every tint (Darkberry, Lingonberry,
// Cloudberry, Crowberry, Blueberry), twenty files in all.
//
// Usage: scripts/darkberry-themes.mjs [path to github.com/shythulu/DarkBerry]
//        (default ../DarkBerry beside this repository)
//
// Colours are resolved through Darkberry's own roles (lib/resolve.mjs), so a theme here
// agrees with the Darkberry ports for terminals and editors. Each file is a serialized
// cosmic_theme::ThemeBuilder, the format src/theme_catalog.rs reads:
//
//   palette        Darkberry's neutrals as libcosmic's neutral ramp (background end first),
//                  and its accents in libcosmic's accent and extended slots
//   bg_color       ui.background; libcosmic derives the container surfaces from it
//   neutral_tint   ui.background, so controls take its hue instead of plain grey
//   text_tint      ui.text
//   accent         ui.accent
//   success, warning, destructive
//                  ui.success, ui.warning, ui.error

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const darkberry = path.resolve(process.argv[2] ?? path.join(repoRoot, "..", "DarkBerry"));
const outDir = path.join(repoRoot, "res", "themes", "cosmic");

const readJson = (rel) => JSON.parse(fs.readFileSync(path.join(darkberry, rel), "utf8"));
const { indexRoles, flavourContext } = await import(
  pathToFileURL(path.join(darkberry, "lib", "resolve.mjs")).href
);

const ROLES = readJson("src/roles.json");
const roleIndex = indexRoles(ROLES);
const tints = readJson("src/tints.json");
delete tints.$comment;

// libcosmic's neutral ramp runs from the background end (neutral_0) to the text end
// (neutral_10) in both polarities. Darkberry has twelve neutrals; mantle, between base and
// crust, is the one left out.
const NEUTRALS = {
  dark: ["crust", "base", "surface0", "surface1", "surface2", "overlay0", "overlay1", "overlay2", "subtext0", "subtext1", "text"],
  light: ["base", "crust", "surface0", "surface1", "surface2", "overlay0", "overlay1", "overlay2", "subtext0", "subtext1", "text"],
};

// libcosmic's named colours, which the accent picker and a few widgets draw from.
const PALETTE_COLOURS = {
  bright_red: "cranberry",
  bright_green: "gooseberry",
  bright_orange: "apricot",
  gray_1: "base",
  gray_2: "surface0",
  accent_blue: "blueberry",
  accent_indigo: "lavender",
  accent_purple: "plum",
  accent_pink: "berry",
  accent_red: "cranberry",
  accent_orange: "apricot",
  accent_yellow: "honey",
  accent_green: "gooseberry",
  accent_warm_grey: "overlay1",
  ext_warm_grey: "overlay2",
  ext_orange: "apricot",
  ext_yellow: "honey",
  ext_blue: "frost",
  ext_purple: "plum",
  ext_pink: "petal",
  ext_indigo: "bilberry",
};

const channel = (hex, i) => +(parseInt(hex.slice(1 + 2 * i, 3 + 2 * i), 16) / 255).toFixed(8);
const srgb = (hex) => `(red:${channel(hex, 0)},green:${channel(hex, 1)},blue:${channel(hex, 2)})`;
const srgba = (hex) => `(red:${channel(hex, 0)},green:${channel(hex, 1)},blue:${channel(hex, 2)},alpha:1.0)`;

// libcosmic's defaults, as the bundled cosmic-ext-themes files carry them.
const SPACING =
  "spacing:(space_none:0,space_xxxs:4,space_xxs:8,space_xs:12,space_s:16,space_m:24,space_l:32,space_xl:48,space_xxl:64,space_xxxl:128)";
const CORNER_RADII =
  "corner_radii:(radius_0:(0.0,0.0,0.0,0.0),radius_xs:(4.0,4.0,4.0,4.0),radius_s:(8.0,8.0,8.0,8.0),radius_m:(16.0,16.0,16.0,16.0),radius_l:(32.0,32.0,32.0,32.0),radius_xl:(160.0,160.0,160.0,160.0))";

function theme(tintId, flavourId, flavour) {
  const ctx = flavourContext(flavourId, flavour, ROLES, roleIndex);
  const colour = (expr) => ctx.resolve(expr)[0];
  const polarity = flavour.dark ? "dark" : "light";

  const palette = [`name:"${tintId}-${flavourId}"`];
  for (const [field, name] of Object.entries(PALETTE_COLOURS)) palette.push(`${field}:${srgba(colour(name))}`);
  NEUTRALS[polarity].forEach((name, i) => palette.push(`neutral_${i}:${srgba(colour(name))}`));

  const background = colour("ui.background");
  return (
    `(palette:${flavour.dark ? "Dark" : "Light"}((${palette.join(",")})),` +
    `${SPACING},${CORNER_RADII},` +
    `neutral_tint:Some(${srgb(background)}),` +
    `bg_color:Some(${srgba(background)}),` +
    `primary_container_bg:None,secondary_container_bg:None,` +
    `text_tint:Some(${srgb(colour("ui.text"))}),` +
    `accent:Some(${srgb(colour("ui.accent"))}),` +
    `success:Some(${srgb(colour("ui.success"))}),` +
    `warning:Some(${srgb(colour("ui.warning"))}),` +
    `destructive:Some(${srgb(colour("ui.error"))}),` +
    `gaps:(0,8),active_hint:3,window_hint:None)\n`
  );
}

let written = 0;
for (const [tintId, tint] of Object.entries(tints)) {
  const pal = tintId === "darkberry" ? readJson("src/palette.json") : readJson(`src/variants/${tintId}.json`);
  for (const [flavourId, flavour] of Object.entries(pal.flavours)) {
    const file = path.join(outDir, `${tint.name} ${flavour.name}.ron`);
    fs.writeFileSync(file, theme(tintId, flavourId, flavour));
    written += 1;
  }
}
console.log(`wrote ${written} Darkberry themes to ${path.relative(repoRoot, outDir)}`);
