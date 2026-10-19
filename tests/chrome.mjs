// Run on a normal machine with Chrome/Chromium. Uses local fixture pages only.
// The test copy pre-grants ONLY the two local fixture origins, because opening
// popup.html as a test page does not grant Chrome's toolbar activeTab gesture.
// The distributed extension manifest does not contain these host permissions.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp,cp,mkdir} from 'node:fs/promises';
import {createServer} from 'node:http';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {chromium} from 'playwright';
import {startBridge,waitFor,root,fixtureHTML} from './harness.mjs';

const bridge=await startBridge();
const fixture=createServer((_req,res)=>{res.setHeader('Content-Type','text/html');res.end(fixtureHTML);});
await new Promise(r=>fixture.listen(0,'0.0.0.0',r));
const fixtureUrl=`http://127.0.0.1:${fixture.address().port}`;
const temp=await mkdtemp(join(tmpdir(),'browser-link-chrome-'));
const extension=join(temp,'extension');await cp(join(root,'extension'),extension,{recursive:true});
const manifest=JSON.parse(await readFile(join(extension,'manifest.json'),'utf8'));
manifest.host_permissions=[fixtureUrl+'/*',bridge.base+'/*'];
await writeFile(join(extension,'manifest.json'),JSON.stringify(manifest));
let context;
try {
  context=await chromium.launchPersistentContext(join(temp,'profile'),{
    headless:process.env.HEADED!=='1',
    ...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{channel:'chromium'}),
    args:[`--disable-extensions-except=${extension}`,`--load-extension=${extension}`]
  });
  const worker=context.serviceWorkers()[0]||await context.waitForEvent('serviceworker');
  const extensionId=new URL(worker.url()).hostname;
  const page=await context.newPage();await page.goto(fixtureUrl+'/contact');
  const popup=await context.newPage();await popup.goto(`chrome-extension://${extensionId}/popup.html`);
  await popup.locator('#server').fill(bridge.base);await popup.locator('#key').fill(bridge.key);await popup.locator('#pair').click();
  await waitFor(async()=>await popup.locator('#status').textContent()==='Connected to your bridge','Extension did not connect');
  assert.equal(await popup.locator('#key').inputValue(),'');
  const tabId=await worker.evaluate(async url=>(await chrome.tabs.query({})).find(t=>t.url===url).id,fixtureUrl+'/contact');
  await popup.evaluate(tabId=>chrome.runtime.sendMessage({type:'enable',tabId}),tabId);
  const token=await bridge.authorize();
  const call=(name,args={})=>bridge.rpc(token.access_token,'tools/call',{name,arguments:args});
  const text=result=>JSON.parse(result.content.find(c=>c.type==='text').text);
  const tabs=text(await call('browser_tabs')).tabs;assert.deepEqual(tabs.map(t=>t.tab_id),[tabId]);
  let snapshot=text(await call('browser_snapshot',{tab_id:tabId}));assert.match(snapshot.text,/Contact workspace/);
  const name=snapshot.elements.find(e=>e.label==='Customer name').reference;
  assert(!(await call('browser_fill',{tab_id:tabId,reference:name,text:'Alex'})).isError);
  assert.equal(await page.locator('#name').inputValue(),'Alex');
  const selected=snapshot.elements.find(e=>e.label==='Plan').reference;
  assert(!(await call('browser_select',{tab_id:tabId,reference:selected,value:'pro'})).isError);
  assert.equal(await page.locator('#plan').inputValue(),'pro');
  const save=snapshot.elements.find(e=>e.label==='Save draft').reference;
  assert(!(await call('browser_click',{tab_id:tabId,reference:save})).isError);
  assert.equal(await page.locator('#result').textContent(),'Draft saved for Alex');
  snapshot=text(await call('browser_snapshot',{tab_id:tabId}));assert.equal(snapshot.elements.find(e=>e.label==='Customer name').value,'Alex');
  assert((await call('browser_click',{tab_id:tabId,reference:save})).isError);
  assert((await call('browser_navigate',{tab_id:tabId,url:'https://unapproved.example/'})).isError);
  // Capturing a viewport requires the actual toolbar activeTab grant; do that in
  // the manual smoke check, rather than broadening this test copy to <all_urls>.
  await popup.evaluate(()=>chrome.runtime.sendMessage({type:'stop'}));
  await waitFor(async()=>text(await call('browser_status')).connected===false,'Stop did not disconnect');
  const directory=join(root,'test-results');await mkdir(directory,{recursive:true});
  await writeFile(join(directory,'chrome.json'),JSON.stringify({passed:true,browser:context.browser()?.version(),note:'Actual unpacked extension and Rust/Wasm on local fixtures. Test-only manifest pre-grants fixture origins. Toolbar activeTab permission prompting and real screenshots require the manual smoke check.'},null,2)+'\n');
  console.log('PASS real Chrome: pairing, enabled tabs, Wasm snapshots, fill, select, click, stale references, origin boundary and Stop.');
} finally {await context?.close();await new Promise(r=>fixture.close(r));await bridge.stop();}
