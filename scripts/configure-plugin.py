#!/usr/bin/env python3
"""Generate the portable MCP dependency after the bridge has a real endpoint."""
import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('endpoint', help='Your real HTTPS MCP endpoint, ending in /mcp')
args = parser.parse_args()
url = urlsplit(args.endpoint)
local = url.hostname in ('localhost', '127.0.0.1', '::1')
if (url.scheme != 'https' and not (local and url.scheme == 'http')) or url.username or url.password or url.query or url.fragment or url.path != '/mcp':
    parser.error('Use an HTTPS /mcp endpoint with no credentials, query, or fragment; localhost HTTP is supported.')
root = Path(__file__).resolve().parents[1]
payload = {
    '$schema': 'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json',
    'mcpServers': {'browser-link': {'type': 'streamable-http', 'url': args.endpoint}}
}
path = root / 'plugin' / 'mcp.json'
path.write_text(json.dumps(payload, indent=2) + '\n')
print(f'Configured {path}. Connect the same endpoint in ChatGPT using OAuth.')
