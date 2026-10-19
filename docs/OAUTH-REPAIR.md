# Repair Browser Link OAuth approval

The packaged server is version 0.1.0. Its approval page uses a privacy policy that hides the form's origin and a form policy that can block the redirect back to ChatGPT. Resubmitting an already-approved form then receives 400.

The Docker VM setup now applies both corrections in Caddy, using the existing server binary. The compatibility rule only changes the old strict form policy; it preserves newer policies containing a registered callback origin.

On the VM:

```bash
cd ~/chat-gpt-mcp
git pull --ff-only
sudo docker compose -f compose.vm.yaml run --rm --no-deps caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sudo docker compose -f compose.vm.yaml up -d --no-deps caddy
```

Close the previous approval tab and start a new Browser Link connection from ChatGPT. No Rust build or setup-key change is needed. Pairings and OAuth credentials remain in their existing named volume.

The source server is version 0.1.1 and applies these policies itself. It also returns readable approval error codes. Its compiled build passes 32 integration checks; real Chrome execution remains unverified in this build environment. The bundled binary is still version 0.1.0, so use the included Caddy configuration when deploying it.
