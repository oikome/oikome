# Oikome plugin SDK

A dashboard plugin is a card on Home: a small web page in a folder, with a
manifest. The owner adds it in Oikome (Plugins → Add From Folder), chooses
which collections it may read, and it shows on Home.

## What a plugin can and can't do

- It reads only the collections its owner chose. The app reads them through
  the node *as the plugin*, and the node refuses any other; each read is in
  Activity.
- It can't reach the network: no requests, no images or fonts from
  elsewhere, no WebRTC, no new windows, no navigation away. It has no
  cookies or storage.
- It runs only its own files: no inline scripts, no `eval`.

## The folder

```
my-card/
  oikome-plugin.json
  index.html
  main.js
  style.css
```

`oikome-plugin.json`:

```json
{
  "id": "my-card",
  "name": "My card",
  "by": "Your name",
  "version": "1.0.0",
  "description": "One sentence: what it shows.",
  "reads": { "what": "A table with a day and an amount", "shape": "table", "columns": ["date", "number"] },
  "card": { "width": 4, "height": 240 },
  "entry": "index.html"
}
```

- `id`: lowercase letters, digits and `-`, starting with a letter.
- `reads.shape`: `table`, `photos`, `notes`, `series` (readings over time)
  or `any`; `reads.columns`: the kinds of column it needs (`date`, `number`,
  `text`, `image`, `bool`). Oikome offers the owner's collections of that
  kind with those columns, and the owner chooses.
- `card.width`: 4 to 12 of Home's twelve columns; `card.height`: 120 to 600
  pixels.
- Files: up to 200, 8 MB in all; `.html .js .css .json .svg .png .jpg
  .jpeg .webp .gif .woff2 .txt .md`.

## The bridge

Oikome puts `sdk.js` first in your page; use the `oikome` object it makes:

```js
oikome.onData(({ collections, theme, today }) => {
  const [first] = collections;
  // first.name, first.columns [{ name, type }], first.rows [{ ... }]
});
oikome.open("spending"); // opens one of your collections in the app
```

Column types are `text`, `number`, `date` (ISO text), `image` (a `data:`
URL), `bool` and `other`. Up to 1,000 rows of each collection come.

Style with the page's variables, set to match the owner's style (Ink or
Bold): `--oik-bg`, `--oik-raised`, `--oik-ink`, `--oik-ink2`, `--oik-ink3`,
`--oik-line`, `--oik-accent`, `--oik-on-accent`, `--oik-font`, `--oik-mono`,
`--oik-display`, `--oik-radius`. `html[data-theme]` is `ink` or `bold`.

Oikome's own plugins, in `plugins/` of the repository, are worked examples.
