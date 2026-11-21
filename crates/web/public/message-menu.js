// Native dialog focus handling; commands are copied from the authorized row UI.
const changed=()=>dispatchEvent(new Event('sticktalk-surfaces-changed'));
export function closeMessageMenu(notify=true) {
  const menu=document.getElementById('live-message-menu');
  if(menu){menu.dataset.silent=String(!notify);menu.close();menu.remove();if(notify)changed();}
}
export function openMessageMenu(row) {
  closeMessageMenu(false);
  const actions=[...row.querySelectorAll('.message-actions button')];
  if(!actions.length)return;
  const menu=document.createElement('dialog');menu.id='live-message-menu';menu.className='message-menu';
  const opened=performance.now();
  menu.setAttribute('aria-label','Message Actions');
  const handle=document.createElement('div');handle.className='sheet-handle';handle.setAttribute('aria-hidden','true');menu.append(handle);
  const defaults=['👍','❤️','😂','🎉','😮','😢'];
  let recent=[];
  try { const raw=localStorage.getItem('sticktalk-recent-reactions')??'[]';const stored=raw.length<=512?JSON.parse(raw):[];if(Array.isArray(stored))recent=stored.filter(e=>typeof e==='string'&&e.length<=16).slice(0,6); } catch {}
  const quick=document.createElement('div');quick.className='quick-reactions';quick.setAttribute('aria-label','Quick reactions');
  for(const emoji of [...new Set([...recent,...defaults])].slice(0,6)) {
    const button=document.createElement('button');button.textContent=emoji;button.setAttribute('aria-label',`React with ${emoji}`);button.dataset.live=`react:${row.dataset.liveMessage},${emoji}`;
    button.addEventListener('click',()=>{try{localStorage.setItem('sticktalk-recent-reactions',JSON.stringify([emoji,...recent.filter(e=>e!==emoji)].slice(0,6)));}catch{}});
    quick.append(button);
  }
  menu.append(quick);
  const heading=document.createElement('h2');heading.textContent='Message Actions';menu.append(heading);
  for(const source of actions) {
    const button=source.cloneNode(true);button.className='button message-menu-action';
    const label=document.createElement('span');label.textContent=source.getAttribute('aria-label');button.append(label);
    if(button.dataset.live.startsWith('delete:'))button.classList.add('danger-text');
    menu.append(button);
  }
  // One line per paragraph, as it was written.
  const text=[...row.querySelectorAll('.markdown > *')].map(block=>block.textContent).join('\n');
  if(text){const copy=document.createElement('button');copy.className='button message-menu-action';const glyph=document.createElementNS('http://www.w3.org/2000/svg','svg');glyph.classList.add('icon');glyph.setAttribute('viewBox','0 0 24 24');glyph.setAttribute('aria-hidden','true');const use=document.createElementNS(glyph.namespaceURI,'use');use.setAttribute('href','#i-copy');glyph.append(use);const label=document.createElement('span');label.textContent='Copy Message';copy.append(glyph,label);copy.dataset.copy=text;menu.append(copy);}
  let start;
  menu.addEventListener('pointerdown',e=>{if(e.target===handle||e.target===menu||e.target===heading)start={x:e.clientX,y:e.clientY};});
  menu.addEventListener('pointerup',e=>{if(start&&e.clientY-start.y>64&&Math.abs(e.clientX-start.x)<64)closeMessageMenu();start=undefined;});
  menu.addEventListener('pointercancel',()=>{start=undefined;});
  menu.addEventListener('close',()=>{menu.remove();if(menu.dataset.silent!=='true')changed();});
  // The finger which opened a long-press sheet is still over its backdrop.
  // Its release must not immediately dismiss the freshly opened sheet.
  menu.addEventListener('click',e=>{if(e.target===menu&&performance.now()-opened>750){const r=menu.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)closeMessageMenu();}});
  document.body.append(menu);menu.showModal();changed();
}
