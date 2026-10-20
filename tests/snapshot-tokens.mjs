// Identical local DOM fixtures, actual Wasm extractors and real MCP replies.
// This measures text replies, not native-browser/model-driven task success or billing.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {JSDOM} from 'jsdom';
import WebSocket from 'ws';
import {getEncoding} from 'js-tiktoken';
import {startBridge,root} from './harness.mjs';

const enc=getEncoding('o200k_base');
const count=value=>enc.encode(typeof value==='string'?value:JSON.stringify(value)).length;
const baselineRoot=process.env.BROWSER_LINK_BASELINE_ROOT;
const baselineBinary=process.env.BROWSER_LINK_BASELINE_BINARY;
if(Boolean(baselineRoot)!==Boolean(baselineBinary))throw new Error('Provide both BROWSER_LINK_BASELINE_ROOT and BROWSER_LINK_BASELINE_BINARY, or neither.');
const dom=new JSDOM('<!doctype html><html><head><title>Fixture</title></head><body></body></html>',{url:'https://fixture.example/',pretendToBeVisual:true});
for(const name of ['Window','Document','Element','Node','HTMLElement','HTMLInputElement','HTMLTextAreaElement','HTMLSelectElement','ShadowRoot','Event','EventInit'])globalThis[name]=dom.window[name];
globalThis.window=dom.window;globalThis.document=dom.window.document;
dom.window.HTMLElement.prototype.getBoundingClientRect=function(){return {width:200,height:30,top:0,left:0,right:200,bottom:30,x:0,y:0};};
Object.defineProperty(dom.window.HTMLElement.prototype,'innerText',{get(){return this.textContent;}});
async function loadCore(directory){
  const wasm=await import(pathToFileURL(join(directory,'extension/pkg/browser_link_core.js')));
  await wasm.default({module_or_path:new Uint8Array(await readFile(join(directory,'extension/pkg/browser_link_core_bg.wasm')))});
  return wasm;
}
const currentCore=await loadCore(root);
const previousCore=baselineRoot?await loadCore(resolve(baselineRoot)):currentCore;
const bridges=[];
async function replay(binary){
  const bridge=await startBridge({binary});bridges.push(bridge);
  const auth=await bridge.authorize();let raw;
  const paired=await(await fetch(bridge.base+'/pair',{method:'POST',headers:{Authorization:'Bearer '+bridge.key,'Content-Type':'application/json'},body:JSON.stringify({name:'Local token fixture'})})).json();
  const ws=new WebSocket(bridge.base.replace('http:','ws:')+'/bridge');
  await new Promise((resolve_,reject)=>{
    ws.on('open',()=>ws.send(JSON.stringify({token:paired.device_token})));ws.on('error',reject);
    ws.on('message',data=>{const message=JSON.parse(String(data));if(message.type==='connected')resolve_();else if(message.id)ws.send(JSON.stringify({id:message.id,result:raw}));});
  });
  return {
    version:(await(await fetch(bridge.base+'/health')).json()).version,
    tools:(await bridge.rpc(auth.access_token,'tools/list')).tools,
    async snapshot(value,args={}){raw=value;const result=await bridge.rpc(auth.access_token,'tools/call',{name:'browser_snapshot',arguments:{tab_id:1,...args}});assert(!result.isError,JSON.stringify(result));return result.content.find(c=>c.type==='text').text;},
    close:()=>ws.close(),
  };
}
const dashboardNames=[...Array.from({length:100},(_,i)=>`History action ${String(i+1).padStart(3,'0')}`),'Message composer','Add files','Select model'];
const fixtures=[
  {name:'contact-form',html:'<h1>Contact workspace</h1><p>Save a contact draft locally.</p><form><label for="name">Customer name</label><input id="name"><label for="email">Email</label><input id="email" type="email"><label for="company">Company</label><input id="company"><label for="plan">Plan</label><select id="plan"><option value="basic">Basic</option><option value="pro">Pro</option></select><button>Save draft</button></form>',expected:['Customer name','Email','Company','Plan','Save draft']},
  {name:'dashboard-103-controls',html:'<h1>Workspace dashboard</h1><p>'+Array.from({length:15},(_,i)=>`Activity ${i+1}: Contact record reviewed.`).join(' ')+'</p>'+dashboardNames.slice(0,100).map(name=>`<button>${name}</button>`).join('')+'<label for="composer">Message composer</label><textarea id="composer"></textarea><button>Add files</button><button>Select model</button>',expected:dashboardNames},
  {name:'accessible-controls',html:'<h1>Preferences</h1><span id="email-label">Contact email</span><input aria-labelledby="email-label"><div role="checkbox" aria-checked="true">Subscribe to updates</div><div role="switch" aria-label="Notifications" aria-checked="false"></div><div id="shadow-host"></div>',expected:['Contact email','Subscribe to updates','Notifications','Shadow save'],shadow:true},
];
let current,previous;
try{
  current=await replay();previous=baselineBinary?await replay(baselineBinary):current;
  const report={tokenizer:'o200k_base',method:baselineRoot?'v0.1.3 and updated Wasm on identical fixed-geometry DOM fixtures, actual MCP replies':'Updated extractor: structured JSON vs readable text on identical DOM fixtures',limitations:'Not a native Chrome, model-driven full-task, or billed-token benchmark.',versions:{baseline:previous.version,current:current.version},tool_definitions:{baseline_tokens:count(previous.tools),current_tokens:count(current.tools)},fixtures:[]};
  for(const fixture of fixtures){
    dom.reconfigure({url:'https://fixture.example/'+fixture.name});document.title=fixture.name;document.body.innerHTML=fixture.html;
    if(fixture.shadow)document.getElementById('shadow-host').attachShadow({mode:'open'}).innerHTML='<button>Shadow save</button>';
    const oldRaw=JSON.parse(previousCore.snapshot()),newRaw=JSON.parse(currentCore.snapshot());
    const oldText=await previous.snapshot(oldRaw,baselineRoot?{}:{format:'json'});
    const newJson=JSON.parse(await current.snapshot(newRaw,{format:'json'}));
    const newText=await current.snapshot(newRaw);
    const oldMissing=fixture.expected.filter(label=>!oldRaw.elements.some(e=>e.label===label));
    const newMissing=fixture.expected.filter(label=>!newJson.elements.some(e=>e.label===label));assert.equal(newMissing.length,0);
    for(const label of fixture.expected)assert(newText.split('\n').some(line=>line.includes('[ref=')&&line.includes(JSON.stringify(label))),label);
    const full=JSON.parse(await current.snapshot(newRaw,{format:'json',detail:'full'}));assert.equal(full.text,newRaw.text);assert.equal(full.elements.length,newRaw.elements.length);
    const row={name:fixture.name,baseline_tokens:count(oldText),current_tokens:count(newText),reduction_percent:Number((100*(1-count(newText)/count(oldText))).toFixed(1)),baseline_controls:oldRaw.elements.length,current_controls:newJson.elements.length,baseline_missing_targets:oldMissing,current_missing_targets:newMissing};
    if(fixture.shadow){assert(newText.includes('checked=true'));assert(newText.includes('checked=false'));}
    report.fixtures.push(row);
  }
  assert(report.fixtures.find(f=>f.name==='dashboard-103-controls').reduction_percent>20,'Busy page should retain its targets with materially less output');
  await mkdir(join(root,'test-results'),{recursive:true});await writeFile(join(root,'test-results/snapshot-tokens.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify(report,null,2));
}finally{current?.close();if(previous!==current)previous?.close();for(const bridge of bridges)await bridge.stop();dom.window.close();}
