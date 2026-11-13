export function swipeAction(panel, dx, dy) {
  if(Math.abs(dx)<80 || Math.abs(dx)<=Math.abs(dy)*1.25)return null;
  if(panel==='left')return dx<0?'panel:':null;
  if(panel==='right')return dx>0?'panel:':null;
  return dx>0?'panel:left':'panel:right';
}
let previous='', emit, traversing=false;
function state() {
  const shell=document.querySelector('.live-shell');
  return {dialog:document.querySelector('.live-modal')?.dataset.dialog??'',
    panel:shell?.classList.contains('panel-left')?'left':shell?.classList.contains('panel-right')?'right':''};
}
export function installOverlayHistory(send) {
  emit=send;
  history.replaceState({...history.state,sticktalkOverlay:{dialog:'',panel:'',depth:0}},'');
  addEventListener('popstate',()=>{
    const next=history.state?.sticktalkOverlay??{dialog:'',panel:'',depth:0};
    previous=JSON.stringify({dialog:next.dialog,panel:next.panel});
    traversing=false;
    if(!document.querySelector('.live-shell'))return;
    if(next.dialog)emit(`dialog:${next.dialog}`);
    else if(next.panel){emit('close');emit(`panel:${next.panel}`);}
    else emit('close');
  });
}
export function syncOverlayHistory() {
  const next=state(), key=JSON.stringify(next);
  if(key===previous || traversing)return;
  const old=history.state?.sticktalkOverlay??{dialog:'',panel:'',depth:0};
  previous=key;
  if(!next.dialog&&!next.panel) {
    if(old.depth>0){traversing=true;history.go(-old.depth);}
    return;
  }
  const nested=(!old.dialog&&!!next.dialog)||(['settings','server'].includes(old.dialog)&&old.dialog!==next.dialog);
  const push=old.depth===0||nested;
  const depth=push?old.depth+1:old.depth;
  history[push?'pushState':'replaceState']({...history.state,sticktalkOverlay:{...next,depth}},'');
}
