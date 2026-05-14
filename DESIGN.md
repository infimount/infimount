---
name: Infimount
description: Minimal local-first storage browser with native desktop utility aesthetics.
colors:
  app-background: "#F7F7F7"
  app-foreground: "#3D3D3D"
  surface: "#FFFFFF"
  surface-muted: "#F0F0F0"
  sidebar-light: "#EBEBEB"
  sidebar-dark: "#262626"
  border-soft: "#EAEAEA"
  border-strong: "#DADADA"
  accent-orange: "#E95420"
  accent-orange-soft: "#F6D8C8"
  success-muted: "#476A52"
  warning-muted: "#9A6A14"
  destructive-muted: "#6B3A3A"
typography:
  display:
    fontFamily: "Ubuntu, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "2rem"
    fontWeight: 700
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  title:
    fontFamily: "Ubuntu, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "1rem"
    fontWeight: 550
    lineHeight: 1.25
    letterSpacing: "0"
  body:
    fontFamily: "Ubuntu, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "0"
  label:
    fontFamily: "Ubuntu, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.35
    letterSpacing: "0"
  mono:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "0"
rounded:
  sm: "6px"
  md: "8px"
  lg: "12px"
  dialog: "16px"
  window: "12px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
components:
  button-primary:
    backgroundColor: "{colors.app-foreground}"
    textColor: "{colors.surface}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.app-foreground}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.app-foreground}"
    rounded: "{rounded.md}"
    padding: "8px 12px"
  sidebar-item-selected:
    backgroundColor: "{colors.border-strong}"
    textColor: "{colors.app-foreground}"
    rounded: "{rounded.md}"
    padding: "10px 12px"
---

# Design System: Infimount

## 1. Overview

**Creative North Star: "Ubuntu File Manager, widened for cloud storage."**

Infimount should feel like a native desktop utility that quietly understands many storage systems. The interface should not compete with the files. It should give users clear navigation, reliable state, and confidence that local credentials and MCP exposure are under their control.

The default product register is restrained and task-focused. Marketing and GitHub Pages surfaces may be more expressive, but they should still borrow the same native desktop cues: neutral surfaces, dark sidebar contrast, product screenshots, small orange accents, and direct copy.

**Key Characteristics:**

- Minimal layout with strong structure and low visual noise.
- Ubuntu-inspired neutral greys, white content surfaces, and subtle orange accents.
- Familiar file-explorer patterns over custom novelty.
- Clear state for selected, focused, loading, read-only, exposed, running, stopped, warning, and error.
- Native-feeling dialogs with consistent buttons, inputs, focus, and confirmation patterns.

## 2. Colors

The palette is mostly neutral grey and white, with orange used sparingly for Ubuntu-flavored emphasis and product identity.

### Primary

- **Ash Foreground** (`#3D3D3D`): Primary text, primary buttons, and active controls.
- **Ubuntu Orange** (`#E95420`): Rare accent for brand moments, important highlights, or controlled warning emphasis. Do not use it as a broad background theme.

### Secondary

- **Sidebar Charcoal** (`#262626`): Dark-mode sidebar and high-contrast shell areas.
- **Soft Orange Tint** (`#F6D8C8`): Low-emphasis accent background for small badges or gentle highlights.

### Neutral

- **App Background** (`#F7F7F7`): Main app background.
- **Surface White** (`#FFFFFF`): Dialogs, inputs, file surfaces, and high-priority panels.
- **Muted Surface** (`#F0F0F0`): Sidebar, secondary panels, hover states, and low-emphasis toolbars.
- **Soft Border** (`#EAEAEA`): Default borders and dividers.
- **Strong Border** (`#DADADA`): Selected, focused, or structurally important boundaries.

### Named Rules

**The Orange Rarity Rule.** Orange is used as a small accent, not as a theme. If more than one major surface on a screen is orange, the design is too loud.

**The Grey Ladder Rule.** Depth should usually come from grey tonal steps and borders, not from heavy shadows or saturated backgrounds.

## 3. Typography

**Display Font:** Ubuntu with system fallbacks  
**Body Font:** Ubuntu with system fallbacks  
**Label/Mono Font:** Ubuntu for UI labels, system monospace for paths, JSON, snippets, and file contents

**Character:** Typography should feel native and practical. Use a compact product scale. Avoid expressive display type inside the app shell.

### Hierarchy

- **Display** (700, 2rem, 1.2): Rare. Use for landing page hero or major marketing headings, not compact tool panels.
- **Headline** (550 to 700, 1.25rem to 1.5rem, 1.2): Section headers and major empty states.
- **Title** (550, 1rem, 1.25): Dialog titles, panel headers, and major row labels.
- **Body** (400, 0.875rem, 1.5): Normal UI text, descriptions, and file metadata.
- **Label** (400 to 500, 0.75rem, 1.35): Form labels, status labels, compact helper text.
- **Mono** (400, 0.75rem, 1.6): Paths, MCP snippets, JSON, backend config, and text file previews.

### Named Rules

**The Utility Type Rule.** If text lives inside a sidebar, toolbar, dialog, file list, or settings panel, keep it compact and readable. No hero-sized type in product surfaces.

## 4. Elevation

Infimount uses tonal layering first and shadows second. Most surfaces are flat at rest. Borders, background steps, and selected states should do the structural work. Shadows are reserved for dialogs, popovers, and release or landing-page hero frames.

### Shadow Vocabulary

- **Dialog Shadow** (`0 22px 70px rgba(49, 43, 34, 0.14)`): Use for modal dialogs and landing-page hero frames.
- **Subtle Panel Shadow** (`0 10px 24px rgba(0, 0, 0, 0.04)`): Use only where a panel needs separation from a same-color background.

### Named Rules

**The Flat-By-Default Rule.** Surfaces should not float unless they are temporary UI, such as dialogs, popovers, menus, or overlays.

## 5. Components

### Buttons

- **Shape:** 8px radius for product UI. Pill buttons are acceptable on the landing page only.
- **Primary:** Dark grey background with white text. Use for the one action that commits or starts a flow.
- **Secondary:** White or background-colored button with a soft border.
- **Danger:** Muted destructive tone with explicit destructive copy. Avoid pure red unless the action is truly destructive.
- **Hover / Focus:** Hover can shift tone subtly. Focus must be clearly visible with a ring or strong border. Do not remove focus rings without replacement.

### Cards / Containers

- **Corner Style:** 8px to 12px for product panels, 16px for dialogs.
- **Background:** White or muted grey. Avoid nested card stacks unless each layer has a distinct job.
- **Border:** Soft grey border by default. Strong border only for focus, selected state, or important structure.
- **Internal Padding:** 12px to 16px for compact product panels, 20px to 24px for marketing sections.

### Inputs / Fields

- **Style:** White background, grey border, 8px radius, compact height.
- **Focus:** Visible ring or stronger border. Keyboard focus must be obvious.
- **Error:** Rose or muted destructive border with plain-language message below or near the field.
- **Disabled:** Lower contrast, but still legible.

### Switches

- **Style:** Grey unchecked track with a visible border and thumb. Checked state may use dark grey or subtle accent.
- **Labeling:** Pair every switch with a plain-language label and a short effect description.
- **Behavior:** If a switch requires restart or only changes persisted settings, say so next to the control.

### Dialogs

- **Shape:** 16px radius with clipped overlay corners matching the app window.
- **Content:** Keep headings small. Use sections only when they reduce scanning effort.
- **Actions:** Primary action on the right. Destructive actions use confirmation dialogs, not browser-native `confirm()`.
- **Overlay:** Dark translucent overlay that respects app window radius.

### Sidebar

- **Style:** Native file-manager rail. Muted grey in light mode, charcoal in dark mode.
- **Selection:** Full-width clickable row with visible selected and keyboard-focused states.
- **Icons:** Small, familiar storage icons. Avoid oversized decorative icons.

### Code and JSON Editors

- **Style:** White editor surface in light mode, visible line gutter, monospace text, clear border.
- **Warnings:** Secret visibility warnings should be explicit and close to the editor.
- **Actions:** Format, reload, apply, and copy actions should use icon plus text when the command can mutate or replace user config.

## 6. Do's and Don'ts

### Do

- Use Ubuntu and system-native cues.
- Keep the desktop app minimal, structured, and quiet.
- Use greys, white surfaces, and subtle orange accents.
- Make selected, focused, loading, running, stopped, read-only, and exposed states obvious.
- Prefer standard file-manager and settings patterns.
- Use product screenshots on marketing surfaces.
- Keep credentials and MCP exposure language direct and visible.

### Don't

- Do not make the app look like a SaaS dashboard.
- Do not use purple, neon blue, or heavy gradients as the product identity.
- Do not make orange dominate the UI.
- Do not remove keyboard focus indicators.
- Do not rely on browser-native `alert()` or `confirm()` for product flows.
- Do not use cards inside cards unless each layer has a distinct functional boundary.
- Do not hide security-sensitive effects behind vague labels.
- Do not add decorative animation that does not explain state.
