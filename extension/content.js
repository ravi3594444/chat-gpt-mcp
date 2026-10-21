(() => {
  if (globalThis.__browserLinkInstalled) return;
  globalThis.__browserLinkInstalled = true;
  const ready = import(chrome.runtime.getURL('pkg/browser_link_core.js')).then(async rust => {
    await rust.default(chrome.runtime.getURL('pkg/browser_link_core_bg.wasm'));
    return rust;
  });
  chrome.runtime.onMessage.addListener((message,_sender,reply) => {
    if (message.type !== 'browser-link-ping' && message.type !== 'browser-link-command') return;
    const received = performance.now();
    ready.then(rust => {
      if (message.type === 'browser-link-ping') reply({ready:true});
      else {
        if (!Number.isFinite(message.command.expires_at) || message.command.expires_at <= Date.now()) throw new Error('Command expired before reaching the page.');
        const started = performance.now();
        const result = JSON.parse(rust.operate(JSON.stringify(message.command)));
        reply({result,timing:{page_ready_ms:Math.round((started-received)*100)/100,page_execution_ms:Math.round((performance.now()-started)*100)/100}});
      }
    }).catch(error => reply({error:String(error.message || error)}));
    return true;
  });
})();
