// Queue/deadline regressions using the actual extension adapters and synthetic Chrome APIs.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {performance} from 'node:perf_hooks';
import test from 'node:test';

const workerSource = await readFile(new URL('../extension/worker.js',import.meta.url),'utf8');
const contentSource = await readFile(new URL('../extension/content.js',import.meta.url),'utf8');
const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
async function until(check) {
  for (let i=0;i<400;i++) { const result=await check(); if (result) return result; await delay(5); }
  throw new Error('Fixture condition timed out');
}
function deferred() { let resolve; const promise=new Promise(done => {resolve=done;}); return {promise,resolve}; }

async function workerFixture() {
  const sockets=[],responses=[],pageCalls=[];
  const api={ping:async()=>({ready:true}),page:async command=>({result:{ok:true,action:command.action},timing:{page_ready_ms:0.1,page_execution_ms:0.25}})};
  const local={config:{server:'https://bridge.example',deviceToken:'synthetic-fixture-only',paused:false}};
  const session={enabled:{1:{origin:'https://fixture.example'}}};
  const storage=data=>({get:async key=>({[key]:data[key]}),set:async patch=>Object.assign(data,patch),remove:async key=>{delete data[key];}});
  let listener;
  const ignore={addListener(){}};
  const chrome={
    storage:{local:storage(local),session:storage(session)},
    action:{setBadgeText:async()=>{},setBadgeBackgroundColor:async()=>{}},
    tabs:{get:async id=>({id,url:'https://fixture.example/page',title:'Local fixture'}),
      sendMessage:async(_id,payload)=>{
        if (payload.type==='browser-link-ping') return api.ping();
        pageCalls.push(payload.command); return api.page(payload.command);
      },onRemoved:ignore,onUpdated:ignore},
    scripting:{executeScript:async()=>[]},
    runtime:{onStartup:ignore,onMessage:{addListener:fn=>{listener=fn;}}}
  };
  class Socket {
    static OPEN=1;
    constructor(){this.readyState=1;sockets.push(this);queueMicrotask(()=>this.onopen?.());}
    send(value){const message=JSON.parse(value);if(message.type==='auth')queueMicrotask(()=>this.onmessage?.({data:JSON.stringify({type:'connected'})}));else responses.push(message);}
    close(){this.readyState=3;queueMicrotask(()=>this.onclose?.());}
  }
  const context=vm.createContext({chrome,WebSocket:Socket,URL,performance,init:async()=>{},validate_command:value=>value,setTimeout,clearTimeout,setInterval,clearInterval});
  vm.runInContext(workerSource.replace(/^import[^\n]*\n/,''),context,{filename:'extension/worker.js'});
  const send=payload=>new Promise((resolve,reject)=>listener(payload,{},result=>result?.error?reject(new Error(result.error)):resolve(result)));
  await until(async()=>(await send({type:'status'})).state==='connected');
  let id=0;
  function command(action,options={}){
    const request={id:'fixture-'+(++id),action,tab_id:1,expires_at:Date.now()+5000,...options};
    sockets.at(-1).onmessage({data:JSON.stringify(request)});
    return {request,reply:()=>until(()=>responses.find(value=>value.id===request.id))};
  }
  return {api,pageCalls,responses,command,send,disconnect:()=>sockets.at(-1).close(),close:()=>send({type:'stop'})};
}

test('Successful actions expose only numeric stage durations and the extension version',async()=>{
  const f=await workerFixture();
  try{
    const response=await f.command('click').reply();assert(!response.error);
    const timing=response.result.timing;
    assert.equal(timing.extension_version,'0.2.2');
    for(const [key,value] of Object.entries(timing))if(key!=='extension_version')assert(Number.isFinite(value)&&value>=0,key);
    assert.equal(timing.page_execution_ms,0.25);assert.equal(timing.page_ready_ms,0.1);
    assert(timing.extension_total_ms>=timing.extension_work_ms);
    assert.deepEqual(Object.keys(timing).sort(),['extension_prepare_ms','extension_queue_ms','extension_total_ms','extension_version','extension_work_ms','page_execution_ms','page_ready_ms','page_roundtrip_ms'].sort());
    const status=await f.send({type:'status'});assert.equal(status.last_timing.action,'click');
    assert(!JSON.stringify(timing).includes('fixture.example'));assert(!JSON.stringify(timing).includes('synthetic-fixture-only'));
  }finally{await f.close();}
});

test('A stalled page response releases the queue and requires inspection before another action',async()=>{
  const f=await workerFixture(),stalled=deferred();
  try{
    f.api.page=()=>stalled.promise;
    const first=f.command('click',{expires_at:Date.now()+70});
    await until(()=>f.pageCalls.length===1);
    const tabs=f.command('tabs');
    assert.match((await first.reply()).error,/timed out/);assert((await tabs.reply()).result.tabs);
    const blocked=await f.command('click').reply();assert.match(blocked.error,/fresh snapshot/);assert.equal(f.pageCalls.length,1);
    f.api.page=async command=>({result:{ok:true,action:command.action}});
    assert(!(await f.command('snapshot').reply()).error);
    assert(!(await f.command('click').reply()).error);
    stalled.resolve({result:{ok:true}});await delay(10);
    assert.equal(f.responses.filter(value=>value.id===first.request.id).length,1,'A late result must not send another response');
  }finally{stalled.resolve({result:{ok:true}});await f.close();}
});

test('A page that becomes ready after the deadline cannot receive a late click',async()=>{
  const f=await workerFixture(),ready=deferred();let pinged=false;
  try{
    f.api.ping=()=>{pinged=true;return ready.promise;};
    const first=f.command('click',{expires_at:Date.now()+70});await until(()=>pinged);
    assert.match((await first.reply()).error,/timed out/);
    ready.resolve({ready:true});await delay(15);
    assert.equal(f.pageCalls.length,0,'Expired work must stop after warmup, before dispatch');
  }finally{ready.resolve({ready:true});await f.close();}
});

test('A command that expires in the queue never reaches the page',async()=>{
  const f=await workerFixture(),firstReady=deferred();
  try{
    f.api.page=async command=>{if(command.action==='snapshot')await firstReady.promise;return{result:{ok:true}};};
    const first=f.command('snapshot');await until(()=>f.pageCalls.length===1);
    const expired=f.command('click',{expires_at:Date.now()+25});
    await delay(45);firstReady.resolve();assert(!(await first.reply()).error);
    assert.match((await expired.reply()).error,/expired/);assert.equal(f.pageCalls.length,1);
  }finally{firstReady.resolve();await f.close();}
});

test('Stop cancels a stalled command promptly and prevents dispatch after Resume',async()=>{
  const f=await workerFixture(),ready=deferred();let pinged=false;
  try{
    f.api.ping=()=>{pinged=true;return ready.promise;};
    f.command('click');await until(()=>pinged);
    await f.send({type:'stop'});await f.send({type:'resume'});
    await until(async()=>(await f.send({type:'status'})).state==='connected');
    await f.send({type:'enable',tabId:1});
    assert((await f.command('tabs').reply()).result.tabs);
    ready.resolve({ready:true});await delay(15);assert.equal(f.pageCalls.length,0);
  }finally{ready.resolve({ready:true});await f.close();}
});

test('A server expiry longer than twenty seconds is shortened before page dispatch',async()=>{
  const f=await workerFixture();
  try{
    const received=Date.now();assert(!(await f.command('click',{expires_at:received+60_000}).reply()).error);
    assert(f.pageCalls[0].expires_at<=received+20_050);
    assert(f.pageCalls[0].expires_at>received+19_000);
  }finally{await f.close();}
});

test('An unexpected socket close releases a stalled command before reconnecting',async()=>{
  const f=await workerFixture(),ready=deferred();let pinged=false;
  try{
    f.api.ping=()=>{pinged=true;return ready.promise;};
    f.command('click');await until(()=>pinged);f.disconnect();
    await until(async()=>(await f.send({type:'status'})).last_timing?.outcome==='error');
    await f.send({type:'resume'});await until(async()=>(await f.send({type:'status'})).state==='connected');
    assert((await f.command('tabs').reply()).result.tabs);
    ready.resolve({ready:true});await delay(15);assert.equal(f.pageCalls.length,0);
  }finally{ready.resolve({ready:true});await f.close();}
});

function contentFixture({ready=Promise.resolve(),operate=value=>JSON.stringify({ok:true,action:JSON.parse(value).action})}={}){
  let listener,calls=0;
  const rust={default:()=>ready,operate:value=>{calls++;return operate(value);}};
  const chrome={runtime:{getURL:value=>value,onMessage:{addListener:fn=>{listener=fn;}}}};
  const context=vm.createContext({chrome,performance,testRust:rust});
  vm.runInContext(contentSource.replace("import(chrome.runtime.getURL('pkg/browser_link_core.js'))",'Promise.resolve(testRust)'),context,{filename:'extension/content.js'});
  const send=command=>new Promise(resolve=>listener({type:'browser-link-command',command},{},resolve));
  return {send,calls:()=>calls};
}

test('The content adapter reports time in Rust/Wasm separately from module readiness',async()=>{
  const ready=deferred();
  const f=contentFixture({ready:ready.promise,operate:()=>{const end=performance.now()+5;while(performance.now()<end){}return'{"ok":true}';}});
  const reply=f.send({action:'click',expires_at:Date.now()+5000});await delay(15);ready.resolve();
  const result=await reply;assert(result.result.ok);assert(result.timing.page_ready_ms>=10);assert(result.timing.page_execution_ms>=4.5);assert.equal(f.calls(),1);
});

test('The content adapter never executes a command that expired during Wasm loading',async()=>{
  const ready=deferred(),f=contentFixture({ready:ready.promise});
  const reply=f.send({action:'click',expires_at:Date.now()+25});await delay(45);ready.resolve();
  assert.match((await reply).error,/expired/);assert.equal(f.calls(),0);
});

test('The content adapter rejects missing or invalid expiration before execution',async()=>{
  const f=contentFixture();
  for(const expires_at of [undefined,null,'tomorrow',NaN,Infinity,Date.now()-1]){
    assert.match((await f.send({action:'click',expires_at})).error,/expired/);
  }
  assert.equal(f.calls(),0);
});
