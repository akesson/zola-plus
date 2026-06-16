+++
title = "Responsive tables"
weight = 85
+++

Markdown tables render as a normal HTML `<table>`, which overflows or squishes on
narrow screens. zola-plus can make an individual table *respond* to its container,
with no JavaScript, in one of two layouts:

- **reflow** — each row turns into a "card" of `Header  Value` pairs. Good for
  data tables with many rows, where each row is a self-contained record.
- **transpose** — the table flips, so the header row becomes a left-hand label
  column and each data row becomes a column. Good for short, wide tables (a
  feature comparison, a spec sheet) where you would rather read down the headers.

Both are **opt-in per table** and share the same mechanism: a comment on its own
line directly above the table, a breakpoint you choose, and a single generated
stylesheet. They differ only in the layout they produce.

## The directive

Put a `reflow` or `transpose` comment on its own line directly above the table,
with a **blank line between the comment and the table**:

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

A table with no directive is left exactly as it is. The two are mutually
exclusive — if both somehow precede one table, the last one wins.

## The breakpoint

The value is the **breakpoint**: when the table's container is narrower than it,
the table switches to its responsive layout. Any CSS length works — `rem`, `px`,
`em`, `ch`, `vw`, … You choose it by eye: narrow your window until the table
starts to feel cramped, and use that width.

Because the rules are driven by a [container query](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_containment/Container_queries)
(not a media query), the threshold reacts to the table's **own container width**,
not the viewport — so a table in a sidebar responds independently of one in the
main column, at any font size or zoom.

Here are two live ones. Narrow your window (or this column) and watch them change.
A **reflow** directory — each row folds into a card:

<!-- reflow: 40rem -->

| Name  | Role          | Location | Joined |
| ----- | ------------- | -------- | ------ |
| Ada   | Platform lead | Lisbon   | 2021   |
| Linus | Kernel        | Helsinki | 2019   |
| Grace | Compilers     | New York | 2020   |

A **transpose** comparison — the wide header row flips down the left:

<!-- transpose: 40rem -->

| Plan     | Free | Pro  | Team |
| -------- | ---- | ---- | ---- |
| Price/mo | $0   | $12  | $40  |
| Storage  | 5 GB | 1 TB | 5 TB |

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
        <td data-label="Location">Lisbon</td>
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
  <table>
    <thead><tr><th>Plan</th> … </tr></thead>
    <tbody>
      <tr><td>Price/mo</td><td>$0</td><td>$12</td><td>$40</td></tr>
      …
    </tbody>
  </table>
</div>
```

- `transpose-cols-4` records the column count, which the CSS needs to lay out the
  flipped grid.

In both cases the breakpoint lives in the class (`reflow-bp-40rem`,
`transpose-bp-40rem`) — a decimal point becomes an underscore, so `37.5rem` →
`…-bp-37_5rem`. The stable `reflow` / `transpose` class is your styling hook (see
[Styling](#styling)).

## The generated stylesheet

zola-plus collects every breakpoint used across your site and writes a single
`responsive-tables.css` into the output. Link it once — exactly like the
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

Why a generated file rather than a value you set inline? A container query can't
read a custom property in its condition — `@container (max-width: var(--bp))` is
invalid CSS — so the breakpoint has to live in a real rule keyed by a class.
(The transpose column count is keyed into a class for the same reason: `repeat()`
can't reliably take a `var()` as its count.) zola generates exactly the rules you
use.

## Styling

zola-plus's emitted CSS does the **layout only** — it adds no colours, fonts,
spacing, or borders. Every responsive table carries a stable `.reflow` or
`.transpose` class, your hook for styling it from your own stylesheet:

```scss
.reflow td {
  gap: 0.25rem 1rem;            // space between label and value
  text-align: left;            // override numeric alignment inside cards
}
.reflow td::before { font-weight: 600; }   // bolder labels
.reflow > table   { width: 100%; }         // full-width cards

.transpose > table { width: 100%; }
.transpose th      { text-align: left; }   // header column reads as labels
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
- **The value must be a CSS length** — a number plus a unit (`40rem`, `640px`,
  `60ch`, …). An unrecognised value is ignored and the table renders plain.
- **Accessibility.**
  - *reflow* keeps the real `<table>` and only *visually* hides the header row,
    but a card layout can still change how some screen readers announce the table.
  - *transpose* is a purely **visual** flip — the DOM and reading order are
    unchanged, so assistive technology still reads the table column-by-column as
    authored. It relies on `display: contents` for the table's `thead`/`tbody`/`tr`;
    this is well supported in current browsers, but very old engines may drop
    those elements from the accessibility tree.

  Test with assistive technology, and leave the annotation off for tables where
  the tabular relationship is essential.
- **Feeds and other consumers.** The wrapper `<div>` (and, for reflow, the
  `data-label` attributes) are part of the rendered body, so they also appear in
  your Atom/RSS feeds — but no CSS is injected into the body; it all lives in the
  linked `responsive-tables.css`. Without that stylesheet the markup is inert (the
  table just renders normally), so this degrades gracefully.
