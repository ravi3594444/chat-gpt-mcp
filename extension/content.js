(() => {
  if (globalThis.__browserLinkInstalled) return;
  globalThis.__browserLinkInstalled = true;
  const ready = import(chrome.runtime.getURL('pkg/browser_link_core.js')).then(async rust => {
    await rust.default(chrome.runtime.getURL('pkg/browser_link_core_bg.wasm'));
    return rust;
  });
  chrome.runtime.onMessage.addListener((message,_sender,reply) => {
    if (message.type !== 'browser-link-ping' && message.type !== 'browser-link-command') return;
    ready.then(rust => {
      if (message.type === 'browser-link-ping') reply({ready:true});
      else {
        if (message.command.expires_at < Date.now()) throw new Error('Command expired before reaching the page.');
        reply({result:JSON.parse(rust.operate(JSON.stringify(message.command)))});
      }
    }).catch(error => reply({error:String(error.message || error)}));
    return true;
  });
})();
