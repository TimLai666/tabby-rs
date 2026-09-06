# Tabby RS · Brand Spec

> Collection date: 2026-09-06
> Asset source: Existing repository artwork and Tauri bundle configuration
> Asset completeness: Partial, scoped to the Rust desktop icon

## Core assets

### Logo

- Existing Tabby source mark: `app/assets/logo.svg`
- Rust variant source: `src-tauri/icons/icon.png.b64`
- Generated Rust PNG: `src-tauri/icons/icon.png`
- Generated Rust ICO: `src-tauri/icons/icon.ico`
- Rust macOS icon: `src-tauri/icons/icon.icns`
- Legacy configured macOS icon: `build/mac/icon.icns`
- Usage: Tauri desktop application bundle icon
- Do not: stretch, redraw, or apply a generic glow

## Supporting assets

### Palette

- Background: `#000000` — requested opaque black field
- Arrow: `#E84A8A` — requested pink central arrow face
- Main gold side plane: `#E6C34A` — replaces the original cyan side plane
- Dark gold side planes: `#B58D2B` and `#A98226` — preserve the original blue depth hierarchy
- RS badge: `#18202C` — retained from the existing mark

### Typeface

- RS badge lettering is embedded in the existing raster artwork.
- No additional typeface is introduced.

## Signature detail

The signature is the contrast between the black field, pink central arrow, gold extrusion, and preserved RS badge.

## Boundaries

- This is a Tabby RS variant, not a replacement for the official Rust logo.
- Keep the Rust-specific treatment inside `src-tauri/icons/` and the configured legacy macOS mirror at `build/mac/icon.icns`.
- Keep `app/assets/logo.svg` and `build/icons/` unchanged unless a separate cross-platform branding request is made.

## Tone

- high contrast
- technical
- restrained
- dimensional
