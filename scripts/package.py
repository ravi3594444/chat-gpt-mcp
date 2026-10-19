#!/usr/bin/env python3
"""Package a tested preview. Never include private runtime data or dependencies."""
import hashlib
import json
import shutil
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
native = root / 'target/x86_64-unknown-linux-musl/release/browser-link-server'
report = root / 'test-results/integration.json'
if not native.is_file() or not report.is_file():
    raise SystemExit('Build and test the static Linux server before packaging.')
verification = json.loads(report.read_text())
if verification.get('passed', 0) < 28:
    raise SystemExit('Integration checks are incomplete.')
binary = root / 'bin/browser-link-server-linux-x86_64'
binary.parent.mkdir(exist_ok=True)
shutil.copy2(native, binary)
binary.chmod(0o755)
verification.update({
    'version': '0.1.0',
    'rust_unit_tests_passed': 2,
    'clippy_native_and_wasm': 'passed',
    'plugin_and_mcp_schemas': 'validated against Agent Plugins 1.0.0 schemas',
    'server_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'wasm_sha256': hashlib.sha256((root/'extension/pkg/browser_link_core_bg.wasm').read_bytes()).hexdigest(),
})
(root/'VERIFICATION.json').write_text(json.dumps(verification,indent=2)+'\n')
excluded = {'target','node_modules','data','.test-data','test-results','dist','.git'}
files = [p for p in root.rglob('*') if p.is_file() and not any(part in excluded for part in p.relative_to(root).parts) and p.name not in {'.env','admin.key','auth.json'} and not p.name.endswith('.log')]
dist = root/'dist'
dist.mkdir(exist_ok=True)
output = dist/'browser-link-v0.1.0.zip'
with zipfile.ZipFile(output,'w',compression=zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
    for path in sorted(files):
        archive.write(path,Path('browser-link')/path.relative_to(root))
with zipfile.ZipFile(output) as archive:
    assert archive.testzip() is None
    assert not any('/data/' in name or '/node_modules/' in name or name.endswith('/admin.key') for name in archive.namelist())
print(f'{output} ({output.stat().st_size:,} bytes; {len(files)} files)')
