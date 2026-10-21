// Real Rust MCP + OAuth + WebSocket + compiled Rust/Wasm, with DOM/Chrome API fixtures.
// This does NOT replace the separate test:chrome suite.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {JSDOM} from 'jsdom';
import WebSocket from 'ws';
import {startBridge,waitFor,delay,root,fixtureHTML} from './harness.mjs';

const checks=[];
const pass=name=>{checks.push(name);console.log('PASS '+name);};
const bridge=await startBridge();
globalThis.WebSocket=WebSocket;
let rust;
const listeners=[];
let tabListeners=[];
let activeId=1;
let latestRawSnapshot;
const tabs=new Map([[1,{id:1,windowId:1,url:'https://fixture.example/contact',title:'Contact fixture'}],[2,{id:2,windowId:1,url:'https://private.example/account',title:'Private tab'}]]);
const dom=new JSDOM(fixtureHTML,{url:tabs.get(1).url,pretendToBeVisual:true});
for(const name of ['Window','Document','Element','Node','HtmlElement','HTMLElement','HTMLInputElement','HTMLTextAreaElement','HTMLSelectElement','ShadowRoot','Event','EventInit'])if(dom.window[name])globalThis[name]=dom.window[name];
globalThis.window=dom.window;globalThis.document=dom.window.document;
dom.window.HTMLElement.prototype.getBoundingClientRect=function(){return {width:200,height:30,top:0,left:0,right:200,bottom:30,x:0,y:0};};
dom.window.HTMLElement.prototype.scrollIntoView=function(){};
// jsdom has no layout engine or innerText implementation; native Chrome is tested separately.
Object.defineProperty(dom.window.HTMLElement.prototype,'innerText',{get(){return this.textContent;}});
dom.window.scrollBy=()=>{};
document.getElementById('contact').addEventListener('submit',e=>{e.preventDefault();document.getElementById('result').textContent='Draft saved for '+document.getElementById('name').value;});
const local={},session={};
const storage=data=>({get:async key=>({[key]:data[key]}),set:async patch=>Object.assign(data,patch),remove:async key=>{delete data[key]}});
globalThis.chrome={
  storage:{local:storage(local),session:storage(session)},
  action:{setBadgeText:async()=>{},setBadgeBackgroundColor:async()=>{}},
  scripting:{executeScript:async()=>[]},
  tabs:{
    get:async id=>{if(!tabs.has(id))throw new Error('Unknown tab');return {...tabs.get(id)};},
    query:async()=>[{...tabs.get(activeId)}],
    update:async(id,patch)=>{Object.assign(tabs.get(id),patch);if(patch.active)activeId=id;if(patch.url){dom.reconfigure({url:patch.url});for(const fn of tabListeners)await fn(id,{url:patch.url});}return {...tabs.get(id)};},
    sendMessage:async(id,message)=>{
      if(message.type==='browser-link-ping')return {ready:true};
      const started=performance.now();
      const result=JSON.parse(rust.operate(JSON.stringify(message.command)));
      if(message.command.action==='snapshot')latestRawSnapshot=result;
      return {result,timing:{page_ready_ms:0,page_execution_ms:performance.now()-started}};
    },
    captureVisibleTab:async()=> 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jYioAAAAASUVORK5CYII=',
    onRemoved:{addListener:()=>{}},onUpdated:{addListener:fn=>tabListeners.push(fn)}
  },
  runtime:{onStartup:{addListener:()=>{}},onMessage:{addListener:fn=>listeners.push(fn)}}
};
async function message(payload){return new Promise((resolve,reject)=>{listeners[0](payload,{},result=>result?.error?reject(new Error(result.error)):resolve(result));});}
let oauth;
async function call(name,args={}){return bridge.rpc(oauth.access_token,'tools/call',{name,arguments:name==='browser_snapshot'?{format:'json',...args}:args});}
function text(result){assert(!result.isError,result.content?.map(c=>c.text).join('\n'));return JSON.parse(result.content.find(c=>c.type==='text').text);}
try {
  const unauthorized=await fetch(bridge.base+'/mcp',{method:'POST',headers:{'Content-Type':'application/json'},body:'{}'});
  assert.equal(unauthorized.status,401);assert.match(unauthorized.headers.get('www-authenticate'),/oauth-protected-resource/);pass('Unauthenticated MCP is blocked with OAuth discovery');
  const origin=await fetch(bridge.base+'/health',{headers:{Origin:'https://attacker.example'}});assert.equal(origin.status,403);pass('Untrusted request origins are rejected');
  const opaque=await fetch(bridge.base+'/oauth/approve',{method:'POST',headers:{Origin:'null'},body:new URLSearchParams({nonce:'invalid',key:'invalid'})});assert.equal(opaque.status,403);pass('Opaque-origin approval submissions remain blocked');
  const badRedirect=await fetch(bridge.base+'/oauth/register',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({redirect_uris:['https://attacker.example/callback']})});assert.equal(badRedirect.status,400);pass('OAuth redirect allowlist rejects unrelated hosts');
  oauth=await bridge.authorize();pass('OAuth authorization, wrong-key rejection, PKCE exchange and state round-trip');
  for(const redirect of ['https://chatgpt.com/connector/oauth/callback','https://chat.openai.com/connector/oauth/callback'])await bridge.authorize(undefined,redirect);
  pass('ChatGPT callback policies and repeated-form error messages are correct');
  const replay=await fetch(bridge.base+'/oauth/token',{method:'POST',body:new URLSearchParams(oauth.form)});assert.equal(replay.status,400);pass('OAuth authorization codes cannot be replayed');
  const info=await bridge.rpc(oauth.access_token,'initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'browser-link-fixture',version:'1'}});assert.equal(info.serverInfo.name,'browser-link');pass('Real MCP protocol initialization');
  const tools=await bridge.rpc(oauth.access_token,'tools/list');assert.equal(tools.tools.length,10);assert.equal(tools.tools.find(t=>t.name==='browser_click').annotations.destructiveHint,true);pass('Ten MCP tools discover with consequential-click annotations');
  assert.equal(text(await call('browser_status')).connected,false);pass('Offline browser state is explicit');
  const pkg=join(root,'extension/pkg/browser_link_core.js');
  globalThis.__browserLinkWasmBytes=new Uint8Array(await readFile(join(root,'extension/pkg/browser_link_core_bg.wasm')));
  rust=await import(pathToFileURL(pkg));await rust.default({module_or_path:globalThis.__browserLinkWasmBytes});
  assert.throws(()=>rust.validate_command(JSON.stringify({action:'navigate',tab_id:1,url:'javascript:alert(1)'})));pass('Actual compiled WebAssembly rejects executable navigation');
  const results=join(root,'test-results');await mkdir(results,{recursive:true});
  let worker=await readFile(join(root,'extension/worker.js'),'utf8');
  worker=worker.replace("'./pkg/browser_link_core.js'",JSON.stringify(pathToFileURL(pkg).href)).replace('const rustReady = init();','const rustReady = init({module_or_path:globalThis.__browserLinkWasmBytes});');
  const workerPath=join(results,'worker-fixture.mjs');await writeFile(workerPath,worker);await import(pathToFileURL(workerPath));
  await assert.rejects(()=>message({type:'pair',server:bridge.base,key:'wrong'}),/incorrect/);pass('Extension pairing rejects an incorrect setup key');
  await message({type:'pair',server:bridge.base,key:bridge.key});await waitFor(async()=>(await message({type:'status'})).state==='connected','Extension fixture did not connect');pass('Rust bridge authenticates the extension over a real WebSocket');
  assert.equal(text(await call('browser_tabs')).tabs.length,0);pass('Pairing alone exposes no tabs');
  await message({type:'enable',tabId:1});
  const listed=text(await call('browser_tabs')).tabs;assert.deepEqual(listed.map(t=>t.tab_id),[1]);pass('Only explicitly enabled tabs appear');
  assert.equal((await call('browser_snapshot',{tab_id:2})).isError,true);pass('Direct access to a non-enabled tab is blocked');
  let snap=text(await call('browser_snapshot',{tab_id:1}));
  assert.match(snap.text,/Contact workspace/);assert(!snap.elements.some(e=>e.label==='Hidden command'));assert(!JSON.stringify(snap).includes('private-fixture-secret'));pass('Rust DOM snapshot reads visible content and excludes password values');
  const name=snap.elements.find(e=>e.label==='Customer name').reference;
  assert.equal((await call('browser_fill',{tab_id:1,reference:name,text:'Alex'})).isError,false);assert.equal(document.getElementById('name').value,'Alex');pass('MCP fill reaches the real Rust/Wasm DOM action');
  const plan=snap.elements.find(e=>e.label==='Plan').reference;
  assert.equal((await call('browser_select',{tab_id:1,reference:plan,value:'pro'})).isError,false);assert.equal(document.getElementById('plan').value,'pro');pass('Native select options work through MCP');
  const password=snap.elements.find(e=>e.type==='password').reference;
  assert.equal((await call('browser_fill',{tab_id:1,reference:password,text:'blocked'})).isError,true);pass('Password input automation is blocked');
  const note=snap.elements.find(e=>e.label==='Notes').reference;
  const filled=await call('browser_fill_form',{tab_id:1,fields:[{type:'text',reference:name,text:'Batch customer'},{type:'text',reference:note,text:'Prepared locally'},{type:'select',reference:plan,value:'basic'}]});
  assert.deepEqual(text(filled),{ok:true,filled:3});
  assert.equal(document.getElementById('name').value,'Batch customer');assert.equal(document.getElementById('note').value,'Prepared locally');assert.equal(document.getElementById('plan').value,'basic');assert.equal(document.getElementById('result').textContent,'');
  pass('One MCP form call edits multiple fields without clicking submit');
  for(const fields of [[],[{type:'text',reference:name,text:'must not apply'},{type:'text',reference:'invalid',text:'x'}],[{type:'text',reference:name,text:'must not apply'},{type:'text',reference:name,text:'duplicate'}],[{type:'text',reference:name,text:'x'.repeat(20001)}]])assert.equal((await call('browser_fill_form',{tab_id:1,fields})).isError,true);
  assert.equal(document.getElementById('name').value,'Batch customer');
  pass('Invalid batch inputs and stale or duplicate refs fail before editing fields');
  const partial=await call('browser_fill_form',{tab_id:1,fields:[{type:'text',reference:name,text:'Partial customer'},{type:'text',reference:password,text:'blocked'},{type:'text',reference:note,text:'must not apply'}]});
  assert.equal(partial.isError,true);const progress=JSON.parse(partial.content[0].text);assert.equal(progress.completed,1);assert.equal(progress.stopped_at_field,2);assert.equal(document.getElementById('name').value,'Partial customer');assert.equal(document.getElementById('note').value,'Prepared locally');
  pass('A failed batch reports partial completion and never executes later fields');
  await call('browser_fill',{tab_id:1,reference:name,text:'Alex'});
  const save=snap.elements.find(e=>e.label==='Save draft').reference;
  document.querySelector('button[type="submit"]').textContent='Delete everything';
  assert.equal((await call('browser_click',{tab_id:1,reference:save})).isError,true);
  document.querySelector('button[type="submit"]').textContent='Save draft';pass('Changed element meaning requires a fresh inspection before a click');
  const clickStarted=performance.now();
  const clickResult=await call('browser_click',{tab_id:1,reference:save});
  const clickRoundtrip=performance.now()-clickStarted;
  assert.equal(clickResult.isError,false);assert.equal(document.getElementById('result').textContent,'Draft saved for Alex');pass('A click submits only the local fixture and its outcome is observable');
  const clickTiming=text(clickResult).timing;
  assert.equal(clickTiming.extension_version,'0.2.2');
  for(const key of ['extension_queue_ms','extension_prepare_ms','page_roundtrip_ms','page_execution_ms','extension_work_ms','extension_total_ms'])assert(Number.isFinite(clickTiming[key])&&clickTiming[key]>=0,key);
  assert(clickTiming.extension_total_ms<=clickRoundtrip+1);
  await writeFile(join(results,'click-latency.json'),JSON.stringify({scope:'Local Rust MCP + OAuth + WebSocket + Wasm, with jsdom/Chrome API fixtures. No ChatGPT routing, model inference, public VM or native Chrome.',tool_roundtrip_ms:clickRoundtrip,timing:clickTiming},null,2)+'\n');
  pass('Click stage timings pass through the unchanged Rust server without browser data or credentials');
  await call('browser_snapshot',{tab_id:1});assert.equal((await call('browser_click',{tab_id:1,reference:save})).isError,true);pass('A new snapshot invalidates previous element references');
  const focused=text(await call('browser_snapshot',{tab_id:1,query:'CUSTOMER NAME',max_chars:0}));
  assert.equal(focused.elements.length,1);assert.equal(focused.elements[0].label,'Customer name');assert.equal(focused.text,'');
  assert(focused.elements[0].reference.length < 30);assert(!('disabled' in focused.elements[0]));
  pass('Compact search returns short references and supports controls-only reads');
  for(const args of [{max_chars:16001},{max_elements:0},{query:'x'.repeat(201)},{element_offset:251}])assert.equal((await call('browser_snapshot',{tab_id:1,...args})).isError,true);
  assert.equal((await call('browser_fill',{tab_id:1,reference:focused.elements[0].reference,text:'Alex'})).isError,false);
  assert.equal((await call('browser_fill',{tab_id:2,reference:focused.elements[0].reference,text:'blocked'})).isError,true);
  pass('Invalid snapshot limits preserve references; references cannot cross tabs');
  const bulk=document.createElement('section');
  bulk.innerHTML='<p>'+('Local dashboard activity. '.repeat(800))+'</p>'+Array.from({length:100},(_,i)=>`<button>Action ${String(i).padStart(3,'0')}</button>`).join('')+'<textarea aria-label="Message composer"></textarea><button>Add files and more</button><button>Select ChatGPT model</button>';
  document.body.append(bulk);
  const compact=text(await call('browser_snapshot',{tab_id:1}));
  assert.equal(compact.text.length,4000);assert.equal(compact.elements.length,latestRawSnapshot.elements.length);
  assert.equal(compact.next_text_offset,4000);assert(!('next_element_offset' in compact));
  for(const label of ['Message composer','Add files and more','Select ChatGPT model'])assert(compact.elements.some(e=>e.label===label));
  assert(compact.source_limit_possible.includes('text'));
  const before=JSON.stringify(latestRawSnapshot),after=JSON.stringify(compact);
  const benchmark={fixture:'100 dashboard actions and more than 16000 characters of activity text',raw_chars:before.length,compact_chars:after.length,raw_bytes:Buffer.byteLength(before),compact_bytes:Buffer.byteLength(after),character_reduction_percent:Number((100*(1-after.length/before.length)).toFixed(1)),note:'Serialized tool-result text, not total ChatGPT tokens or billing.'};
  assert(after.length < before.length*0.6);
  await writeFile(join(results,'snapshot-size.json'),JSON.stringify(benchmark,null,2)+'\n');
  pass('Default snapshot retains late composer controls while reducing fixture output by over 40 percent');
  const paged=text(await call('browser_snapshot',{tab_id:1,max_chars:0,max_elements:40}));
  assert.equal(paged.elements.length,40);assert.equal(paged.next_element_offset,40);
  const next=text(await call('browser_snapshot',{tab_id:1,text_offset:compact.next_text_offset,element_offset:paged.next_element_offset,max_elements:40}));
  assert.equal(next.elements[0].label,latestRawSnapshot.elements[40].label);
  assert.equal(next.text,latestRawSnapshot.text.slice(4000,8000));
  assert.equal((await call('browser_click',{tab_id:1,reference:compact.elements[0].reference})).isError,true);
  pass('Pagination retrieves omitted content and invalidates earlier page references');
  const last=text(await call('browser_snapshot',{tab_id:1,query:'Action 099',max_chars:0}));
  assert.equal(last.elements.length,1);assert.equal(last.elements[0].label,'Action 099');
  pass('Focused search finds controls outside the default first page');
  const nativePlan=document.getElementById('plan');
  for(let i=0;i<20;i++){const option=document.createElement('option');option.value=`extra-${i}`;option.textContent=`Extra plan ${i}`;nativePlan.append(option);}
  const smallOptions=text(await call('browser_snapshot',{tab_id:1,query:'Plan',max_chars:0})).elements.find(e=>e.label==='Plan');
  assert.equal(smallOptions.options.length,5);assert(smallOptions.options_truncated);
  const fullOptions=text(await call('browser_snapshot',{tab_id:1,query:'Plan',detail:'full',max_chars:0})).elements.find(e=>e.label==='Plan');
  assert.equal(fullOptions.options.length,22);
  assert.equal((await call('browser_select',{tab_id:1,reference:fullOptions.reference,value:'extra-19'})).isError,false);
  pass('Full focused detail recovers omitted options and selects through the original Wasm reference');
  let notes=text(await call('browser_snapshot',{tab_id:1,query:'Notes',max_chars:0})).elements[0];
  const longNote='Long local note. '.repeat(40);
  assert.equal((await call('browser_fill',{tab_id:1,reference:notes.reference,text:longNote})).isError,false);
  notes=text(await call('browser_snapshot',{tab_id:1,query:'Notes',max_chars:0})).elements[0];
  assert.equal(notes.value.length,160);assert(notes.value_truncated);
  notes=text(await call('browser_snapshot',{tab_id:1,query:'Notes',detail:'full',max_chars:0})).elements[0];
  assert.equal(notes.value,longNote);
  bulk.remove();pass('Long values are marked in compact output and recovered with full detail');
  const advanced=document.createElement('section');
  advanced.innerHTML='<span id="aria-email-label">Contact email</span><input id="aria-email" aria-labelledby="aria-email-label"><div role="checkbox" aria-checked="true">Subscribe to updates</div><div role="switch" aria-label="Notifications" aria-checked="false"></div><div id="shadow-host"></div><div style="display:none"><button>Parent-hidden action</button></div><p>Save draft</p><p>Total <span>10</span><span>.50</span></p><button aria-label="Approve record">Accept record</button>';
  document.body.append(advanced);
  const shadow=document.getElementById('shadow-host').attachShadow({mode:'open'});shadow.innerHTML='<button id="shadow-save">Shadow save</button><label for="shadow-name">Shadow name</label><input id="shadow-name">';
  let shadowClicked=false;shadow.getElementById('shadow-save').onclick=()=>{shadowClicked=true;};
  const advancedSnapshot=text(await call('browser_snapshot',{tab_id:1}));
  for(const label of ['Contact email','Subscribe to updates','Notifications','Shadow save','Shadow name'])assert(advancedSnapshot.elements.some(e=>e.label===label),label);
  assert.equal(advancedSnapshot.elements.find(e=>e.label==='Subscribe to updates').checked,true);assert.equal(advancedSnapshot.elements.find(e=>e.label==='Notifications').checked,false);
  assert(!advancedSnapshot.elements.some(e=>e.label==='Parent-hidden action'));
  await call('browser_click',{tab_id:1,reference:advancedSnapshot.elements.find(e=>e.label==='Shadow save').reference});assert(shadowClicked);
  await call('browser_fill',{tab_id:1,reference:advancedSnapshot.elements.find(e=>e.label==='Shadow name').reference,text:'In component'});assert.equal(shadow.getElementById('shadow-name').value,'In component');
  pass('ARIA names, custom toggle states and open shadow-root targets are discovered and actionable');
  const readable=await bridge.rpc(oauth.access_token,'tools/call',{name:'browser_snapshot',arguments:{tab_id:1}});
  assert(!readable.isError);const readableText=readable.content[0].text;assert(readableText.startsWith('Page:'));assert(readableText.includes('checkbox "Subscribe to updates"'));assert(readableText.includes('checked=true'));assert(readableText.includes('switch "Notifications"'));assert(readableText.includes('checked=false'));
  assert(readableText.includes('\nSave draft\n'),'Unrelated text matching a control must remain');assert(readableText.includes('Accept record'),'Visible text differing from its ARIA name must remain');
  assert(readableText.includes('Total 10.50'),'Inline text fragments must retain punctuation and spacing');
  pass('Default readable output retains action refs, states and unrelated or differently labelled visible text');
  const changedName=text(await call('browser_snapshot',{tab_id:1,query:'Contact email'})).elements.find(e=>e.label==='Contact email').reference;
  document.getElementById('aria-email-label').textContent='Different destination';assert.equal((await call('browser_fill',{tab_id:1,reference:changedName,text:'blocked'})).isError,true);
  const formSnapshot=text(await call('browser_snapshot',{tab_id:1}));
  const batchRefs=['Customer name','Notes'].map(label=>formSnapshot.elements.find(e=>e.label===label).reference);
  const originalSend=chrome.tabs.sendMessage;
  let signalStarted,resumeFirst,pauseFirst=true;
  const firstStarted=new Promise(resolve=>{signalStarted=resolve;});const firstReleased=new Promise(resolve=>{resumeFirst=resolve;});
  chrome.tabs.sendMessage=async(id,payload)=>{if(pauseFirst&&payload.command?.action==='fill'){pauseFirst=false;signalStarted();await firstReleased;}return originalSend(id,payload);};
  try{
    const pendingBatch=call('browser_fill_form',{tab_id:1,fields:[{type:'text',reference:batchRefs[0],text:'Concurrent customer'},{type:'text',reference:batchRefs[1],text:'Both edits retained'}]});
    await firstStarted;const pendingRead=call('browser_snapshot',{tab_id:1});await delay(20);resumeFirst();
    assert.equal((await pendingBatch).isError,false);const concurrentRead=text(await pendingRead);
    assert.equal(concurrentRead.elements.find(e=>e.label==='Customer name').value,'Concurrent customer');assert.equal(concurrentRead.elements.find(e=>e.label==='Notes').value,'Both edits retained');
  }finally{resumeFirst();chrome.tabs.sendMessage=originalSend;}
  pass('A concurrent snapshot cannot interleave with a batch or invalidate its remaining refs');
  advanced.remove();
  pass('Changing an associated accessible name invalidates its action reference');
  assert.equal((await call('browser_scroll',{tab_id:1,pixels:500})).isError,false);assert.equal((await call('browser_scroll',{tab_id:1,pixels:6000})).isError,true);pass('Scroll actions enforce their bounds');
  assert.equal((await call('browser_navigate',{tab_id:1,url:'https://other.example/'})).isError,true);pass('Cross-origin navigation requires local user action');
  const shot=await call('browser_screenshot',{tab_id:1});assert(shot.content.some(c=>c.type==='image'&&c.mimeType==='image/png'));pass('MCP returns screenshot content as an image block (fixture image)');
  await chrome.tabs.update(1,{url:'https://other.example/'});assert.equal(text(await call('browser_tabs')).tabs.length,0);pass('Manual navigation to another origin revokes tab access');
  await message({type:'stop'});await waitFor(async()=>text(await call('browser_status')).connected===false,'Stop did not disconnect');assert.equal((await message({type:'status'})).tabs.length,0);pass('Stop immediately disconnects and clears enabled tabs');
  await message({type:'resume'});await waitFor(async()=>(await message({type:'status'})).state==='connected','Reconnect failed');assert.equal(text(await call('browser_tabs')).tabs.length,0);pass('Reconnect works without re-enabling previously stopped tabs');
  const refreshed=await fetch(bridge.base+'/oauth/token',{method:'POST',body:new URLSearchParams({grant_type:'refresh_token',client_id:oauth.client_id,refresh_token:oauth.refresh_token})});assert.equal(refreshed.status,200);
  const rotated=await refreshed.json();
  const old=await fetch(bridge.base+'/oauth/token',{method:'POST',body:new URLSearchParams({grant_type:'refresh_token',client_id:oauth.client_id,refresh_token:oauth.refresh_token})});assert.equal(old.status,400);pass('OAuth refresh tokens rotate and old refresh tokens are rejected');
  const revoked=await fetch(bridge.base+'/oauth/revoke',{method:'POST',body:new URLSearchParams({client_id:oauth.client_id,token:rotated.access_token})});assert.equal(revoked.status,200);
  const denied=await fetch(bridge.base+'/mcp',{method:'POST',headers:{Authorization:`Bearer ${rotated.access_token}`,'Content-Type':'application/json'},body:'{}'});assert.equal(denied.status,401);pass('Revoked OAuth access tokens cannot call MCP');
  const oldDevice=local.config.deviceToken;
  await message({type:'forget'});assert.equal((await message({type:'status'})).paired,false);
  const rejected=await new Promise((resolve,reject)=>{const ws=new WebSocket(bridge.base.replace('http:','ws:')+'/bridge');const timer=setTimeout(()=>{ws.close();reject(new Error('Revoked device socket timed out'));},3000);ws.onopen=()=>ws.send(JSON.stringify({token:oldDevice}));ws.onmessage=e=>{clearTimeout(timer);ws.close();resolve(JSON.parse(String(e.data)));};ws.onerror=reject;});
  assert.equal(rejected.type,'error');pass('Unpair removes the local credential and revokes it on the server');
  const report={suite:'Rust server + OAuth + real WebSocket + compiled Rust/Wasm + DOM/Chrome API fixtures',checks,passed:checks.length,chrome_verified:false,limitation:'Local Chrome cannot launch in the build workspace because AF_UNIX sockets are blocked. Run npm run test:chrome on a normal machine.'};
  await writeFile(join(root,'test-results/integration.json'),JSON.stringify(report,null,2)+'\n');
  console.log(`\n${checks.length} checks passed. Native Chrome is a separate, unverified suite.`);
} finally {
  try {if(listeners.length)await message({type:'stop'});}catch{}
  await bridge.stop();dom.window.close();
}
