+++
title = "Table reflow"
weight = 85
+++

Markdown tables render as a normal HTML `<table>`, which overflows or squishes on
narrow screens. zola-plus can make an individual table *reflow*: on a narrow
container each row turns into a "card" showing `Header  Value` pairs, with no
JavaScript.

It is **opt-in per table**. Put a `reflow` comment on its own line directly above
the table, with a **blank line between the comment and the table**:

```md
<!-- reflow: 40rem -->

| Name  | Department | Started |
| ----- | ---------- | ------- |
| Ada   | Platform   | 2021    |
| Linus | Kernel     | 2019    |
```

The value is the **breakpoint**: when the table's container is narrower than it,
the table reflows into cards. Any CSS length works — `rem`, `px`, `em`, `ch`,
`vw`, … You choose it by eye: narrow your window until the table starts to feel
cramped, and use that width. A table with no `reflow` comment is left exactly as
it is.

Here is a live one — it carries `<!-- reflow: 40rem -->`. Narrow your window (or
this column) and it folds into cards:

<!-- reflow: 40rem -->

| Name  | Department | Started |
| ----- | ---------- | ------- |
| Ada   | Platform   | 2021    |
| Linus | Kernel     | 2019    |

## What it emits

For an annotated table, zola-plus wraps it and labels its cells — but adds **no
CSS to the page itself**:

```html
<div class="reflow reflow-bp-40rem">
  <table>
    <thead><tr><th scope="col">Name</th> … </tr></thead>
    <tbody>
      <tr>
        <td data-label="Name">Ada</td>
        <td data-label="Department">Platform</td>
        <td data-label="Started">2021</td>
      </tr>
      …
    </tbody>
  </table>
</div>
```

- The breakpoint lives in the class: `reflow-bp-40rem` (a decimal point becomes an
  underscore, so `37.5rem` → `reflow-bp-37_5rem`).
- `data-label` on each body cell is its column header; the CSS shows it via
  `::before` once the table has reflowed.
- The stable `reflow` class is your styling hook (see [Styling the cards](#styling-the-cards)).

## The generated stylesheet

zola-plus collects every breakpoint used across your site and writes a single
`reflow.css` into the output. Link it once — exactly like the stylesheet for
class-based syntax highlighting:

```html
<link rel="stylesheet" href="/reflow.css">
```

Each distinct breakpoint becomes one [container query](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_containment/Container_queries):

```css
.reflow-bp-40rem { container-type: inline-size; }
@container (max-width: 40rem) {
  .reflow-bp-40rem thead { /* visually hidden */ }
  .reflow-bp-40rem tr { display: block; }
  .reflow-bp-40rem td { display: grid; grid-template-columns: auto 1fr; }
  .reflow-bp-40rem td::before { content: attr(data-label); }
}
```

Because it's a *container* query (not a media query), the threshold reacts to the
table's **own container width**, not the viewport — so a table in a sidebar reflows
independently of one in the main column, at any font size or zoom.

Why a generated file rather than a value you set inline? A container query can't
read a custom property in its condition — `@container (max-width: var(--bp))` is
invalid CSS — so the breakpoint has to live in a real rule keyed by a class. zola
generates those rules for you from the breakpoints you actually use.

## Styling the cards

zola-plus's emitted CSS does the **reflow only** — it adds no colours, fonts,
spacing, or borders. Every reflowed table also carries a stable `.reflow` class,
which is your hook for styling the cards however you like, from your own
stylesheet:

```scss
.reflow td {
  gap: 0.25rem 1rem;            // space between label and value
  text-align: left;            // override numeric alignment inside cards
}
.reflow td::before { font-weight: 600; }   // bolder labels
.reflow > table   { width: 100%; }         // full-width cards
```

## Limitations & notes

- **Inline content only.** Markdown table cells can hold inline markup (bold,
  links, inline code, inline HTML) but *not* block content — lists, `<div>`s, or
  multiple paragraphs are not expressible in a pipe table. Use `<br>` for line
  breaks, or write the whole table as raw HTML, if you need more.
- **The blank line is required.** Without it the table is absorbed into the HTML
  comment block and won't render as a table at all. The comment applies only to
  the table that *immediately* follows it — if other content comes in between, the
  comment is ignored (so a stray `reflow` can't silently affect an unrelated table
  further down the page).
- **The value must be a CSS length** — a number plus a unit (`40rem`, `640px`,
  `60ch`, …). An unrecognised value is ignored and the table renders plain.
- **Accessibility.** The real `<table>` element is kept and the header row is only
  hidden visually, but a card layout can still change how some screen readers
  announce the table. Test with assistive technology, and leave the annotation off
  for tables where the tabular relationship is essential.
- **Feeds and other consumers.** The wrapper `<div>` and the `data-label`
  attributes are part of the rendered body, so they also appear in your Atom/RSS
  feeds — but no CSS is injected into the body; it all lives in the linked
  `reflow.css`. Without that stylesheet the markup is inert (the table just renders
  normally), so this degrades gracefully.
