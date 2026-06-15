+++
title = "Responsive Tables"
weight = 85
+++

Markdown tables render as a normal HTML `<table>`, which overflows or squishes on
narrow screens. Zola can optionally make every table *responsive*: on a narrow
container each row reflows into a "card" showing `Header  Value` pairs, with no
JavaScript. It is the build-time equivalent of injecting the header labels at
runtime — Zola writes a `data-label` onto each cell so CSS alone can do the rest.

This is **off by default**. Enable it under `[markdown]`:

```toml
[markdown]
responsive_tables = true
```

With it on, a standard markdown table:

```md
| Name  | Department | Started |
| ----- | ---------- | ------- |
| Ada   | Platform   | 2021    |
| Linus | Kernel     | 2019    |
```

is wrapped and annotated at build time:

```html
<div class="rt rt--cards rt--bp-40" style="--rt-label:10ch">
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

- `rt--bp-40` is the breakpoint, in `ch` (characters). It is estimated from the
  table's own content (with a safety margin), so each table cards at roughly the
  width *it* needs — erring slightly wide so content never overflows first.
- `--rt-label` is the longest header's width, so the card's label column can be
  sized to align every value.
- `data-label` on each body cell is its column header, used by the CSS.

The page you are reading has this feature enabled — narrow your window (or this
table's column) and the table above will reflow into cards.

## The stylesheet

The emitted classes do nothing until you add the matching CSS. Zola ships no
stylesheet (just like class-based syntax highlighting), so copy the partial below
into your `sass/` folder and import it:

```scss
// sass/_responsive-tables.scss  →  @import "responsive-tables";
$rt-bp-step: 4; // ch per rung   — keep in sync with Zola (16ch … 160ch / 4ch)
$rt-bp-from: 4;
$rt-bp-to: 40;

.rt { container-type: inline-size; } // the wrapper is the query container
.rt > table { width: 100%; }

@mixin rt-cards {
  thead { position: absolute; width: 1px; height: 1px; margin: -1px;
          padding: 0; border: 0; overflow: hidden; clip: rect(0 0 0 0);
          white-space: nowrap; }              // hidden visually, kept for screen readers
  tr  { display: block; margin-block-end: 1rem; }
  td  { display: grid; grid-template-columns: var(--rt-label, 8ch) 1fr;
        gap: 0.25rem 1rem; text-align: left; }
  td::before { content: attr(data-label); font-weight: 600; }
}

@for $i from $rt-bp-from through $rt-bp-to {
  $w: $i * $rt-bp-step;
  .rt--bp-#{$w}.rt--cards {
    @container (max-width: #{$w}ch) { @include rt-cards; }
  }
}
```

It uses a [container query](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_containment/Container_queries)
rather than a media query, so the breakpoint reacts to the table's *own*
container width (not the viewport) and the `ch` unit resolves against the table's
real font — the table cards correctly whether it sits in a sidebar or the main
column, at any font size or zoom.

## Choosing the breakpoint per table

By default the breakpoint is estimated from the number of columns and the width of
the content. To override it for a single table, put a directive comment on the line
before the table:

```md
<!-- rt: bp=64 -->

| … | … |
```

- `bp=64` — force the breakpoint to 64 `ch` (snapped to the nearest ladder rung).
- `bp=auto` — the default (estimate from content).
- `off` (or `none`) — leave this one table as a plain, non-responsive table.

## Limitations & notes

- **Inline content only.** Markdown table cells can hold inline markup (bold,
  links, inline code, inline HTML) but *not* block content — lists, `<div>`s, or
  multiple paragraphs are not expressible in a pipe table. Use `<br>` for line
  breaks, or write the whole table as raw HTML, if you need more.
- **Keep the table's font on the wrapper.** The `ch` math is measured against the
  `.rt` wrapper's font. Both inherit from `<body>` by default, so this is
  automatic — just don't set a *different* font specifically on `table`/`td` than
  on `.rt`.
- **Accessibility.** The real `<table>` element is kept and the header row is only
  hidden visually, but a card layout can still change how some screen readers
  announce the table. Test with assistive technology, and prefer `off` for tables
  where the tabular relationship is essential.
