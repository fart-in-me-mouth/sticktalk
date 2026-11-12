import hashlib,json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
exe=Path('C:/Users/dwhee/.cargo-target/sticktalk-client/release/sticktalk.exe')
sha=hashlib.sha256(exe.read_bytes()).hexdigest().upper()
for name in ['scroll-after-gpu.json','scroll-after-tiny.json']:
 p=root/name;d=json.loads(p.read_text());d['binary_sha256']=sha;p.write_text(json.dumps(d,indent=2),encoding='utf-8')
