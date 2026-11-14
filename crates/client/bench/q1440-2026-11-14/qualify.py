"""Fail-closed two-viewer qualification; preserves failed runs and every first bin."""
import argparse
import json
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent
WORKSPACE = ROOT.parents[3]

def load_present(run):
    return all(any(p.get('Name') == 'rustc' and p.get('CPU', 0) > 0
                   for p in run.get(key, []))
               for key in ['compiler_processes', 'compiler_processes_end'])

def metric_pass(m, run, role):
    bins = m['fps_by_second']
    strict = run['quality'] == '1080'
    return (len(bins) == run['seconds'] and m['seconds'] >= run['seconds']
            and sum(bins) == m['frames'] and m['frames'] / m['seconds'] >= 58
            and min(bins) >= (55 if strict else 45)
            and abs(m['drift_ms']) < (5 if strict else 20)
            and (role != 'sharer' or m['encoder_reconfigurations'] == 0))

def results():
    rows = []
    for directory in sorted(ROOT.iterdir()):
        if not (directory / 'run.json').exists():
            continue
        run = json.loads((directory / 'run.json').read_text(encoding='utf-8-sig'))
        row = {'name': directory.name, 'run': run, 'roles': {}, 'passed': True}
        if directory.name.startswith('final-e-load-'):
            row['passed'] &= load_present(run)
        for role in ['sharer', 'viewer_b', 'viewer_c']:
            path = directory / f'{role}.metrics.json'
            if not path.exists():
                row['passed'] = False
                continue
            m = json.loads(path.read_text(encoding='utf-8-sig'))
            bins = m['fps_by_second']
            avg = m['frames'] / m['seconds']
            okay = metric_pass(m, run, role)
            row['roles'][role] = {'average': avg, 'min': min(bins), 'first': bins[0],
                                  'drift_ms': m['drift_ms'], 'passed': okay,
                                  'reconfigs': m['encoder_reconfigurations']}
            row['passed'] &= okay
        exit_path = directory / 'exit.json'
        if exit_path.exists():
            row['exit'] = json.loads(exit_path.read_text())
            row['passed'] &= row['exit']['code'] == 0
        else:
            row['passed'] = False
        rows.append(row)
    (ROOT / 'results.json').write_text(json.dumps(rows, indent=2))
    table = ['# Hardware qualification (including failures)', '',
             '| Case | B avg/min/first fps | C avg/min/first fps | B/C drift ms | Reconfigs | Result |',
             '|---|---:|---:|---:|---:|---|']
    for row in rows:
        def fps(role):
            r = row['roles'].get(role)
            return f"{r['average']:.3f}/{r['min']}/{r['first']}" if r else 'missing'
        def drift(role):
            r = row['roles'].get(role)
            return f"{r['drift_ms']:+.3f}" if r else 'missing'
        table.append(f"| {row['name']} | {fps('viewer_b')} | {fps('viewer_c')} | "
                     f"{drift('viewer_b')}/{drift('viewer_c')} | "
                     f"{row['roles'].get('sharer', {}).get('reconfigs', 'missing')} | "
                     f"{'PASS' if row['passed'] else 'FAIL'} |")
    (ROOT / 'acceptance-table.md').write_text('\n'.join(table) + '\n')
    required = ['final-a-1080'] + [f'final-b-startup-{i}' for i in range(1, 6)]
    required += ['final-c-1440-long', 'final-d-wgc-1440', 'final-d-wgc-source']
    required += [f'final-e-load-{i}' for i in range(1, 6)]
    present = {r['name']: r['passed'] for r in rows}
    status = {'complete': all(present.get(n, False) for n in required),
              'missing_or_failed': [n for n in required if not present.get(n, False)]}
    (ROOT / 'qualification-status.json').write_text(json.dumps(status, indent=2))
    return rows

def case(name, source, quality, seconds):
    begin = time.time()
    with (ROOT / f'{name}.runner.log').open('w') as log:
        code = subprocess.call(['pwsh', '-NoProfile', '-File', str(ROOT / 'run-acceptance.ps1'),
                               '-Name', name, '-Source', source, '-Quality', quality,
                               '-Seconds', str(seconds)], cwd=WORKSPACE, stdout=log, stderr=subprocess.STDOUT)
    directory = ROOT / name
    if directory.exists():
        (directory / 'exit.json').write_text(json.dumps({'code': code, 'wall_seconds': time.time()-begin}))
    rows = results()
    row = next((r for r in rows if r['name'] == name), None)
    print(json.dumps(row), flush=True)
    return bool(row and row['passed'])

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--case')
    parser.add_argument('--source', default='synthetic')
    parser.add_argument('--quality', default='1440')
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--final', action='store_true')
    args = parser.parse_args()
    if args.case:
        sys.exit(0 if case(args.case, args.source, args.quality, args.seconds) else 1)
    if args.final:
        schedule = [('final-a-1080', 'synthetic', '1080', 180)]
        schedule += [(f'final-b-startup-{i}', 'synthetic', '1440', 60) for i in range(1, 6)]
        schedule += [('final-c-1440-long', 'synthetic', '1440', 600),
                     ('final-d-wgc-1440', 'wgc-motion', '1440', 180),
                     ('final-d-wgc-source', 'wgc-motion', 'source', 180)]
        for spec in schedule:
            if not case(*spec):
                sys.exit(1)
    else:
        results()
