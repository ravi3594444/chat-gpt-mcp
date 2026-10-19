import init, { validate_command } from './pkg/browser_link_core.js';
const rustReady = init();
let socket = null;
let heartbeat = null;
let reconnectTimer = null;
let connectionState = 'disconnected';
let lastError = '';
let enabled = {};
let commandQueue = Promise.resolve();
const loaded = chrome.storage.session.get('enabled').then(v => { enabled = v.enabled || {}; });

function originOf(url) {
  const parsed = new URL(url);
  if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) throw new Error('Use an ordinary HTTP or HTTPS website.');
  return parsed.origin;
}
async function saveTabs() { await chrome.storage.session.set({enabled}); }
async function badge() {
  const connected = connectionState === 'connected';
  await chrome.action.setBadgeText({text: connected ? String(Object.keys(enabled).length) : ''});
  await chrome.action.setBadgeBackgroundColor({color: '#a8edcd'});
}
async function getEnabledTab(tabId) {
  await loaded;
  const tab = await chrome.tabs.get(tabId);
  if (!enabled[tabId] || originOf(tab.url) !== enabled[tabId].origin) {
    delete enabled[tabId]; await saveTabs(); await badge();
    throw new Error('This tab or site is not enabled. Open Browser Link on that tab and choose Enable this tab.');
  }
  return tab;
}
async function pageCall(tabId, command) {
  await getEnabledTab(tabId);
  try { await chrome.tabs.sendMessage(tabId, {type:'browser-link-ping'}, {frameId:0}); }
  catch { await chrome.scripting.executeScript({target:{tabId,frameIds:[0]},files:['content.js']}); }
  // The document may have changed while the Rust module was loading. Recheck the origin.
  await getEnabledTab(tabId);
  const reply = await chrome.tabs.sendMessage(tabId, {type:'browser-link-command',command}, {frameId:0});
  if (reply?.error) throw new Error(reply.error);
  if (!reply?.result) throw new Error('Page changed before returning a result. Read it again.');
  return reply.result;
}
async function runCommand(raw, expectedSocket) {
  await rustReady;
  const command = JSON.parse(validate_command(JSON.stringify(raw)));
  if (socket !== expectedSocket || connectionState !== 'connected') throw new Error('Browser connection stopped.');
  if (!Number.isFinite(raw.expires_at) || raw.expires_at < Date.now()) throw new Error('Command expired before execution. Read the page before retrying.');
  if (command.action === 'tabs') {
    await loaded;
    const tabs = [];
    for (const id of Object.keys(enabled)) {
      try { const tab = await getEnabledTab(Number(id)); tabs.push({tab_id:tab.id,title:tab.title,url:tab.url}); } catch {}
    }
    return {tabs};
  }
  const tab = await getEnabledTab(command.tab_id);
  if (socket !== expectedSocket) throw new Error('Browser connection stopped.');
  if (command.action === 'navigate') {
    if (originOf(command.url) !== enabled[tab.id].origin) throw new Error('Enable the new site manually before navigating to it.');
    await chrome.tabs.update(tab.id, {url:command.url});
    return {ok:true,instruction:'Navigation started. Take a fresh snapshot after the page loads.'};
  }
  if (command.action === 'screenshot') {
    await chrome.tabs.update(tab.id, {active:true});
    await getEnabledTab(tab.id);
    const screenshot = await chrome.tabs.captureVisibleTab(tab.windowId, {format:'png'});
    const active = await chrome.tabs.query({active:true,windowId:tab.windowId});
    if (active[0]?.id !== tab.id) throw new Error('Active tab changed during capture. No screenshot was returned.');
    await getEnabledTab(tab.id);
    return {tab_id:tab.id,url:tab.url,screenshot};
  }
  return pageCall(tab.id, command);
}
function stopSocket() {
  clearInterval(heartbeat); clearTimeout(reconnectTimer);
  heartbeat = null; reconnectTimer = null;
  const old = socket; socket = null;
  old?.close(); connectionState = 'disconnected';
}
async function connect() {
  await loaded;
  const {config} = await chrome.storage.local.get('config');
  if (!config?.deviceToken || config.paused) return;
  stopSocket(); connectionState = 'connecting'; lastError = '';
  const url = new URL('/bridge',config.server); url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(url); socket = ws;
  ws.onopen = () => ws.send(JSON.stringify({type:'auth',token:config.deviceToken}));
  ws.onmessage = ({data}) => {
    if (socket !== ws) return;
    let message; try { message = JSON.parse(data); } catch { return; }
    if (message.type === 'connected') {
      connectionState = 'connected'; badge();
      heartbeat = setInterval(() => { if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({type:'ping'})); },20_000);
    } else if (message.type === 'error') {
      lastError = message.error; stopSocket(); badge();
    } else if (message.id) {
      commandQueue = commandQueue.then(async () => {
        let response;
        try { response = {id:message.id,result:await runCommand(message,ws)}; }
        catch (error) { response = {id:message.id,error:String(error.message || error)}; }
        if (socket === ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(response));
      }).catch(() => {});
    }
  };
  ws.onerror = () => { if (socket === ws) lastError = 'Cannot reach the bridge. Check its URL and that the server is running.'; };
  ws.onclose = () => {
    if (socket !== ws) return;
    socket = null; connectionState = 'disconnected'; clearInterval(heartbeat); badge();
    reconnectTimer = setTimeout(connect,5000);
  };
}
chrome.tabs.onRemoved.addListener(async id => { await loaded; delete enabled[id]; await saveTabs(); await badge(); });
chrome.tabs.onUpdated.addListener(async (id,change) => {
  await loaded;
  if (change.url && enabled[id]) {
    let origin; try { origin = originOf(change.url); } catch {}
    if (origin !== enabled[id].origin) { delete enabled[id]; await saveTabs(); await badge(); }
  }
});
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onMessage.addListener((message,_sender,reply) => {
  if (!['status','pair','enable','disable','stop','resume','forget'].includes(message.type)) return;
  (async () => {
    await loaded;
    if (message.type === 'status') {
      const {config} = await chrome.storage.local.get('config');
      return {state:connectionState,error:lastError,server:config?.server||'',paired:!!config?.deviceToken,paused:config?.paused||false,tabs:Object.keys(enabled).map(Number)};
    }
    if (message.type === 'pair') {
      const parsed = new URL(message.server); const local = ['127.0.0.1','localhost','[::1]'].includes(parsed.hostname);
      originOf(message.server);
      if (parsed.pathname !== '/' || parsed.search || parsed.hash || (!local && parsed.protocol !== 'https:')) throw new Error('Enter an HTTPS server origin, or HTTP localhost for local use.');
      const response = await fetch(new URL('/pair',parsed.origin),{method:'POST',headers:{'Content-Type':'application/json','Authorization':`Bearer ${message.key}`},body:JSON.stringify({name:'Personal Chrome browser'}),redirect:'error',signal:AbortSignal.timeout(12_000)});
      if (!response.ok) throw new Error(response.status === 401 ? 'Setup key is incorrect.' : `Pairing failed (${response.status}).`);
      const {device_token} = await response.json();
      await chrome.storage.local.set({config:{server:parsed.origin,deviceToken:device_token,paused:false}});
      enabled = {}; await saveTabs(); await connect(); return {ok:true};
    }
    if (message.type === 'enable') {
      const tab = await chrome.tabs.get(message.tabId);
      const origin = originOf(tab.url);
      // Succeeds only after the user's extension action granted activeTab, or a site permission.
      await chrome.scripting.executeScript({target:{tabId:tab.id,frameIds:[0]},func:() => document.title});
      enabled[tab.id] = {origin}; await saveTabs(); await badge(); return {ok:true};
    }
    if (message.type === 'disable') { delete enabled[message.tabId]; await saveTabs(); await badge(); return {ok:true}; }
    const {config} = await chrome.storage.local.get('config');
    if (message.type === 'stop' || message.type === 'forget') {
      stopSocket(); enabled = {}; await saveTabs();
      if (message.type === 'forget') {
        try {
          const response = await fetch(new URL('/unpair',config.server),{method:'POST',headers:{Authorization:`Bearer ${config.deviceToken}`},redirect:'error',signal:AbortSignal.timeout(10_000)});
          if (!response.ok && response.status !== 401) throw new Error('Revocation failed');
        } catch {
          lastError = 'Local pairing removed. The bridge was unreachable; pair again when it is online to invalidate the previous device credential.';
        } finally { await chrome.storage.local.remove('config'); }
      }
      else if (config) await chrome.storage.local.set({config:{...config,paused:true}});
      await badge(); return {ok:true};
    }
    if (message.type === 'resume') { if (!config) throw new Error('Pair the extension first.'); await chrome.storage.local.set({config:{...config,paused:false}}); await connect(); return {ok:true}; }
  })().then(result => reply(result)).catch(error => reply({error:String(error.message || error)}));
  return true;
});
connect();
