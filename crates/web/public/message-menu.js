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
  const heading=document.createElement('h2');heading.textContent='Message Actions';menu.append(heading);
  for(const source of actions) {
    const button=source.cloneNode(true);button.className='button message-menu-action';
    const label=document.createElement('span');label.textContent=source.getAttribute('aria-label');button.append(label);
    if(button.dataset.live.startsWith('delete:'))button.classList.add('danger-text');
    menu.append(button);
  }
  const text=row.querySelector('.markdown')?.textContent;
  if(text){const copy=document.createElement('button');copy.className='button message-menu-action';copy.textContent='Copy Message';copy.dataset.copy=text;menu.append(copy);}
  const cancel=document.createElement('button');cancel.className='button secondary';cancel.textContent='Cancel';cancel.onclick=closeMessageMenu;menu.append(cancel);
  menu.addEventListener('close',()=>{menu.remove();if(menu.dataset.silent!=='true')changed();});
  // The finger which opened a long-press sheet is still over its backdrop.
  // Its release must not immediately dismiss the freshly opened sheet.
  menu.addEventListener('click',e=>{if(e.target===menu&&performance.now()-opened>750){const r=menu.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)closeMessageMenu();}});
  document.body.append(menu);menu.showModal();changed();
}
