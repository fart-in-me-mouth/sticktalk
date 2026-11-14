import os
import hashlib,json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
exe=(Path(os.environ.get('CARGO_TARGET_DIR', Path(__file__).resolve().parents[3] / 'target')) / 'release/sticktalk.exe')
sha=hashlib.sha256(exe.read_bytes()).hexdigest().upper()
for name in ['scroll-after-gpu.json','scroll-after-tiny.json']:
 p=root/name;d=json.loads(p.read_text(encoding='utf-8'));d['binary_sha256']=sha;p.write_text(json.dumps(d,indent=2),encoding='utf-8')
