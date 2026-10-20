# Smaller browser reads

Browser Link makes no model API calls. ChatGPT calls a tool, the bridge asks the
extension to inspect an enabled tab, and the resulting text is sent back to
ChatGPT. That tool output adds context. Implementing the bridge in Rust does not
by itself reduce the text that ChatGPT reads.

The old snapshot forwarded up to 16,000 text characters, 250 controls, 1,000
characters per field value, and 80 options per dropdown on every read. It also
repeated a long document nonce in every element reference.

Server 0.1.3 changes the default presentation:

| Data | Compact default | More detail |
| --- | --- | --- |
| Page text | 4,000 characters | `next_text_offset` or `max_chars` up to 16,000 |
| Controls | All extracted controls, up to 250 | Set `max_elements` explicitly to page with `next_element_offset` |
| Field values | 160 characters, with `value_truncated` when shortened | `detail: "full"` |
| Dropdown options | Up to 5 with short values; omissions marked | `detail: "full"` |
| References | Short aliases scoped to one tab and snapshot | Same underlying Wasm identity checks |

The initial 40-control default was too aggressive: a real conversation page put
the message composer, attachments, and model selector after that cutoff. Version
0.1.3 removes the extra control count and byte cutoffs, preserving all controls
already extracted by the extension. Text, long values, and options still use the
compact limits above; their omissions must be recovered when needed for a task.
Use full detail for reading or analyzing an entire page.

Search page text and control labels, values, links, and dropdown options with
`query`. Text matches include neighboring lines; text offsets then refer to this
matching text. Use `max_chars: 0` for controls-only work. For example:

```json
{"tab_id": 123, "query": "Plan", "detail": "full", "max_chars": 0}
```

Use a real enabled tab ID from `browser_tabs`. A query searches all data extracted
by the extension, including controls outside the default first page. Full detail
works with a query and pagination, so a long dropdown does not require a whole
page dump. `text_total` and `elements_total` describe the matching extracted data.
An empty search result does not prove the entire website lacks that content.

**Every snapshot, including a search or page of results, replaces that tab's
references.** Use only references from its latest result. Another tab's references
remain independent. The bridge expands aliases before sending actions to the
unchanged extension, where stale, changed, hidden, disabled, and unauthorized
elements are still rejected. Reference caches contain identifiers, not page text,
and retain at most 64 tabs.

The existing extension's extraction caps still apply: 16,000 text characters,
250 controls, 1,000 characters per field, and 80 options per dropdown. When a cap
may have been reached, `source_limit_possible` identifies it. Server pagination
cannot retrieve beyond these source caps. Dynamic pages can change between reads;
offsets are not a frozen copy of the page.

Check connection state and list tabs on first use or after reconnecting. Reuse
references while the page remains unchanged; filling several existing fields
does not require a snapshot after each field. Inspect again after navigation,
meaningful page changes, and consequential actions. Screenshots remain available
when text is insufficient. These changes reduce tool output; they do not establish
a fixed reduction in total ChatGPT usage or billing.

## Update the Ubuntu VM

`Dockerfile.vm` still contains the original v0.1.0 binary. Rebuilding that file does
not install these changes. Build current source with the supplied override:

```bash
cd ~/chat-gpt-mcp
git pull --ff-only
sudo docker compose -f compose.vm.yaml -f compose.vm.source.yaml up -d --build
curl --fail --show-error https://extension-chat.duckdns.org/health
```

The health response must report `0.1.3`. The source build uses one Cargo job and
disables release LTO to reduce peak memory on a 2 GB VM. The first build takes
time and requires disk space for the Rust image and build files. Existing Compose
volumes retain pairing and OAuth data. Use both Compose files for future bridge
builds and updates; the original file alone selects the old bundled binary.

The installed Chrome extension remains compatible; no extension reload is needed.
Refresh the Browser Link tool definitions in your ChatGPT connection and start a
new chat so it can see the new snapshot parameters.

To verify locally, run `cargo test --workspace`, build the server, and run
`npm ci && npm test`. Integration tests exercise the real Rust MCP server, original
compiled Wasm, reference translation, search, paging, detail recovery, origin
boundaries, and OAuth. `test-results/snapshot-size.json` compares serialized raw
and compact output for a synthetic dashboard; it measures payload size, not model
tokens. Native Chrome remains a separate check with `npm run test:chrome`.

`npm run test:quality` replays a conversation-style fixture through the
real MCP server and checks full-detail equivalence and late control recovery.
It fails if the default omits a control; `ALLOW_MISSING_CONTROLS=1` explicitly
allows omissions when measuring an older version as a baseline. An optional
snapshot JSON path supports a private live capture; do not commit those captures.
This checks returned data, not a complete model-driven browser workflow. The VM
Docker build and native Chrome smoke check still need their target environments.

Validation for 0.1.3 passed five Rust unit tests, 39 integration checks, native
Clippy, and both synthetic and live-capture replay checks. The live capture
retained all 76 extracted controls and all 3,229 text characters; serialized
output fell from 13,704 to 10,118 characters (26.2%). The dashboard fixture retained
its late composer controls and shrank from 29,538 to 11,623 characters (60.7%).
These are payload measurements, not billed model tokens or a general guarantee
of equal task success. Private live page content is excluded from the repository.
