export function swipeAction(panel, dx, dy) {
  if(Math.abs(dx)<80 || Math.abs(dx)<=Math.abs(dy)*1.25)return null;
  if(panel==='left')return dx<0?'panel:':null;
  if(panel==='right')return dx>0?'panel:':null;
  return dx>0?'panel:left':'panel:right';
}
let previous='', emit, traversing=false, replaying=false, queued;
function state() {
  const shell=document.querySelector('.live-shell');
  return {dialog:document.querySelector('#live-message-menu')?'@actions':document.querySelector('#live-viewer')?'@viewer':document.querySelector('.live-modal')?.dataset.dialog??'',
    panel:shell?.classList.contains('panel-left')?'left':shell?.classList.contains('panel-right')?'right':''};
}
export function installOverlayHistory(send) {
  emit=send;
  // history.go is asynchronous. Coalesce a fast second navigation until its
  // popstate has restored the previous view, rather than closing the new drawer.
  addEventListener('sticktalk-live',event=>{
    if(traversing&&!replaying&&/^(panel:|dialog:|channel:|guild:|home$)/.test(event.detail)) {
      queued=event.detail;event.stopImmediatePropagation();
    }
  },true);
  history.replaceState({...history.state,sticktalkOverlay:{dialog:'',panel:'',depth:0}},'');
  addEventListener('popstate',()=>{
    let next=history.state?.sticktalkOverlay??{dialog:'',panel:'',depth:0};
    if(next.dialog.startsWith('@')&&state().dialog!==next.dialog) {
      next={dialog:'',panel:'',depth:0};history.replaceState({...history.state,sticktalkOverlay:next},'');
    }
    previous=JSON.stringify({dialog:next.dialog,panel:next.panel});
    traversing=true;replaying=true;
    if(!next.dialog.startsWith('@'))dispatchEvent(new Event('sticktalk-close-surfaces'));
    if(!document.querySelector('.live-shell')){traversing=false;replaying=false;queued=undefined;return;}
    if(next.dialog.startsWith('@')){traversing=false;replaying=false;return;}
    if(next.dialog)emit(`dialog:${next.dialog}`);
    else if(next.panel){emit('close');emit(`panel:${next.panel}`);}
    else emit('close');
    replaying=false;
    queueMicrotask(()=>{traversing=false;const action=queued;queued=undefined;if(action)emit(action);});
  });
}
export function syncOverlayHistory() {
  const next=state(), key=JSON.stringify(next);
  if(key===previous || traversing)return;
  const old=history.state?.sticktalkOverlay??{dialog:'',panel:'',depth:0};
  previous=key;
  if(old.dialog.startsWith('@')&&old.parentDialog===next.dialog&&!next.dialog.startsWith('@')) {
    traversing=true;history.back();return;
  }
  if(!next.dialog&&!next.panel) {
    if(old.depth>0){traversing=true;history.go(-old.depth);}
    return;
  }
  const nested=(!old.dialog&&!!next.dialog)||(['settings','server'].includes(old.dialog)&&old.dialog!==next.dialog)||(next.dialog.startsWith('@')&&!old.dialog.startsWith('@'));
  const push=old.depth===0||nested;
  const depth=push?old.depth+1:old.depth;
  history[push?'pushState':'replaceState']({...history.state,sticktalkOverlay:{...next,depth,...(next.dialog.startsWith('@')?{parentDialog:old.dialog}:{})}},'');
}
