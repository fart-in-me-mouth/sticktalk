// Only the deliberately public /meta client key reaches this direct adapter.
// Server credentials/cookies are never forwarded to KLIPY.
export async function directSearch(key, query) {
  const url = new URL(`https://api.klipy.com/v2/${query ? 'search' : 'featured'}`);
  url.search = new URLSearchParams({key, client_key:'sticktalk', limit:'24', q:query.slice(0,200), media_filter:'nanowebm,nanomp4,tinywebm,tinymp4,mediumgif',contentfilter:'high'});
  const controller = new AbortController(), timer = setTimeout(()=>controller.abort(),8000);
  try {
    const r = await fetch(url,{credentials:'omit',redirect:'error',referrerPolicy:'no-referrer',signal:controller.signal});
    if(!r.ok) throw Error('GIF provider unavailable. Try again later.');
    const reader=r.body.getReader(), chunks=[]; let size=0;
    try {
      for (;;) { const {done,value}=await reader.read(); if(done)break; size+=value.length; if(size>1048576)throw Error('GIF response too large.'); chunks.push(value); }
    } finally { await reader.cancel(); }
    const bytes=new Uint8Array(size); let at=0; for(const c of chunks){bytes.set(c,at);at+=c.length;}
    const raw=JSON.parse(new TextDecoder().decode(bytes));
    if(!Array.isArray(raw.results)||raw.results.length>24)throw Error('Invalid GIF response.');
    const media=(formats,choices)=>{
      for(const [name,type] of choices){const m=formats[name]; if(!m)continue;
        const u=new URL(m.url);
        if(u.protocol!=='https:'||u.port||u.username||u.password||u.hash||!['static.klipy.com','static1.klipy.com','static2.klipy.com'].includes(u.hostname))throw Error('Invalid GIF media host.');
        const [width,height]=m.dims;
        if(!Number.isInteger(width)||!Number.isInteger(height)||width<1||height<1||width>4096||height>4096)throw Error('Invalid GIF dimensions.');
        return {url:u.href,width,height,content_type:type};
      } throw Error('Missing GIF rendition.');
    };
    return JSON.stringify({items:raw.results.map(g=>{
      if(!/^[a-zA-Z0-9_-]{1,128}$/.test(g.id))throw Error('Invalid GIF ID.');
      return {id:`klipy:${g.id}`, title:String(g.content_description||g.title||'GIF').slice(0,200),preview:media(g.media_formats,[['nanowebm','video/webm'],['nanomp4','video/mp4']]),send:media(g.media_formats,[['mediumgif','image/gif'],['tinywebm','video/webm'],['tinymp4','video/mp4']])};
    }),next_cursor:null});
  } finally {clearTimeout(timer);}
}
