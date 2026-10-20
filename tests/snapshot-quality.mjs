// Replay a snapshot through the real server. No user-browser actions are sent.
// Optional input is a private, temporary capture; it must never be committed.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import WebSocket from 'ws';
import {startBridge} from './harness.mjs';

const input=process.argv[2];
const raw=input?JSON.parse(await readFile(input,'utf8')):{
  title:'Conversation fixture',url:'https://fixture.example/chat',text:'Local conversation fixture',
  elements:[
    ...Array.from({length:60},(_,i)=>({reference:`document:1:${i}`,tag:'button',label:`History action ${i}`,type:'button',disabled:false})),
    ...['Work with ChatGPT','Add files and more','Select ChatGPT model'].map((label,i)=>({reference:`document:1:${60+i}`,tag:i?'button':'div',label,type:i?'button':'',disabled:false}))
  ]
};
const bridge=await startBridge();
let socket;
try{
  const health=await (await fetch(bridge.base+'/health')).json();
  const paired=await fetch(bridge.base+'/pair',{method:'POST',headers:{Authorization:`Bearer ${bridge.key}`,'Content-Type':'application/json'},body:JSON.stringify({name:'Local snapshot replay'})});
  assert.equal(paired.status,200);
  const {device_token}=await paired.json();
  socket=new WebSocket(bridge.base.replace('http:','ws:')+'/bridge');
  await new Promise((resolve,reject)=>{
    socket.on('open',()=>socket.send(JSON.stringify({token:device_token})));
    socket.on('error',reject);
    socket.on('message',data=>{
      const command=JSON.parse(String(data));
      if(command.type==='connected'){resolve();return;}
      if(command.id){
        const response=command.action==='snapshot'?{id:command.id,result:raw}:{id:command.id,error:'Replay is read-only'};
        socket.send(JSON.stringify(response));
      }
    });
  });
  const auth=await bridge.authorize();
  const discovered=await bridge.rpc(auth.access_token,'tools/list');
  assert(discovered.tools.find(t=>t.name==='browser_snapshot')?.inputSchema?.properties?.detail,'Build the updated server source before running this check');
  const call=async args=>{
    const result=await bridge.rpc(auth.access_token,'tools/call',{name:'browser_snapshot',arguments:{tab_id:1,...args}});
    assert(!result.isError,result.content.map(c=>c.text).join('\n'));
    return JSON.parse(result.content.find(c=>c.type==='text').text);
  };
  const compact=await call({});
  const full=await call({detail:'full'});
  const equalElement=(a,b)=>{
    const left={...a},right={...b};delete left.reference;delete right.reference;
    assert.deepEqual(left,right);
  };
  assert.equal(full.text,raw.text);
  assert.equal(full.elements.length,raw.elements.length);
  full.elements.forEach((e,i)=>equalElement(e,raw.elements[i]));
  const labelKey=e=>JSON.stringify([e.tag,e.label]);
  const returnedCounts=new Map();
  for(const e of compact.elements)returnedCounts.set(labelKey(e),(returnedCounts.get(labelKey(e))||0)+1);
  const missing=[];
  for(const e of raw.elements){
    const count=returnedCounts.get(labelKey(e))||0;
    if(count>0)returnedCounts.set(labelKey(e),count-1);else missing.push(e.label);
  }
  const focused=[];
  for(const label of ['Work with ChatGPT','Add files and more','Select ChatGPT model']){
    if(!raw.elements.some(e=>e.label===label))continue;
    const result=await call({query:label,max_chars:0,detail:'full'});
    assert(result.elements.some(e=>e.label===label),`Focused read failed to recover ${label}`);
    focused.push(label);
  }
  const rawChars=Array.from(JSON.stringify(raw)).length,compactChars=Array.from(JSON.stringify(compact)).length;
  console.log(JSON.stringify({version:health.version,source:input?'Live capture replayed locally':'Synthetic conversation fixture',raw_controls:raw.elements.length,compact_controls:compact.elements.length,missing_controls:missing,raw_text_chars:Array.from(raw.text).length,compact_text_chars:Array.from(compact.text).length,full_detail_matches_raw:true,focused_controls_recovered:focused,raw_chars:rawChars,compact_chars:compactChars,character_reduction_percent:Number((100*(1-compactChars/rawChars)).toFixed(1))},null,2));
  if(process.env.ALLOW_MISSING_CONTROLS!=='1')assert.equal(missing.length,0,'Default read omitted controls');
}finally{
  socket?.close();
  await bridge.stop();
}
