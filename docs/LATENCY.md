# Browser Link latency diagnostics (extension 0.2.2)

The 0.2.2 extension works with the existing 0.2.0 source-built server and compiled Wasm. It adds measurements to successful individual action replies and prevents an unanswered Chrome message from holding the worker queue indefinitely. It does not establish the cause of the previously observed approximately 5.5-second live tool calls.

## Install

Download the repository ZIP, replace the files in the same `extension/` directory Chrome already loaded, and click **Reload** for Browser Link in `chrome://extensions`. Reload the target website as well so the updated content adapter is installed. Enable that tab again if needed. The existing 0.2.0 server requires no VM rebuild for this update. The Rust source, Wasm and tool schemas are unchanged.

## Measure a harmless action

Measure elapsed time around an individual `browser_click` call using a monotonic client timer. First take a current snapshot and choose a reversible local action such as opening a menu; verify the visible result afterward. Do not time sends, purchases or deletions by repeating them.

Successful individual action replies now include a small `timing` object:

| Field | Measurement |
| --- | --- |
| `extension_version` | Adapter version, expected `0.2.2` |
| `extension_queue_ms` | Arrival at the worker until the command starts |
| `extension_prepare_ms` | Command start until page-command dispatch, including validation, permission checks and content readiness |
| `page_roundtrip_ms` | Chrome content-message dispatch through receipt of its reply |
| `page_ready_ms` | Content-adapter readiness wait after message arrival |
| `page_execution_ms` | Rust/Wasm operation and JSON-result parsing in the page, including synchronous page event handlers |
| `extension_work_ms` | Command execution in the worker, excluding queue wait |
| `extension_total_ms` | Worker arrival through completion, including queue wait |

These durations overlap; do not add every field together. For example, `page_execution_ms` is already part of `page_roundtrip_ms`, which is part of `extension_work_ms`. Browser durations use one browser's monotonic clock and do not depend on synchronization with the VM.

If the externally measured tool call is about 5500 ms but `extension_total_ms` is small, the remaining delay is outside extension execution: MCP/client routing, authorization, server processing or queues, network transport, and result delivery remain to be distinguished. A large `page_execution_ms` points to the page operation or its event handlers. A large `extension_queue_ms` points to earlier browser work. A large `page_roundtrip_ms` with a small `page_execution_ms` points to browser message scheduling, readiness or reply delivery. These are diagnostic leads, not proof of an exact bottleneck.

The external timer starts after the model has chosen its tool call; model thinking between calls is not measured by that timer. Network ping alone is not an upper bound for the whole command.

Snapshot and tab results keep their existing compact schemas. The worker retains only its last timing summary in memory and includes it in the extension's internal status response; no page contents, URLs, tab IDs or credentials are added to that summary. A batch form reply remains `{ok:true,filled:N}` because the server consumes each individual edit's reply. Time an individual action to inspect its timing fields.

## Deadline and recovery

Commands expire at the earlier of the server deadline and 20 seconds after worker arrival. The queue advances when a command times out, even if the underlying Chrome promise never resolves. Checks after asynchronous setup and immediately before page execution prevent expired work from being dispatched later. **Stop** and an unexpected connection close also cancel the worker's wait.

Already-started page actions cannot be undone by a timeout. An uncertain command blocks further actions on that tab until a fresh snapshot succeeds. The extension never retries an action automatically. Browser event-loop stalls can delay timers; the deadline is a recovery mechanism, not a guaranteed real-time SLA.

## Verification

Run `npm run test:latency` for queue, expiration, readiness, Stop/Resume and timing regressions. Run `npm test` for real Rust MCP/OAuth/WebSocket/compiled-Wasm integration with synthetic Chrome APIs. The local fixture click timing is recorded in `VERIFICATION.json`; it excludes ChatGPT routing, model inference, the public VM and native Chrome. Native Chrome remains unverified in this workspace because it blocks AF_UNIX sockets.
