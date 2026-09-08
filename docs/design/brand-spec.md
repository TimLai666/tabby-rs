# Tabby RS · Brand Spec

> Collection date: 2026-09-06
> Asset source: Existing repository artwork and Tauri bundle configuration
> Asset completeness: Shared across the application UI, tray, and desktop bundles

## Core assets

### Logo

- Canonical application source mark: `app/assets/logo.svg`
- Legacy Electron source mirror: `build/icons/icon.svg`
- Rust variant source: `src-tauri/icons/icon.png.b64`
- Generated Rust PNG: `src-tauri/icons/icon.png`
- Generated Rust ICO: `src-tauri/icons/icon.ico`
- Rust macOS icon: `src-tauri/icons/icon.icns`
- Legacy configured macOS icon: `build/mac/icon.icns`
- Usage: application pages, startup screen, tray, and desktop application bundles
- Do not: add an RS badge, stretch to the canvas edge, or apply a generic glow

## Supporting assets

### Palette

- Desktop icon background: `#000000` — required black application icon field
- UI/tray background: transparent — preserves breathing room around the mark
- Arrow: `#E84A8A` — requested pink central arrow face
- Main gold side plane: `#E6C34A` — replaces the original cyan side plane
- Dark gold side planes: `#B58D2B` and `#A98226` — preserve the original blue depth hierarchy
- UI chrome: `#18202C` — used around the mark, not embedded in it

### Typeface

- No lettering is embedded in the mark.
- UI titles use gold `#E6C34A`, with pink `#E84A8A` for accents.

## Signature detail

The signature is the contrast between the restrained pink arrow face, gold extrusion, black desktop icon field, and transparent UI/tray canvas.

## Boundaries

- This is a Tabby RS variant, not a replacement for the official Rust logo.
- Keep the transparent UI/tray treatment in `app/assets/logo.svg`; derive desktop assets in `build/icons/`, `src-tauri/icons/`, and `build/mac/` with a black field behind the same arrow.
- Keep macOS tray template images monochrome while preserving the canonical arrow silhouette.

## Tone

- high contrast
- technical
- restrained
- dimensional
