const $ = id => document.getElementById(id);
const send = message => chrome.runtime.sendMessage(message).then(result => { if (result?.error) throw new Error(result.error); return result; });
let currentTab;
let latest;
async function refresh() {
  latest = await send({type:'status'});
  currentTab = (await chrome.tabs.query({active:true,currentWindow:true}))[0];
  $('status').textContent = latest.state === 'connected' ? 'Connected to your bridge' : latest.paused ? 'Access stopped' : latest.state === 'connecting' ? 'Connecting…' : 'Bridge disconnected';
  $('dot').classList.toggle('live',latest.state === 'connected');
  $('setup').hidden = latest.paired;
  $('controls').hidden = !latest.paired;
  $('tab-title').textContent = currentTab?.title || 'No website selected';
  let origin = ''; try { origin = new URL(currentTab?.url).origin; } catch {}
  $('tab-origin').textContent = origin;
  const enabled = latest.tabs.includes(currentTab?.id);
  $('toggle-tab').textContent = enabled ? 'Disable this tab' : 'Enable this tab';
  $('toggle-tab').disabled = !/^https?:/.test(currentTab?.url || '') || latest.paused;
  $('tab-note').textContent = enabled ? 'ChatGPT can work in this tab while it stays on this site.' : 'ChatGPT can access this tab only after you enable it.';
  $('tab-count').textContent = latest.tabs.length;
  $('connected-server').textContent = latest.server;
  $('stop').hidden = latest.paused;
  $('resume').hidden = latest.state === 'connected';
  if (latest.error) $('message').textContent = latest.error;
}
function action(fn) { return async event => { event?.preventDefault(); $('message').textContent = ''; try { await fn(); await refresh(); } catch(error) { $('message').textContent = error.message; } }; }
$('pair-form').addEventListener('submit',action(async () => {
  const server = new URL($('server').value).origin;
  const allowed = await chrome.permissions.request({origins:[server+'/*']});
  if (!allowed) throw new Error('Allow access to your bridge address to connect.');
  await send({type:'pair',server:$('server').value,key:$('key').value});
  $('key').value = '';
}));
$('toggle-tab').addEventListener('click',action(() => send({type:latest.tabs.includes(currentTab.id)?'disable':'enable',tabId:currentTab.id})));
$('stop').addEventListener('click',action(() => send({type:'stop'})));
$('resume').addEventListener('click',action(() => send({type:'resume'})));
$('forget').addEventListener('click',action(() => send({type:'forget'})));
refresh().catch(error => { $('message').textContent = error.message; });
setInterval(() => refresh().catch(() => {}),1500);
