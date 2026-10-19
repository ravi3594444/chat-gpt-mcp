#!/usr/bin/env python3
"""Build the native server and loadable Rust/Wasm Chrome extension."""
import argparse
import json
import shutil
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--extension-only', action='store_true')
args = parser.parse_args()

def run(*command):
    subprocess.run(command, cwd=root, check=True)

if not shutil.which('cargo') or not shutil.which('wasm-bindgen'):
    raise SystemExit('Install Rust, add wasm32-unknown-unknown, and install wasm-bindgen-cli 0.2.129. See README.md.')
if not args.extension_only:
    run('cargo', 'build', '--release', '--locked', '-p', 'browser-link-server')
run('cargo', 'build', '--release', '--locked', '-p', 'browser-link-core', '--target', 'wasm32-unknown-unknown')
out = root / 'extension' / 'pkg'
out.mkdir(exist_ok=True)
run('wasm-bindgen', str(root / 'target/wasm32-unknown-unknown/release/browser_link_core.wasm'), '--target', 'web', '--out-dir', str(out), '--out-name', 'browser_link_core', '--no-typescript')
assert (out / 'browser_link_core_bg.wasm').stat().st_size > 1000
json.loads((root / 'extension/manifest.json').read_text())
print('Load the extension/ folder using Chrome → Extensions → Developer mode → Load unpacked.')
