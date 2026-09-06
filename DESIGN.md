---
version: alpha
name: Tabby RS
description: A dark, high-contrast Rust desktop icon built from the existing Tabby mark.
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

Tabby RS is the Rust desktop variant of the existing Tabby mark. The icon keeps the original three-dimensional arrow construction and RS badge, while using an opaque black field, a pink central arrow, and gold side planes for a clearer Rust-build identity.

## Colors

- **Primary (#000000):** Opaque background for the Rust desktop icon.
- **Secondary (#E84A8A):** Pink central arrow face requested for the Rust variant.
- **Tertiary (#E6C34A):** Main gold side plane replacing the original blue extrusion.
- **Neutral (#18202C):** Existing RS badge surface.

## Typography

The icon has no display typography. The existing RS badge lettering is preserved as part of the source artwork.

## Layout

The mark remains centered in a square canvas. The original proportions, arrow geometry, and RS badge placement are unchanged.

## Elevation & Depth

The side planes use a gold family with darker companion shades to preserve the original depth. No glow, shadow, or extra decorative layer is added.

## Shapes

The existing chamfered arrow and RS badge shapes are preserved without stretching or redrawing.

## Components

The Rust bundle consumes the PNG, ICO, and both configured macOS ICNS assets. They share the same black, pink, and gold treatment.

## Do's and Don'ts

- Do use the Rust icon assets together so all desktop platforms share the same treatment.
- Don't recolor or replace the Electron logo when making a Rust-only identity change.
- Don't treat this repo-specific mark as the official Rust Foundation logo.
