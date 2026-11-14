"""Actual compiler load for startup sanity; one target tree, unchanged source bytes."""
import json, os, pathlib, subprocess, time
ROOT=pathlib.Path(__file__).resolve().parent
WORKSPACE=ROOT.parents[3]
STOP=ROOT/'stop-load'
assert not STOP.exists(), 'remove only your own stale stop marker before invoking'
env=dict(os.environ, CARGO_TARGET_DIR='C:/Users/dwhee/.cargo-target/sticktalk-q1440', CARGO_INCREMENTAL='0', CMAKE_GENERATOR='Visual Studio 17 2022')
records=[]
try:
    while not STOP.exists():
        # Force a real optimized compile, not a cached cargo no-op. Only the
        # timestamp changes: native qualification code/executable is untouched.
        main=WORKSPACE/'crates/client/src/main.rs'
        os.utime(main, None)
        start=time.time()
        with (ROOT/f'compiler-load-{len(records)+1}.log').open('w') as out:
            code=subprocess.call(['cargo','build','--profile','server','-p','sticktalk','--bin','sticktalk','-j3'],cwd=WORKSPACE,env=env,stdout=out,stderr=subprocess.STDOUT)
        records.append(dict(started_unix=start,ended_unix=time.time(),code=code))
        (ROOT/'compiler-load.json').write_text(json.dumps(records,indent=2))
        if code: break
finally:
    (ROOT/'compiler-load.json').write_text(json.dumps(records,indent=2))
