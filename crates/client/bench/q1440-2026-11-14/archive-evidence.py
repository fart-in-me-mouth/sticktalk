"""Archive full traces only after every owned writer has exited; verify before unlink."""
from pathlib import Path
import hashlib,json,zipfile
ROOT=Path(__file__).resolve().parent
for required in ['compiler-load.json','doc-tests.exit','fmt-final.exit','gate-tests-final.exit']:
    assert (ROOT/required).exists(), f'Writer still active or validation missing: {required}'
assert not (ROOT/'trace-logs.zip').exists(), 'Refusing to replace an existing evidence archive'
paths=sorted(ROOT.rglob('*.log'))
assert paths, 'No raw logs to archive'
manifest=[]
for path in paths:
    assert path.resolve().is_relative_to(ROOT), 'Refusing an out-of-evidence path'
    blob=path.read_bytes()
    manifest.append({'path':path.relative_to(ROOT).as_posix(),'bytes':len(blob),'sha256':hashlib.sha256(blob).hexdigest()})
with zipfile.ZipFile(ROOT/'trace-logs.zip','w',zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
    for path in paths:archive.write(path,path.relative_to(ROOT).as_posix())
with zipfile.ZipFile(ROOT/'trace-logs.zip') as archive:
    assert archive.testzip() is None
    for entry in manifest:
        assert hashlib.sha256(archive.read(entry['path'])).hexdigest()==entry['sha256']
(ROOT/'log-archive.json').write_text(json.dumps({'files':manifest,'zip_bytes':(ROOT/'trace-logs.zip').stat().st_size,'zip_sha256':hashlib.sha256((ROOT/'trace-logs.zip').read_bytes()).hexdigest()},indent=2))
# Only verified .log files under this evidence directory, never a recursive
# directory deletion and never paths enumerated in a different shell.
for path in paths:
    assert path.resolve().is_relative_to(ROOT) and path.suffix=='.log'
    path.unlink()
print(f'Archived and verified {len(paths)} log files')
