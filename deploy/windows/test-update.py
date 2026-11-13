"""Real NSIS/helper integration. All HTTP is loopback; fixtures create no windows."""
import argparse, ctypes, hashlib, http.server, json, os, pathlib, shutil
import subprocess, tempfile, threading, time, urllib.parse, winreg

parser = argparse.ArgumentParser()
parser.add_argument('--key', required=True)
parser.add_argument('--target', required=True)
parser.add_argument('--nsis', required=True)
parser.add_argument('--real-client', action='store_true')
a = parser.parse_args()
repo = pathlib.Path(__file__).resolve().parents[2]
target = pathlib.Path(a.target).resolve()
key = pathlib.Path(a.key).resolve()
assert repo not in key.parents, 'Never use a key inside the repo'
release = target/'release'
work = target/'update-e2e'
work.mkdir(exist_ok=True)
installer_key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\StickTalk'
registry_keys = [installer_key, r'Software\Classes\sticktalk', r'Software\Classes\AppUserModelId\StickTalk.Desktop']
for name in registry_keys:
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, name):
            raise RuntimeError('Refusing to disturb an existing StickTalk installation')
    except FileNotFoundError:
        pass

flags = subprocess.CREATE_NO_WINDOW

def run(args, **kw):
    options = {'capture_output':True} if pathlib.Path(str(args[0])).name != 'update-runner.exe' else {'stdout':subprocess.DEVNULL,'stderr':subprocess.DEVNULL}
    result = subprocess.run([str(x) for x in args], creationflags=flags, text=True, **options, **kw)
    if result.returncode:
        raise RuntimeError(f'{args[0]} exited {result.returncode}: {result.stdout}\n{result.stderr}')
    return result

fixture_source = r'''
#![windows_subsystem = "windows"]
fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("STICKTALK_FIXTURE_LOG").unwrap());
    let _ = std::fs::write(root.join(format!("run-{}.txt",std::process::id())), VERSION);
    if FAIL { std::process::exit(3); }
    if let (Some(file), Ok(nonce)) = (std::env::var_os("STICKTALK_UPDATE_HEALTH_FILE"),std::env::var("STICKTALK_UPDATE_HEALTH_NONCE")) {
        std::fs::write(file,nonce).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_secs(180));
}
'''
payload = {}
for version, fail in [('0.0.1', False), ('0.0.2', False), ('0.0.3', True)]:
    folder = work/version
    folder.mkdir(exist_ok=True)
    source = folder/'fixture.rs'
    source.write_text('const VERSION: &str = "'+version+'";\nconst FAIL: bool = '+str(fail).lower()+';\n'+fixture_source.replace('#![windows_subsystem = "windows"]',''), encoding='utf-8')
    # Subsystem attribute must precede items.
    source.write_text('#![windows_subsystem = "windows"]\n'+source.read_text(), encoding='utf-8')
    run(['rustc', source, '-O', '-C', 'debuginfo=0', '-o', folder/'sticktalk.exe'])
    if a.real_client and version == '0.0.2': shutil.copy2(release/'sticktalk.exe',folder/'sticktalk.exe')
    shutil.copy2(release/'sticktalk-maintenance.exe', folder/'sticktalk-maintenance.exe')
    setup = folder/f'StickTalk-Setup-{version}.exe'
    run([a.nsis, f'/DCLIENT={folder / "sticktalk.exe"}', f'/DMAINTENANCE={folder / "sticktalk-maintenance.exe"}', f'/DOUTPUT={setup}', f'/DVERSION={version}', repo/'tools/release/installer.nsi'])
    payload[version] = (folder, setup)

state = {'manifest':b'', 'artifact':b'', 'ranges':[], 'checks':0, 'downloads':0}
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        route = urllib.parse.urlparse(self.path).path
        if route == '/api/update/latest':
            state['checks'] += 1
            data = state['manifest']
            self.send_response(200)
        elif route == '/api/update/artifact':
            state['downloads'] += 1
            data = state['artifact']
            offset = 0
            if self.headers.get('Range'):
                value = self.headers['Range']
                state['ranges'].append(value)
                offset = int(value.removeprefix('bytes=').removesuffix('-'))
                self.send_response(206)
                self.send_header('Content-Range',f'bytes {offset}-{len(data)-1}/{len(data)}')
                data = data[offset:]
            else:
                self.send_response(200)
        else:
            self.send_error(404)
            return
        self.send_header('Content-Length',str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args): pass

server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Handler)
thread = threading.Thread(target=server.serve_forever,daemon=True)
thread.start()
origin = f'http://127.0.0.1:{server.server_address[1]}'
stage = work/'stage'
stage.mkdir(exist_ok=True)
install = pathlib.Path(tempfile.mkdtemp(prefix='StickTalk update test '))
env = os.environ.copy()
env['STICKTALK_FIXTURE_LOG'] = str(work)
# Isolate preferences/logs/IPC from the owner's existing profile. The actual
# healthy client initializes its real UI/renderer/tray, but starts hidden.
if a.real_client:
    profile = work/'fresh-profile'
    (profile/'StickTalk').mkdir(parents=True,exist_ok=True)
    (profile/'StickTalk'/'desktop.json').write_text('true')
    env['LOCALAPPDATA'] = str(profile)
    env.pop('STICKTALK_AGENT_WINDOW',None)

runner = work/'update-runner.exe'
shutil.copy2(release/'sticktalk-maintenance.exe', runner)
report = {'healthy_payload':'real StickTalk client' if a.real_client else 'headless process fixture', 'origin':'loopback only', 'fixture_versions':['0.0.1','0.0.2','0.0.3'], 'checks':[]}

def publish(version):
    setup = payload[version][1]
    manifest = work/f'{version}-manifest.json'
    run([release/'sticktalk-package.exe','sign',key,setup,version,origin,manifest])
    state['manifest'] = manifest.read_bytes()
    state['artifact'] = setup.read_bytes()

def installed_version():
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,installer_key) as k:
        return winreg.QueryValueEx(k,'DisplayVersion')[0]

def stop_fixtures():
    time.sleep(0.2)  # allow the just-launched fixture to publish its PID marker
    # Open handles only for the PIDs recorded by our own fixture executable, and
    # check their image paths before termination; never stop an owner's client.
    kernel = ctypes.WinDLL('kernel32',use_last_error=True)
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel.QueryFullProcessImageNameW.argtypes = [ctypes.c_void_p,ctypes.c_uint32,ctypes.c_wchar_p,ctypes.POINTER(ctypes.c_uint32)]
    kernel.TerminateProcess.argtypes = [ctypes.c_void_p,ctypes.c_uint32]
    kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p,ctypes.c_uint32]
    markers = [(m,int(m.stem.removeprefix('run-'))) for m in work.glob('run-*.txt')]
    if (stage/'last-update.pid').exists(): markers.append((stage/'last-update.pid',int((stage/'last-update.pid').read_text())))
    for marker,pid in markers:
        h = kernel.OpenProcess(0x1000|0x0001|0x00100000,False,pid)
        if h:
            buffer = ctypes.create_unicode_buffer(32768)
            size = ctypes.c_uint32(len(buffer))
            if kernel.QueryFullProcessImageNameW(h,0,buffer,ctypes.byref(size)):
                if pathlib.Path(buffer.value).resolve() == (install/'sticktalk.exe').resolve():
                    kernel.TerminateProcess(h,0)
                    kernel.WaitForSingleObject(h,5000)
            kernel.CloseHandle(h)
        marker.unlink(missing_ok=True)

try:
    subprocess.run(f'"{payload["0.0.1"][1]}" /S /D={install}',creationflags=flags,check=True)
    assert installed_version() == '0.0.1'
    publish('0.0.2')
    # Prove real interrupted download resumes via Range rather than restarting.
    manifest = json.loads(json.loads(state['manifest'])['manifest'])
    offset = len(state['artifact'])//2
    (stage/(manifest['sha256']+'.part')).write_bytes(state['artifact'][:offset])
    run([release/'sticktalk-package.exe','stage',origin,'0.0.1',stage])
    assert state['ranges'] == [f'bytes={offset}-']
    report['checks'].append('signed manifest + HTTP Range resume + reverified staged hash')
    # A power loss during backup preparation must never restore partial bytes.
    (stage/'rollback').mkdir(exist_ok=True)
    (stage/'rollback'/'sticktalk.exe').write_bytes(b'incomplete backup')
    run([runner,'apply',origin,'0.0.1',stage,'0',install/'sticktalk.exe','--confirmed'],env=env,timeout=90)
    assert installed_version() == '0.0.2'
    assert hashlib.sha256((install/'sticktalk.exe').read_bytes()).digest() == hashlib.sha256((payload['0.0.2'][0]/'sticktalk.exe').read_bytes()).digest()
    assert not (stage/'pending.json').exists() and not (stage/'rollback').exists()
    report['checks'].append('0.0.1 -> 0.0.2 real NSIS apply + health acknowledgment + 15s alive; incomplete backup safely discarded')
    stop_fixtures()
    publish('0.0.3')
    run([release/'sticktalk-package.exe','stage',origin,'0.0.2',stage])
    failed = subprocess.run([str(runner),'apply',origin,'0.0.2',str(stage),'0',str(install/'sticktalk.exe'),'--confirmed'],env=env,creationflags=flags,timeout=90)
    assert failed.returncode != 0
    assert installed_version() == '0.0.2'
    assert (install/'sticktalk.exe').read_bytes() == (payload['0.0.2'][0]/'sticktalk.exe').read_bytes()
    assert (stage/'failed-update.json').exists()
    report['checks'].append('bad-startup 0.0.3 rolls back files + HKCU version + restarts 0.0.2')
    stop_fixtures()
    quarantined = subprocess.run([str(release/'sticktalk-package.exe'),'stage',origin,'0.0.2',str(stage)],creationflags=flags,capture_output=True)
    assert quarantined.returncode != 0
    report['checks'].append('failed release quarantined, not offered repeatedly')
    # Signature alteration must fail before downloading/executing anything.
    envelope = json.loads(state['manifest'])
    envelope['signature'] = 'A'*88
    state['manifest'] = json.dumps(envelope).encode()
    downloads = state['downloads']
    invalid = subprocess.run([str(release/'sticktalk-package.exe'),'stage',origin,'0.0.2',str(stage)],creationflags=flags,capture_output=True)
    assert invalid.returncode != 0 and downloads == state['downloads']
    report['checks'].append('tampered signature rejected before artifact request')
    # Simulate power loss after overwriting the app, before startup acknowledgment.
    backup = stage/'rollback'
    backup.mkdir()
    for f in install.iterdir():
        if f.is_file(): shutil.copy2(f,backup/f.name)
    (backup/'version.txt').write_text('0.0.2')
    shutil.copy2(payload['0.0.3'][0]/'sticktalk.exe',install/'sticktalk.exe')
    run([runner,'recover',install/'sticktalk.exe',stage,'0'],env=env,timeout=20)
    assert (install/'sticktalk.exe').read_bytes() == (payload['0.0.2'][0]/'sticktalk.exe').read_bytes()
    assert not backup.exists()
    report['checks'].append('interrupted setup restores backup before relaunch')
finally:
    stop_fixtures()
    server.shutdown()
    server.server_close()
    if (install/'Uninstall.exe').exists():
        uninstaller = work/'fixture-uninstall.exe'
        shutil.copy2(install/'Uninstall.exe',uninstaller)
        subprocess.run(f'"{uninstaller}" /S _?={install}',creationflags=flags,check=True)
    for name in registry_keys:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER,name):
                raise RuntimeError('Uninstall left registry key '+name)
        except FileNotFoundError: pass
    assert not any(install.iterdir()) if install.exists() else True
    if install.exists(): install.rmdir()
report['http_checks'] = state['checks']
report['artifact_requests'] = state['downloads']
(work/'results.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
