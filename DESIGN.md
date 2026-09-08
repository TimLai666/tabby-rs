---
version: alpha
name: Tabby RS
description: A Tabby RS desktop icon with a black field, a restrained pink-and-gold arrow, and no embedded badge.
colors:
  primary: "#000000"
  secondary: "#E84A8A"
  tertiary: "#E6C34A"
  neutral: "#18202C"
assets:
  rustIconSource: "src-tauri/icons/icon.png.b64"
  rustIconPng: "src-tauri/icons/icon.png"
  rustIconIco: "src-tauri/icons/icon.ico"
  rustIconIcns: "src-tauri/icons/icon.icns"
  rustLegacyMacIcon: "build/mac/icon.icns"
---

## Overview

Tabby RS uses the original three-dimensional arrow construction as a shared product mark. The application logo and tray keep a transparent canvas, while the desktop icon uses a black field behind the restrained pink-and-gold arrow. No embedded RS badge is used.

## Colors

- **Primary (#000000):** Required black field for the desktop application icon.
- **Secondary (#E84A8A):** Pink central arrow face requested for the Rust variant.
- **Tertiary (#E6C34A):** Main gold side plane replacing the original blue extrusion.
- **Neutral (#18202C):** Dark application chrome used around the mark, not embedded in it.

## Typography

The icon has no display typography or embedded lettering. The application UI uses the bundled Source Sans Pro family with the operating system's CJK fallback, with gold primary titles and pink accents. Terminal content and code previews use the bundled Source Code Pro family, with a monospace fallback.

## Layout

The mark remains centered in a square canvas and uses 80% of the original arrow geometry, leaving a clear margin without losing its dimensional silhouette. Desktop icon canvases use black; application pages and tray images remain transparent. The Tauri window opens at 1100×720 logical pixels with a 640×480 minimum, preserving a compact terminal-first workspace. On macOS, the title bar is an overlay so the dark tab strip forms one continuous top chrome instead of a separate white native band.

## Elevation & Depth

The side planes use a gold family with darker companion shades to preserve the original depth. Existing low-opacity depth overlays remain, but no glow or extra decorative layer is added.

## Shapes

The existing chamfered arrow shape is preserved without stretching or redrawing, while the embedded RS badge is intentionally removed.

## Components

The application pages and tray use the transparent canonical mark. Desktop PNG, ICO, and both configured macOS ICNS assets use the same pink-and-gold arrow over black. macOS template tray images use the same silhouette in monochrome for native menu-bar rendering.

## Do's and Don'ts

- Do use the Rust icon assets together so all desktop platforms share the same treatment.
- Don't reintroduce an RS badge or scale the arrow to the canvas edge.
- Don't treat this repo-specific mark as the official Rust Foundation logo.
