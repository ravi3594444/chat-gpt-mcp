# Browser Link · personal preview

Connect selected Chrome tabs to ChatGPT through a private MCP connection. The server is Rust. Page inspection, element references, input handling, and command validation are Rust compiled to WebAssembly. A small JavaScript adapter connects Chrome's extension APIs to that core.

**Status:** the Rust server and Wasm core build successfully. Automated integration tests exercise the real MCP, OAuth, WebSocket, and Wasm components with DOM/Chrome API fixtures. Native Chrome verification is still required: the build workspace blocks the Unix sockets Chrome needs. This is a personal preview, not a verified production release.

## What is included

| Folder | Purpose |
| --- | --- |
| `extension/` | Loadable Chrome extension, including prebuilt Wasm |
| `server/` | Rust MCP server, OAuth authorization, pairing and WebSocket bridge |
| `core/` | Rust/Wasm validation, snapshots, clicks, fills and selections |
| `plugin/` | Portable Agent Plugins manifest and browser-control skill |
| `bin/` | Static Linux x86_64 server in the packaged release |
| `tests/` | Integration suite and separate native Chrome suite |
| `VERIFICATION.json` | Results and limitations of the packaged build |

No separate AI API key is needed by this bridge; it makes no model API calls. ChatGPT uses the exposed tools within the user's account permissions and plan limits. Browser Link does not run autonomously after the ChatGPT task stops.

## 1. Run the bridge

For a local extension test on Linux x86_64, extract the package, open a terminal in it, and run:

```bash
chmod +x bin/browser-link-server-linux-x86_64
./bin/browser-link-server-linux-x86_64
```

The server binds to `127.0.0.1:7319`. It creates a private setup key at `data/admin.key`. To display that key in your own terminal:

```bash
./bin/browser-link-server-linux-x86_64 --show-key
```

For Windows/macOS, use Docker Desktop or build the Rust server on that machine. Docker also works on Linux:

```bash
docker compose up --build -d
docker compose exec bridge browser-link-server --show-key
```

Keep the setup key local. Enter it in the extension and the bridge's own OAuth page, never in ChatGPT messages. The extension stores a separate device token after pairing and discards the setup key.

## 2. Load the extension

1. Open `chrome://extensions` and enable **Developer mode**.
2. Select **Load unpacked**, then select this package's `extension` folder.
3. Pin **Browser Link** to the toolbar and open it.
4. Enter the bridge origin (`http://127.0.0.1:7319` for a local test) and its setup key.
5. Select **Pair browser**. Chrome requests access only to that bridge address.
6. Open the website you want to use. Click the toolbar extension and select **Enable this tab**.

Pairing grants no website access. Each enabled tab remains limited to its current origin. Navigating to another site or closing the tab revokes access. **Stop all access** disconnects and clears every enabled tab. **Unpair this browser** also revokes its device token at the reachable bridge.

## 3. Make the bridge reachable from ChatGPT

ChatGPT on the web cannot connect to the `127.0.0.1` address on your computer. For use here, run the bridge behind HTTPS on a server, or forward its entire origin through an HTTPS tunnel. Forward all routes, not just `/mcp`: OAuth discovery, sign-in, token exchange, pairing and `/bridge` WebSocket also need to work.

Set `BROWSER_LINK_PUBLIC_URL` to the real external origin **before starting the bridge**. For Docker, create a local `.env` file containing your actual address, for example:

```dotenv
BROWSER_LINK_PUBLIC_URL=https://browser.example.com
```

`browser.example.com` is an example; replace it with a domain you control. The Compose file exposes the bridge only on localhost. A reverse proxy on the same machine can provide TLS; see `docs/Caddyfile.example`. The server only accepts HTTP public URLs for loopback development. Keep the public URL stable so OAuth discovery and persisted connections remain consistent.

In the extension, pair with the real HTTPS origin. Then, in ChatGPT:

1. Open **Plugins → + → Add custom MCP server**.
2. Name it **Browser Link** and enter `https://YOUR-ACTUAL-HOST/mcp`.
3. Choose OAuth/automatic OAuth discovery and create the personal plugin.
4. In the bridge's authorization page, enter the setup key and approve the connection.
5. Start a new chat with Browser Link enabled. Ask it to inspect the tab you enabled.

Account/workspace policies still apply. Personal custom MCP setup does not submit the plugin to a public directory. No public listing has been created by this project.

The included personal skill guides tool use, but cannot establish the MCP connection itself. For a portable plugin package, run:

```bash
python3 scripts/configure-plugin.py https://YOUR-ACTUAL-HOST/mcp
```

This creates `plugin/mcp.json` with the real endpoint. `plugin/mcp.example.json` is documentation only. The plugin folder can then be used with a supported local/repo marketplace. Registering the custom MCP endpoint in ChatGPT remains a separate step.

## Available tools

`browser_status`, `browser_tabs`, `browser_snapshot`, `browser_click`, `browser_fill`, `browser_select`, `browser_scroll`, `browser_navigate`, `browser_screenshot`.

Snapshots return text, current field values except password/file inputs, and up to 250 interactive element references. A new snapshot invalidates old references. Removed/hidden elements and changed element identities are rejected. Clicks may have real consequences; the agent must follow the user's authorized task and verify outcomes. Timed-out actions are not retried automatically.

This version operates the top-level document. It does not automate password entry, file uploads, browser settings, arbitrary JavaScript, cross-origin frames, closed shadow roots, or canvas controls. Screenshots use the active enabled tab's visible viewport and can contain private page content. The extension has no cookie-export or browsing-history API. There is one paired, connected browser per bridge.

## Build from source

Install Rust and the pinned toolchain, then:

```bash
rustup target add wasm32-unknown-unknown --toolchain 1.99.1
cargo install wasm-bindgen-cli --version 0.2.129 --locked
python3 scripts/build.py
```

This builds the server for your operating system and refreshes the loadable extension. The release already contains the extension's Wasm assets, so loading it does not require Rust on the browser machine.

## Verify

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p browser-link-core --target wasm32-unknown-unknown -- -D warnings
npm ci
npm test
```

For the packaged static Linux binary, set `BROWSER_LINK_TEST_BINARY` to its absolute path before `npm test`; otherwise the suite uses `target/release/browser-link-server`.

On a normal computer, run the real Chrome suite:

```bash
npx playwright install chromium
npm run test:chrome
```

If using an existing Chrome for Testing/Chromium binary, set `CHROME_PATH` to its executable path. The test copy of the manifest pre-grants only its local fixture origins; the distributed manifest is unchanged. Actual toolbar permission prompting and viewport capture require this short manual check:

1. Load the original extension and pair it. Confirm that no tabs appear before enabling one.
2. Open the toolbar extension on a harmless form, enable the tab, then ask ChatGPT to fill it without submitting.
3. Ask for a screenshot; confirm it captures the enabled tab. Check the filled values yourself.
4. Navigate to another site; verify that access is removed until you enable that site.
5. Select **Stop all access**; verify that further actions fail. Reconnect and confirm previous tabs stay disabled.

Back up the private `data` directory or Docker volume if you want pairings and OAuth sessions to survive moving the bridge. Access tokens expire after one hour; rotating refresh tokens expire after 30 days. No page snapshots or screenshots are persisted by the server.

## References

- [OpenAI: connect and test a custom plugin](https://developers.openai.com/plugins/deploy/connect-chatgpt)
- [OpenAI: package a plugin](https://developers.openai.com/plugins/build/plugins)
- [Chrome: load an unpacked extension](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

