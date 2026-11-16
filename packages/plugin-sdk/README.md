# Oikome plugin SDK

A plugin is a folder with an `oikome-plugin.json` manifest and, when it offers
Dashboard cards, its web files. Official packages and packages installed from a
folder use the same manifest, validation and lifecycle. Being bundled with
Oikome grants no extra runtime privilege.

Installing a package only registers its capabilities. It does not grant a
collection, configure a source or place a card on Home. The owner configures
each capability from the corresponding screen.

## Capability kinds

- `cards`: Dashboard cards on Home. Their web code runs in the existing iframe
  sandbox and reads only collections granted to the plugin.
- `sources`: offers configuration through an existing node connector. This
  declaration does not load package code into the node; third-party connector
  execution is not open yet.
- `preview`: registers format declarations only. A separate host sandbox is
  required before third-party preview code can run; declaring `preview` does
  not enable execution.
- `mcpClients`: describes manual or one-click setup for an MCP client. Official
  adapters bundled with Oikome are installed by default and can be removed
  persistently; an external package must be installed first. Only installed
  adapters appear in MCP Access. No model or provider profile is selected.

A package may contain several capabilities. Current collection grants are
package-wide and shared by its cards; adding the first card is when the owner
chooses them. Sources and MCP connections do not inherit Dashboard grants.

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
  "capabilities": {
    "cards": [
      {
        "id": "summary",
        "name": "My summary",
        "reads": "A table with a day and an amount",
        "shape": "table",
        "columns": ["date", "number"],
        "width": 4,
        "height": 240,
        "entry": "index.html"
      }
    ]
  }
}
```

- Package and capability `id` values use lowercase letters, digits and `-`,
  starting with a letter. Capability ids are unique within their kind.
- `shape` is `table`, `photos`, `notes`, `series` or `any`; `columns` names the
  kinds needed (`date`, `number`, `text`, `image`, `bool`). Oikome offers
  matching collections when the card is added, and the owner chooses.
- `width` is 4 to 12 of Home's twelve columns; `height` is 120 to 600 pixels.
- Files: up to 200, 8 MB in all; `.html .js .css .json .svg .png .jpg .jpeg
  .webp .gif .woff2 .txt .md`.
- The legacy single-card top-level fields (`reads`, `card`, `entry`) still read
  as one card with id `card`, preserving its existing Home id.

## Source declarations

```json
{
  "sources": [
    { "id": "export", "name": "Example export", "kind": "flomo-export" }
  ]
}
```

`kind` must name a connector already in the node. The declaration adds a setup
choice on Sources; the actual connector and its normal destination and sync
rules remain in the node.

## Preview declarations

```json
{
  "preview": [
    { "id": "raw", "name": "RAW Preview", "formats": ["dng", "cr3"] }
  ]
}
```

Formats are lowercase filename extensions without dots. This is registry
metadata, not a preview runtime. A future host must provide one-file read
access, a private temporary directory, resource limits and no default network.

## MCP client declarations

A command adapter uses `{url}` and `{token}` placeholders:

```json
{
  "mcpClients": [
    {
      "id": "example-cli",
      "name": "Example Code",
      "command": "example mcp add --transport http oikome {url} --header \"Authorization: Bearer {token}\""
    }
  ]
}
```

A declarative configuration adapter supports JSON or TOML:

```json
{
  "mcpClients": [
    {
      "id": "example-cli",
      "name": "Example Code",
      "config": {
        "format": "json",
        "homeEnv": "EXAMPLE_HOME",
        "directory": ".example",
        "file": "mcp.json",
        "root": "mcpServers",
        "headers": "headers"
      }
    }
  ]
}
```

When `homeEnv` is set, the file is directly below that directory; otherwise it
is `directory/file` below the user's home. Oikome validates a loopback `/mcp`
address, preserves unrelated settings, writes atomically, uses 0600 for new
files and asks before replacing an existing Oikome entry. TOML normally uses
`root: "mcp_servers"` and `headers: "http_headers"`.

## Official repository

[Oikome Plugins](https://github.com/oikome/oikome-plugins) is the independent
source repository for official packages. Its release workflow creates one ZIP
per package version and an index containing each complete manifest, download
URL, byte size and SHA-256. Oikome's Plugins screen lists the release index,
verifies the downloaded size, digest and manifest, and installs the package
through the same lifecycle as a local folder. Installation still has no data or
Home side effects. Packages must address a concrete user need and prove their
real data behavior; none are added merely to demonstrate the catalog.
Publisher signing and third-party community distribution remain separate later
trust and distribution work.

## Dashboard bridge

Oikome puts `sdk.js` first in a card page; use the `oikome` object it makes:

```js
oikome.onData(({ collections, theme, today, language }) => {
  const [first] = collections;
  // first.name, first.columns [{ name, type }], first.rows [{ ... }]
});
oikome.open("spending"); // opens one of the plugin's granted collections
```

Column types are `text`, `number`, `date` (ISO text), `image` (a `data:` URL),
`bool` and `other`. Up to 1,000 rows of each collection come. The host reads
through `plugin_read` as the plugin, so node grants decide and Activity records
the reads.

Card code has no network, cookies, storage, app API, workers, nested frames,
WebRTC, inline scripts or `eval`. These permissions never carry over to a
source, preview or MCP capability.

Style with `--oik-bg`, `--oik-raised`, `--oik-ink`, `--oik-ink2`, `--oik-ink3`,
`--oik-line`, `--oik-accent`, `--oik-on-accent`, `--oik-font`, `--oik-mono`,
`--oik-display` and `--oik-radius`. `html[data-theme]` is `ink` or `bold`.

## Translations

`translations` maps language tags such as `de` and `zh-Hans` to package `name`,
`description` and `reads`. Its `cards`, `sources` and `mcpClients` objects map
capability ids to translated `name` and, for cards, `reads`. Apps fall back to
the base language, then English. Official packages provide every app language.

Oikome's own packages in `plugins/` are worked examples. RAW Preview and other
complex content capabilities are external official packages distributed on
demand, not bundled into the light core.
