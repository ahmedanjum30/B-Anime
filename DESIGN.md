---
name: B-Anime
description: A fast desktop home for a personal anime and manga library, set like an evening's TV listings.
colors:
  ink-black: "#121318"
  graphite: "#191b22"
  graphite-raised: "#20232c"
  graphite-line: "#2a2e3a"
  graphite-strong: "#3a3f4e"
  slate-muted: "#555b6d"
  slate-text: "#878da1"
  pewter-text: "#b9bcc6"
  newsprint-white: "#ece7dc"
  listings-red: "#e5483b"
  final-amber: "#e2a43b"
  synced-teal: "#3fa79c"
  channel-violet: "#8d7be3"
typography:
  display:
    fontFamily: "Archivo Narrow, Archivo, system-ui, sans-serif"
    fontSize: "32px"
    fontWeight: 700
    lineHeight: "38px"
    letterSpacing: "0.01em"
  headline:
    fontFamily: "Archivo Narrow, Archivo, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 700
    lineHeight: "16px"
    letterSpacing: "0.12em"
  title:
    fontFamily: "Archivo, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: "20px"
  body:
    fontFamily: "Archivo, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "20px"
  label:
    fontFamily: "Archivo Narrow, Archivo, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 700
    lineHeight: "16px"
    letterSpacing: "0.1em"
rounded:
  mark: "2px"
  sm: "4px"
spacing:
  u: "8px"
  sm: "16px"
  md: "24px"
  lg: "40px"
components:
  button-primary:
    backgroundColor: "{colors.listings-red}"
    textColor: "#ffffff"
    rounded: "{rounded.sm}"
    padding: "0 14px"
    height: "32px"
  button-secondary:
    backgroundColor: "{colors.graphite-line}"
    textColor: "{colors.newsprint-white}"
    rounded: "{rounded.sm}"
    padding: "0 14px"
    height: "32px"
  button-secondary-hover:
    backgroundColor: "{colors.graphite-strong}"
  input-search:
    backgroundColor: "{colors.graphite-raised}"
    textColor: "{colors.newsprint-white}"
    rounded: "{rounded.sm}"
    padding: "0 14px"
    height: "40px"
  card-upnext:
    backgroundColor: "{colors.graphite-raised}"
    rounded: "{rounded.sm}"
  ep-chip:
    backgroundColor: "{colors.newsprint-white}"
    textColor: "{colors.ink-black}"
    rounded: "{rounded.mark}"
    padding: "2px 6px"
---

# Design System: B-Anime

## Overview

**Creative North Star: "Broadcast Listings"**

Your library is your own broadcast schedule. Screens read like an evening's TV listings: what's on next, then the week set in a listings-column rhythm. The system refuses the streaming-service banner-and-rows wall; the user's cover art and banners are the only imagery, and the chrome recedes into a dark graphite scene built for evening, lean-back use.

Density is comfortable, not sparse: listings columns, tabular numerals and uppercase narrow column heads carry a lot of information without clutter. Status always shows its evidence (when it last synced, how many files matched), and watched state is stamped, never hidden.

**Key Characteristics:**
- Fixed graphite tonal ramp; no ad-hoc greys.
- Each accent colour does exactly one job.
- Newsprint rules: 2px under section heads, 1px between columns.
- Boxed glyph marks (NEW, FINAL), at most one per item.
- Artwork leads; chrome is flat and quiet.

## Colors

A stepped graphite ramp under newsprint-white type, with four listings colours that each carry one meaning.

### Primary
- **Listings Red** (#e5483b): NEW marks, "now", progress bars, the active nav icon, the today underline and the one primary action per view. Never decorative.

### Secondary
- **Final Amber** (#e2a43b): FINAL marks and warnings (offline copy, can't reach AniList).

### Tertiary
- **Synced Teal** (#3fa79c): synced and healthy states only.
- **Channel Violet** (#8d7be3): a fourth channel colour for listings, never UI chrome.

### Neutral
- **Ink Black** (#121318): the rail and deepest ground.
- **Graphite** (#191b22): the page ground.
- **Graphite Raised** (#20232c): cards, inputs, hover and active rows.
- **Graphite Line** (#2a2e3a): 1px column and row rules, secondary buttons.
- **Graphite Strong** (#3a3f4e): stronger rules, progress tracks, button hover.
- **Slate Muted** (#555b6d): disabled and faint swatches.
- **Slate Text** (#878da1): secondary text (5:1 on Graphite).
- **Pewter Text** (#b9bcc6): tertiary-emphasis text, nav labels, meta lines.
- **Newsprint White** (#ece7dc): primary text and the 2px section rules.

**The One Job Rule.** Each accent colour means one thing. Red is NEW/now/progress/primary action; amber is FINAL/warning; teal is synced. Never use an accent for decoration.

**The Fixed Ramp Rule.** Neutrals come only from the ramp (Ink Black → Newsprint White). Never introduce an ad-hoc grey.

## Typography

**Display Font:** Archivo Narrow (with Archivo, system-ui)
**Body Font:** Archivo (with system-ui)

**Character:** A condensed newsprint voice for column heads, times and marks, over a sturdy grotesque for reading and controls.

### Hierarchy
- **Display** (700, 32px, 38px): show titles on the show page, balanced, max 28ch.
- **Headline** (700, 13px, 0.12em, uppercase): section heads ("UP NEXT", "THIS WEEK") over a 2px Newsprint White rule.
- **Title** (600, 15px, 20px): card titles.
- **Body** (400, 14px, 20px): text and controls; synopsis at 22px line height, max 72ch.
- **Label** (700, 11px, 0.1em, uppercase): select labels, editor labels, marks, ep chips.

**The Tabular Numbers Rule.** Episode numbers, counts, times and dates use tabular numerals (`.num`).

## Layout

An 8px cell grid: every size is a multiple of 8 (`--u`). The desktop shell is a 232px left rail (wordmark, nav with counts, Support, update notice, sync evidence at the foot) beside a scrolling main column, padded 24px top and 40px sides, max 1600px.

- Up next: 4 equal columns of 16:9 cards (2 under 1100px, 6 over 1900px), 16px gaps.
- This week: a 7-day listings table, days across and time rows down, today underlined red.
- Library: a virtualised 2:3 cover grid with 24px column gap and a proportional A–Z rail.
- Show page: 280px banner hero fading into the page, a 248px side column (cover, list editor, facts) overlapping it, and the main column.
- Under 1100px the rail collapses to 72px icons.

## Elevation & Depth

Flat by default, using tonal layering: depth comes from stepping up the graphite ramp (Graphite → Graphite Raised → Graphite Line), not from shadows.

### Shadow Vocabulary
- **Cover lift** (`box-shadow: 0 16px 40px rgb(0 0 0 / 0.45)`): the show page cover over the banner.
- **Play badge** (`box-shadow: 0 4px 12px rgb(0 0 0 / 0.35)`): the red play disc on hover.

**The Flat-By-Default Rule.** Surfaces are flat at rest; shadows only lift artwork over artwork.

## Shapes

Gently squared: 4px corners on cards, inputs, buttons and covers; 2px on marks and chips; full circles only for the play disc, the info button and status dots. Boxed marks use a 1.5px currentColor stroke.

## Components

### Buttons
- **Shape:** gently squared (4px), 32px tall, 14px side padding, 600 weight.
- **Primary:** Listings Red with white text; one per view at most.
- **Secondary:** Graphite Line, hover Graphite Strong.
- **Focus:** 2px Listings Red outline, 2px offset.

### Inputs / Fields
- **Search:** Graphite Raised fill, 1px Graphite Line stroke, 4px radius, 40px tall; focus shifts the stroke to Slate Muted.
- **Select:** same shell with an uppercase Label before the value.

### Navigation
- **Rail links:** 36px rows, Pewter Text labels with Slate Text icons; hover and active fill Graphite Raised, the active icon turns Listings Red. Counts sit right in tabular Label type.
- **Tabs:** uppercase Archivo Narrow over a 2px Newsprint White rule; the selected tab gets a 2px red underline.

### Up next card (signature)
16:9 artwork on Graphite Raised, a Newsprint White EP chip bottom-left, a red play disc that appears on hover, then title (with one mark), meta line, source file and a 3px red progress bar.

### Listings slot (signature)
A channel-colour swatch, the title, then a line with an optional mark and "Ep N"; watched slots are stamped "watched".

### Boxed marks
NEW (red) and FINAL (amber): 18px tall, 1.5px currentColor box, 11px Archivo Narrow, at most one per item.

## Do's and Don'ts

### Do:
- **Do** keep every neutral on the graphite ramp.
- **Do** set section heads in uppercase Archivo Narrow over a 2px Newsprint White rule.
- **Do** let cover art and banners be the only imagery; fall back to the show's AniList colour.
- **Do** show status with its evidence (time since sync, files matched).

### Don't:
- **Don't** build a streaming-service banner-and-rows wall.
- **Don't** use an accent colour for decoration or give an item more than one mark.
- **Don't** add shadows to flat surfaces.
