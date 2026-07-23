+++
title = "Responsive tables"
weight = 85
+++

Markdown tables render as a normal HTML `<table>`, which overflows or squishes on
narrow screens. zola-plus can make an individual table *respond* to its container,
with no JavaScript, in one of four ways. They fall into two groups:

**Reshape the table so it fits** the narrow container:

- **reflow** — each row turns into a "card" of `Header  Value` pairs. Good for
  data tables with many rows, where each row is a self-contained record.
- **transpose** — the table flips, so the header row becomes a left-hand label
  column and each data row becomes a column. Good for short, wide tables (a
  feature comparison, a spec sheet) where you would rather read down the headers.

**Keep the table at full size and give a way to navigate it:**

- **scroll** — a wide table pans horizontally inside its column instead of
  overflowing the page. The simplest option; no breakpoint.
- **expand** — adds a button that opens the table in a full-viewport overlay you
  can pan around, then close. Good for large tables you want to inspect in full.

All four are **opt-in per table** and share the same mechanism: a comment on its
own line directly above the table and a single generated stylesheet. They differ
only in what they produce.

## Choosing one

| You want… | Use |
| --------- | --- |
| Many-row records to stay readable when narrow | `reflow` |
| A short, wide comparison read down its headers | `transpose` |
| A wide table to pan in place without leaving the column | `scroll` |
| A big table you can blow up to full screen and pan | `expand` |

A table can carry only one directive; they are mutually exclusive.

## The directive

Put the directive on its own line directly above the table, with a **blank line
between the comment and the table**:

```md
<!-- reflow: 40rem -->

| Name  | Role          | Location | Joined |
| ----- | ------------- | -------- | ------ |
| Ada   | Platform lead | Lisbon   | 2021   |
| Linus | Kernel        | Helsinki | 2019   |
| Grace | Compilers     | New York | 2020   |
```

```md
<!-- transpose: 40rem -->

| Plan     | Free | Pro  | Team |
| -------- | ---- | ---- | ---- |
| Price/mo | $0   | $12  | $40  |
| Storage  | 5 GB | 1 TB | 5 TB |
```

```md
<!-- scroll -->

| Region | Q1 | Q2 | Q3 | Q4 | Q5 | Q6 | Q7 | Q8 |
| ------ | -- | -- | -- | -- | -- | -- | -- | -- |
| EMEA   | 120 | 132 | 145 | 151 | 160 | 171 | 180 | 199 |
```

```md
<!-- expand: 40rem -->

| Service     | us-east | us-west | eu-west | ap-south | ap-east |
| ----------- | ------- | ------- | ------- | -------- | ------- |
| api-gateway | 12ms    | 18ms    | 40ms    | 120ms    | 132ms   |
| auth        | 9ms     | 14ms    | 38ms    | 118ms    | 129ms   |
```

A table with no directive is left exactly as it is.

There is also a fifth, non-responsive directive: `<!-- table: <length> -->` gives
a table that fits everywhere a fixed, centered panel width (see
[Widths and sticky columns](#widths-and-sticky-columns)).

## The breakpoint

`reflow`, `transpose`, and `expand` take a **breakpoint** — any CSS length (`rem`,
`px`, `em`, `ch`, `vw`, …). You choose it by eye: narrow your window until the
table starts to feel cramped, and use that width. It means slightly different
things per option:

- for **reflow** / **transpose** it is *where the table switches* to its
  responsive layout — wider than it, the table is normal; narrower, it reshapes;
- for **expand** it is *where the button appears* — wider than it the table fits,
  so no button is offered; narrower, the expand control shows up.

**scroll** takes no value: `overflow-x: auto` is harmless while the table fits, so
there is nothing to switch.

Because the rules are driven by a [container query](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_containment/Container_queries)
(not a media query), the threshold reacts to the table's **own container width**,
not the viewport — so a table in a sidebar responds independently of one in the
main column, at any font size or zoom.

Skip to the [live demos](#demos) below to see each one respond.

## Widths and sticky columns

Directive arguments are whitespace-separated tokens after the colon. A bare CSS
length is the directive's primary value — the breakpoint for `reflow` /
`transpose` / `expand`, the panel width for `table` and `scroll`. Two named
arguments extend that:

- **`width <length>`** — on any directive: the wrapper gets an inline
  `--table-w: <length>`, and the generated stylesheet sizes it
  `min(var(--table-w, 100%), 100%)`, centered. Use it to pin a table's panel to a
  deliberate width (e.g. one measured and snapped to a scale at authoring time)
  instead of whatever `fit-content` happens to produce:

  ```md
  <!-- reflow: 37rem width 54rem -->
  ```

  The width-only form is the `table` directive: `<!-- table: 54rem -->` wraps the
  table in `<div class="table-width" style="--table-w: 54rem">` with no pivot.

- **`sticky [<length>]`** — `scroll` only: the first column is pinned
  (`position: sticky`) while the rest pans behind it, so row labels stay
  readable. Bare `sticky` keeps the column at its natural width; `sticky 6rem`
  also clamps it to that width and lets its labels wrap
  (`--sticky-w`). Give the pinned cells an opaque background in your own CSS —
  without one the panning content shows through them:

  ```md
  <!-- scroll: sticky 6rem width 66rem -->
  ```

An invalid token (a typo, `sticky` off `scroll`, a bad length) drops the whole
directive with a build warning, so the table renders plain rather than
half-styled.

## Group rows

On **every** table — with or without a directive — a body row whose cells after
the first are all dashes (3+) is a **group header**. It echoes the delimiter
row, so the source reads as a section divider:

```md
| flag               | arity | effect                 |
|--------------------|-------|------------------------|
| **sync behaviour** |-------|------------------------|
| `--no-sync`        | 0     | skip the snapshot push |
```

The dash-row becomes `<tr class="group"><th colspan="3">…</th></tr>` — a
full-width heading whose content is the first cell, inline markdown intact.
zola-plus emits no styling for it; style `tr.group th` yourself. Caveat: outside
zola-plus (e.g. a GitHub preview of the same markdown) the dashes render as
literal cell text.

## What it emits

zola-plus wraps the table and adds **no CSS to the page itself** — the layout
ships in the linked stylesheet (see below).

**reflow** wraps the table and labels its cells:

```html
<div class="reflow reflow-bp-40rem">
  <table>
    <thead><tr><th scope="col">Name</th> … </tr></thead>
    <tbody>
      <tr>
        <td data-label="Name">Ada</td>
        <td data-label="Role">Platform lead</td>
        …
      </tr>
      …
    </tbody>
  </table>
</div>
```

- `data-label` on each body cell is its column header; the CSS shows it via
  `::before` once the table has reflowed.

**transpose** only wraps the table — the cells are untouched:

```html
<div class="transpose transpose-bp-40rem transpose-cols-4">
  <table> … </table>
</div>
```

- `transpose-cols-4` records the column count, which the CSS needs to lay out the
  flipped grid.

**scroll** wraps the table in a single hook:

```html
<div class="table-scroll">
  <table> … </table>
</div>
```

With `sticky` the wrapper also carries `table-scroll-sticky` (and, clamped,
`table-scroll-clamp` plus an inline `--sticky-w`); a `width` argument on any
directive adds an inline `--table-w`:

```html
<div class="table-scroll table-scroll-sticky table-scroll-clamp"
     style="--table-w: 66rem; --sticky-w: 6rem">
  <table> … </table>
</div>
```

**table** (width only) wraps the table in:

```html
<div class="table-width" style="--table-w: 54rem">
  <table> … </table>
</div>
```

**expand** wraps the table with an id (the `:target` of the overlay), an open and a
close link, and a pannable inner area:

```html
<div class="table-expand table-expand-bp-40rem" id="table-expand-1">
  <a class="table-expand-open" href="#table-expand-1" aria-label="View table fullscreen">⛶</a>
  <a class="table-expand-close" href="#!" aria-label="Close fullscreen">✕</a>
  <div class="table-expand-scroll">
    <table> … </table>
  </div>
</div>
```

- The overlay is pure CSS: clicking the open link points the page fragment at the
  wrapper's id, and the stylesheet's `.table-expand:target` rule restyles that same
  element to fill the viewport — no JavaScript and no duplicated table. Each
  expandable table on a page gets a unique `table-expand-N` id. The close link's
  `#!` fragment matches no element, which clears `:target` without scrolling
  (`href="#"` would jump to the top of the page).

For `reflow`, `transpose`, and `expand` the breakpoint lives in the class
(`reflow-bp-40rem`, …) — a decimal point becomes an underscore, so `37.5rem` →
`…-bp-37_5rem`. The stable `reflow` / `transpose` / `table-scroll` / `table-expand`
class is your styling hook (see [Styling](#styling)).

## The generated stylesheet

zola-plus collects every feature and breakpoint used across your site and writes a
single `responsive-tables.css` into the output. Link it once — exactly like the
stylesheet for class-based syntax highlighting:

```html
<link rel="stylesheet" href="/responsive-tables.css">
```

Each distinct **reflow** breakpoint becomes one container query that hides the
header row and lays each row out as a label/value grid:

```css
.reflow-bp-40rem { container-type: inline-size; }
@container (max-width: 40rem) {
  .reflow-bp-40rem thead { /* visually hidden */ }
  .reflow-bp-40rem tr { display: block; }
  .reflow-bp-40rem td { display: grid; grid-template-columns: auto 1fr; }
  .reflow-bp-40rem td::before { content: attr(data-label); }
}
```

Each distinct **transpose** `(breakpoint, column-count)` becomes one container
query that turns the table into a column-first CSS grid — which flips it:

```css
.transpose-bp-40rem { container-type: inline-size; }
@container (max-width: 40rem) {
  .transpose-bp-40rem.transpose-cols-4 table {
    display: grid;
    grid-auto-flow: column;
    grid-template-rows: repeat(4, auto);
  }
  .transpose-bp-40rem thead,
  .transpose-bp-40rem tbody,
  .transpose-bp-40rem tr { display: contents; }
}
```

**scroll** is a few static rules (no breakpoint) — the sticky ones are inert
unless a directive asked for them:

```css
.table-scroll { overflow-x: auto; }
.table-scroll-sticky th:first-child,
.table-scroll-sticky td:first-child { position: sticky; left: 0; }
.table-scroll-clamp th:first-child,
.table-scroll-clamp td:first-child { max-width: var(--sticky-w, 6rem); white-space: normal; }
```

Any **width** usage adds one static rule making wrappers obey their inline
`--table-w` (the `min()` fallback keeps width-less wrappers at their natural
100%):

```css
.table-width, .reflow, .transpose, .table-scroll, .table-expand {
  width: min(var(--table-w, 100%), 100%);
  margin-inline: auto;
}
```

**expand** is one static block of overlay *layout* plus, per breakpoint, a query
that reveals the button only while the container is narrower than it:

```css
.table-expand { position: relative; container-type: inline-size; }
.table-expand .table-expand-scroll { overflow: auto; }
.table-expand .table-expand-open { position: absolute; top: 0; right: 0; display: none; }
.table-expand .table-expand-close { display: none; }
.table-expand:target { position: fixed; inset: 0; z-index: 1000; display: flex; flex-direction: column; }
.table-expand:target .table-expand-open { display: none; }
.table-expand:target .table-expand-scroll { flex: 1 1 auto; min-height: 0; }
.table-expand:target .table-expand-close { display: block; align-self: flex-end; }
@container (max-width: 40rem) {
  .table-expand-bp-40rem .table-expand-open { display: block; }
}
```

Why a generated file rather than a value you set inline? A container query can't
read a custom property in its condition — `@container (max-width: var(--bp))` is
invalid CSS — so the breakpoint has to live in a real rule keyed by a class. (The
transpose column count is keyed into a class for the same reason: `repeat()` can't
reliably take a `var()` as its count.) zola generates exactly the rules you use.

## Demos

Each demo below sits in a **resizable frame**. Because the tables respond to their
*container's* width — a container query, not the viewport — the frame starts
narrower than the `40rem` breakpoint, so you see the responsive layout straight
away, and you can **drag its bottom-right corner wider** to watch it return to a
normal table. No need to resize your browser.

### reflow — rows become cards

<div class="table-demo">

<!-- reflow: 40rem -->

| Name  | Role          | Location | Joined |
| ----- | ------------- | -------- | ------ |
| Ada   | Platform lead | Lisbon   | 2021   |
| Linus | Kernel        | Helsinki | 2019   |
| Grace | Compilers     | New York | 2020   |

</div>

Each row has folded into a `Header  Value` card. Widen the frame past 40rem and the
header row and columns come back.

### transpose — the table flips

<div class="table-demo">

<!-- transpose: 40rem -->

| Plan     | Free | Pro  | Team |
| -------- | ---- | ---- | ---- |
| Price/mo | $0   | $12  | $40  |
| Storage  | 5 GB | 1 TB | 5 TB |

</div>

The header row (`Plan / Free / Pro / Team`) has become the left-hand label column,
and each plan now reads down its own column.

### scroll — pan a wide table

<div class="table-demo">

<!-- scroll -->

| Region | Q1  | Q2  | Q3  | Q4  | Q5  | Q6  | Q7  | Q8  | Total |
| ------ | --- | --- | --- | --- | --- | --- | --- | --- | ----- |
| EMEA   | 120 | 132 | 145 | 151 | 160 | 171 | 180 | 199 | 1258  |
| APAC   | 90  | 101 | 110 | 128 | 140 | 155 | 166 | 178 | 1068  |
| AMER   | 210 | 220 | 231 | 240 | 255 | 262 | 277 | 288 | 1983  |

</div>

The table is wider than the frame, so it pans horizontally inside it. (`scroll` has
no breakpoint — it simply scrolls whenever it doesn't fit.)

### expand — open it full-screen

<div class="table-demo">

<!-- expand: 40rem -->

| Service     | us-east | us-west | eu-west | eu-north | ap-south | ap-east | sa-east |
| ----------- | ------- | ------- | ------- | -------- | -------- | ------- | ------- |
| api-gateway | 12ms    | 18ms    | 40ms    | 44ms     | 120ms    | 132ms   | 88ms    |
| auth        | 9ms     | 14ms    | 38ms    | 41ms     | 118ms    | 129ms   | 83ms    |
| billing     | 22ms    | 27ms    | 52ms    | 55ms     | 140ms    | 151ms   | 99ms    |

</div>

Because the frame is narrower than 40rem, the **⛶** button is showing. Click it to
open the table in a full-viewport overlay you can pan; **✕** (or your browser's Back
button) closes it. Widen the frame past 40rem and the button goes away.

## Styling

zola-plus's emitted CSS does the **layout only** — it adds no colours, fonts,
spacing, or borders. Every responsive table carries a stable class, your hook for
styling it from your own stylesheet:

```scss
.reflow td {
  gap: 0.25rem 1rem;           // space between label and value
  text-align: left;            // override numeric alignment inside cards
}
.reflow td::before { font-weight: 600; }   // bolder labels

.transpose > table { width: 100%; }
.transpose th      { text-align: left; }   // header column reads as labels

.table-scroll { scrollbar-width: thin; }   // a slimmer scrollbar, say
```

**`expand` needs a little styling to be usable.** Its generated CSS is layout
only: the overlay has **no background**, so without your styling the page shows
through behind the table, and the button is unstyled. Supply an opaque background
and some button chrome via the `.table-expand` hook — for example:

```scss
.table-expand {
  // the open (⛶) and close (✕) controls — colour only, never `display`
  // (the generated CSS decides when each one shows)
  .table-expand-open,
  .table-expand-close {
    width: 2rem; height: 2rem; border-radius: 4px;
    background: #111; color: #fff;
    text-align: center; line-height: 2rem; text-decoration: none;
  }

  &:target {
    background: #fff;          // the opaque backdrop the overlay needs
    padding: 0 1rem 1rem;
    .table-expand-close { margin: 0.75rem 0; }
  }
}
```

## Limitations & notes

- **Inline content only.** Markdown table cells can hold inline markup (bold,
  links, inline code, inline HTML) but *not* block content — lists, `<div>`s, or
  multiple paragraphs are not expressible in a pipe table. Use `<br>` for line
  breaks, or write the whole table as raw HTML, if you need more.
- **The blank line is required.** Without it the table is absorbed into the HTML
  comment block and won't render as a table at all. The comment applies only to
  the table that *immediately* follows it — if other content comes in between, the
  comment is ignored (so a stray directive can't silently affect an unrelated
  table further down the page).
- **The breakpoint must be a CSS length** — a number plus a unit (`40rem`,
  `640px`, `60ch`, …). An unrecognised value is ignored and the table renders
  plain. (`scroll` takes no value.)
- **Accessibility.**
  - *reflow* keeps the real `<table>` and only *visually* hides the header row,
    but a card layout can still change how some screen readers announce the table.
  - *transpose* is a purely **visual** flip — the DOM and reading order are
    unchanged, so assistive technology still reads the table column-by-column as
    authored. It relies on `display: contents` for the table's `thead`/`tbody`/`tr`;
    this is well supported in current browsers, but very old engines may drop
    those elements from the accessibility tree.
  - *expand* is also a purely **visual** convenience over a table that is already
    fully in the DOM — but the overlay is **not a real modal dialog**: focus is not
    trapped, and <kbd>Esc</kbd> does not close it (the browser Back button, or the
    ✕ link, does). The open/close controls are real links, so they are keyboard
    reachable. A horizontally **scrolled** or expanded table can also hide columns
    off-screen; keep the scroll affordance visible.

  Test with assistive technology, and leave the annotation off for tables where
  the tabular relationship is essential.
- **Feeds and other consumers.** The wrapper `<div>` (and, for reflow, the
  `data-label` attributes; for expand, the open/close `<a>` links) are part of the
  rendered body, so they also appear in your Atom/RSS feeds — but no CSS is
  injected into the body; it all lives in the linked `responsive-tables.css`.
  Without that stylesheet the markup is inert (the table just renders normally), so
  this degrades gracefully.
