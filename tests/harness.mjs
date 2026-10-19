import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer} from 'node:net';
import {mkdtemp,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash,randomBytes} from 'node:crypto';

export const root = resolve(fileURLToPath(new URL('..',import.meta.url)));
export const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
export async function waitFor(fn, message, timeout=10_000) {
  const deadline = Date.now()+timeout;
  while (Date.now()<deadline) {try {const value=await fn();if(value)return value;}catch{} await delay(75);}
  throw new Error(message);
}
export async function startBridge() {
  const probe=createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));
  const port=probe.address().port;await new Promise(r=>probe.close(r));
  const directory=await mkdtemp(join(tmpdir(),'browser-link-test-'));
  const base=`http://127.0.0.1:${port}`;
  const binary=process.env.BROWSER_LINK_TEST_BINARY || join(root,'target/release',process.platform==='win32'?'browser-link-server.exe':'browser-link-server');
  const process_=spawn(binary,[],{cwd:root,env:{...process.env,BROWSER_LINK_BIND:`127.0.0.1:${port}`,BROWSER_LINK_PUBLIC_URL:base,BROWSER_LINK_DATA_DIR:directory},stdio:['ignore','ignore','pipe']});
  let logs='';process_.stderr.on('data',d=>{logs+=d});
  process_.on('error',e=>{logs+=e.message});
  await waitFor(async()=>{if(process_.exitCode!==null)throw new Error(logs);return(await fetch(base+'/health')).ok;},'Rust bridge failed to start: '+logs);
  const key=(await readFile(join(directory,'admin.key'),'utf8')).trim();
  let id=0;
  async function rpc(token,method,params={}) {
    const response=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json',Accept:'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25'},body:JSON.stringify({jsonrpc:'2.0',id:++id,method,params}),signal:AbortSignal.timeout(35_000)});
    const text=await response.text();
    assert.equal(response.status,200,`MCP ${method}: HTTP ${response.status}: ${text.slice(0,800)}`);
    const envelope=response.headers.get('content-type')?.includes('text/event-stream')?JSON.parse(text.split('\n').find(l=>l.startsWith('data: {')).slice(6)):JSON.parse(text);
    if(envelope.error)throw new Error(JSON.stringify(envelope.error));
    return envelope.result;
  }
  async function authorize() {
    const redirect='http://127.0.0.1:9876/callback';
    const registered=await fetch(base+'/oauth/register',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({redirect_uris:[redirect],token_endpoint_auth_method:'none'})});
    assert.equal(registered.status,201);
    const {client_id}=await registered.json();
    const verifier=randomBytes(32).toString('base64url');
    const challenge=createHash('sha256').update(verifier).digest('base64url');
    const query=new URLSearchParams({client_id,redirect_uri:redirect,response_type:'code',code_challenge:challenge,code_challenge_method:'S256',state:'fixture-state',resource:base+'/mcp',scope:'browser:control'});
    const page=await fetch(base+'/oauth/authorize?'+query);
    assert.equal(page.status,200);
    const html=await page.text();
    const nonce=html.match(/name="nonce" value="([^"]+)"/)[1];
    const bad=await fetch(base+'/oauth/approve',{method:'POST',body:new URLSearchParams({nonce,key:'incorrect-key'}),redirect:'manual'});
    assert.equal(bad.status,401);
    const approved=await fetch(base+'/oauth/approve',{method:'POST',body:new URLSearchParams({nonce,key}),redirect:'manual'});
    assert.equal(approved.status,303);
    const callback=new URL(approved.headers.get('location'));
    assert.equal(callback.searchParams.get('state'),'fixture-state');
    const code=callback.searchParams.get('code');
    const form={grant_type:'authorization_code',client_id,code,code_verifier:verifier,redirect_uri:redirect,resource:base+'/mcp'};
    const exchanged=await fetch(base+'/oauth/token',{method:'POST',body:new URLSearchParams(form)});
    assert.equal(exchanged.status,200);
    const tokens=await exchanged.json();
    assert.equal(exchanged.headers.get('cache-control'),'no-store');
    return {...tokens,client_id,form};
  }
  return {base,key,directory,rpc,authorize,stop:async()=>{process_.kill('SIGINT');await Promise.race([new Promise(r=>process_.once('exit',r)),delay(3000)]);if(process_.exitCode===null)process_.kill('SIGKILL');}};
}

export const fixtureHTML=`<!doctype html><html><head><title>Browser Link fixture</title></head><body>
<h1>Contact workspace</h1><p>This is a local test page; no messages leave this form.</p>
<form id="contact"><label for="name">Customer name</label><input id="name" type="text"><label for="note">Notes</label><textarea id="note"></textarea>
<label for="plan">Plan</label><select id="plan"><option value="basic">Basic</option><option value="pro">Pro</option></select>
<label for="password">Password</label><input id="password" type="password" value="private-fixture-secret">
<button type="submit">Save draft</button></form><button id="disabled" disabled>Unavailable</button><button style="display:none">Hidden command</button><output id="result"></output><div style="height:1800px"></div></body></html>`;
