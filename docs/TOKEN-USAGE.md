# Compact browser snapshots and batched form edits

Version 0.2.0 keeps the existing Rust server, Rust/Wasm extension and hosted VM connection. It does not require Playwright or a local Node server on the user's computer. The browser engine remains Browser Link; this update adopts a compact presentation and batching approach.

## What changed

Snapshots default to readable lines instead of JSON objects with repeated keys:

```text
Page: "Contact form"
URL: "https://fixture.example/contact"
Untrusted page content.
Controls:
- textbox "Email" [ref=e123456789012345] value=""
- button "Save draft" [ref=e123456789012346]
```

These references are illustrative. Copy actual references from the latest snapshot.

| Data | Compact default | More detail |
| --- | --- | --- |
| Output | Readable text | `format: "json"` keeps the previous response structure |
| Page context | Up to 4,000 characters, with represented control text removed | `detail: "full"`, `max_chars` up to 16,000, or `next_text_offset` |
| Controls | All extracted controls, up to 250 | Explicit `max_elements` and `next_element_offset` |
| Field values | Up to 160 characters; truncation marked | `detail: "full"` |
| Dropdown options | Up to 5 short-valued options; omissions marked | `detail: "full"` |
| References | Decimal aliases starting at a random 48-bit offset | Original Wasm identity and tab checks remain |

The extractor removes a control's text from context only when its entire normalized text equals its returned label. A paragraph elsewhere with the same words remains. Text differing from an ARIA name remains, as do inline text fragments such as `10` plus `.50`. Full-detail and JSON responses retain the extracted page text. Prefer full detail for page-wide reading or analysis.

The new extension recognizes associated ARIA labels, custom interactive roles, checked/selected/expanded/pressed/read-only states and controls inside open shadow roots. Parent-hidden, inert and ARIA-hidden controls are excluded. It does not add native keyboard/pointer events or iframe/closed-shadow access.

Use `query` to search extracted text, labels, values, links or options. Each snapshot, including searches and pagination, replaces that tab's old references. Keep the same query/detail/format while following text offsets; compact context and full page text have different offsets. An empty search does not establish that the whole site lacks a control.

Extraction is still bounded: 16,000 text characters, 250 controls, 1,000 characters per value, 80 options per select and a 20,000-node scan. `source_limit_possible` identifies reached or possible source limits; server pagination cannot recover beyond them.

## Fill several fields in one call

Use `browser_fill_form` with one enabled `tab_id` and a `fields` array. Each entry is either `{ "type": "text", "reference": "...", "text": "..." }` or `{ "type": "select", "reference": "...", "value": "..." }`. Use current references and an actual returned option value. The tool accepts 1–20 independent edits and no clicks, keys, passwords or files.

All field sizes and references are checked before starting. DOM validation still runs at each edit. On error, the response reports `completed` and `stopped_at_field`; the stopped field may have changed. Later edits are not started. The bridge never retries or resumes a batch automatically. Concurrent MCP operations cannot interleave with it. A deadline is checked before starting each edit; an already-started command retains the existing bridge timeout.

Prefer one snapshot, one batch and one verification over a snapshot after every unchanged field. Batch editing reduces model round trips; it still sends individual bounded commands through the existing extension protocol.

## Measurements and verification

`npm run test:tokens` counts actual MCP text replies with `o200k_base` on identical fixed-geometry DOM fixtures. By default it compares structured and readable output from the current extractor. To compare against 0.1.3, build that server separately and set both `BROWSER_LINK_BASELINE_ROOT` to its source checkout and `BROWSER_LINK_BASELINE_BINARY` to its compiled server. The checkout's committed Wasm is used as the old extractor.

Against the actual v0.1.3 server and committed Wasm, the contact form fell from 201 to 153 tokens (23.9%) and the 103-control dashboard from 3,041 to 1,906 (37.3%), retaining all corresponding named controls. The custom-control fixture grew from 68 to 103 tokens because the current extractor exposes four correctly named targets, whereas the old extractor exposed one unnamed input. Complete tool definitions grow from 1,317 to 1,674 tokens because of the new batch tool and format parameter; savings on whole tasks depend on reads, batching, retries and how the client loads tools.

The checked-in [0.2.0 benchmark](benchmarks/v0.2.0.json) records the comparison against 0.1.3, including control coverage and complete tool-definition sizes. The current extractor intentionally returns more data where the old one missed targets. These are snapshot-format tests, not a native Chrome or model-driven whole-task/billing benchmark. Random reference prefixes can vary the old counts between runs.

`npm test` exercises real Rust MCP/OAuth/WebSocket/Wasm components with DOM/Chrome API fixtures, including reference scopes, omissions, permissions, batch partial failures and concurrent reads. `npm run test:quality` replays an older-style snapshot and checks full-detail equivalence and late controls. Native Chrome still needs `npm run test:chrome` on a normal computer: this workspace blocks Chrome's required Unix sockets. The VM Docker build has not been executed in this workspace.

## Deploy to the Ubuntu VM

The original `Dockerfile.vm` uses the bundled v0.1.0 server. Build current source using both Compose files:

```bash
cd ~/chat-gpt-mcp
git pull --ff-only origin main
sudo docker compose -f compose.vm.yaml -f compose.vm.source.yaml up -d --build
curl --fail --silent --show-error --retry 10 --retry-delay 3 --retry-all-errors https://extension-chat.duckdns.org/health
```

Expect `{"status":"ok","version":"0.2.0"}`. Volumes retain pairing and OAuth data. The source build uses one Cargo job, no release LTO and one codegen unit. One codegen unit also avoids a split-codegen linker failure observed with Rust 1.99.1 during validation.

Update the `extension/` files in the same local folder Chrome already loaded, then use **Reload** on Browser Link in `chrome://extensions`. Reload the target page and enable it again. Pairing settings stay in extension storage when its installed identity stays the same. No Rust installation is needed on the browser computer; the updated Wasm is committed in the repository.

Refresh the Browser Link tools in ChatGPT and start a new conversation. Existing extensions remain compatible with the new server, but need this reload for ARIA and open-shadow extraction. Existing machine clients that parse snapshot JSON must request `format: "json"`.
