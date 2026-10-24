#!/usr/bin/env python3
"""Real native capture / process-kill harness. Uses an isolated temp directory.
Run --seconds 18000 for the required wall-clock five-hour hardware soak.
No destructive disk-full or power-loss simulation is performed on user data.
"""
import argparse, hashlib, json, os, pathlib, signal, sqlite3, subprocess, tempfile, time
p = argparse.ArgumentParser()
p.add_argument('--seconds', type=int, default=20)
p.add_argument('--kill-after', type=float)
p.add_argument('--mic')
p.add_argument('--playback')
p.add_argument('--report', type=pathlib.Path, required=True)
a = p.parse_args()
repo = pathlib.Path(__file__).resolve().parents[1]
build = subprocess.run(['cargo','test','--manifest-path',str(repo/'src-tauri/Cargo.toml'),'--lib','--no-run','--message-format=json'],check=True,text=True,capture_output=True)
exe = next(json.loads(line)['executable'] for line in build.stdout.splitlines() if line.startswith('{') and json.loads(line).get('executable') and json.loads(line).get('target',{}).get('name') == 'reflow_lib')
root = pathlib.Path(tempfile.mkdtemp(prefix='reflow-meeting-soak-'))
env = {**os.environ,'REFLOW_MEETING_TEST_DIR':str(root),'REFLOW_MEETING_SOAK_SECONDS':str(a.seconds)}
if a.mic: env['REFLOW_MEETING_MIC'] = a.mic
if a.playback: env['REFLOW_MEETING_PLAYBACK'] = a.playback
log = (root/'native.log').open('w')
child = subprocess.Popen([exe,'meetings::tests::native_soak','--exact','--ignored','--nocapture'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
start=time.monotonic(); started=None; killed_elapsed=None; snapshots=[]; acknowledged={}
while child.poll() is None:
    if started is None and 'MEETING_STARTED' in (root/'native.log').read_text(): started=time.monotonic()
    rss=None; cpu_s=None
    try:
        stat=pathlib.Path(f'/proc/{child.pid}/stat').read_text().rsplit(') ',1)[1].split()
        cpu_s=(int(stat[11])+int(stat[12]))/os.sysconf('SC_CLK_TCK')
    except OSError: pass
    try:
        status=pathlib.Path(f'/proc/{child.pid}/status').read_text()
        rss=int(next(line.split()[1] for line in status.splitlines() if line.startswith('VmRSS:')))*1024
    except (OSError,StopIteration): pass
    db=root/'meetings.db'
    if db.exists():
        try:
            with sqlite3.connect(f'file:{db}?mode=ro',uri=True,timeout=.1) as c:
                committed=[(r[0],r[1],r[2],r[3],r[4]) for r in c.execute('SELECT track,seq,start_frame,frames,checksum FROM meeting_chunk')]
                acknowledged.update({(r[0],r[1]):r for r in committed})
                snapshots.append({'at_s':time.monotonic()-start,'rss_bytes':rss,'process_cpu_s':cpu_s,'disk_bytes':sum(f.stat().st_size for f in root.glob('meetings.db*')),'chunks':len(committed)})
        except sqlite3.Error: pass
    if a.kill_after and started is not None and time.monotonic()-started >= a.kill_after:
        killed_elapsed=time.monotonic()-start
        os.killpg(child.pid,signal.SIGKILL); break
    if started is None and time.monotonic()-start>30: os.killpg(child.pid,signal.SIGKILL); raise RuntimeError('Capture did not start; inspect '+str(root/'native.log'))
    time.sleep(.25)
child.wait();log.close()
if a.kill_after:
    recovered=subprocess.run([exe,'meetings::tests::recover_test_dir','--exact','--ignored','--nocapture'],env=env,text=True,capture_output=True)
    (root/'recovery.log').write_text(recovered.stdout+recovered.stderr)
    if recovered.returncode: raise RuntimeError('Recovery failed; see '+str(root))
elif child.returncode: raise RuntimeError('Native capture failed; inspect '+str(root/'native.log'))
with sqlite3.connect(root/'meetings.db') as c:
    integrity=c.execute('PRAGMA integrity_check').fetchone()[0]
    m=json.loads(c.execute('SELECT state FROM meeting LIMIT 1').fetchone()[0])
    chunks=list(c.execute('SELECT track,seq,start_frame,frames,checksum,pcm FROM meeting_chunk ORDER BY track,seq'))
    assert integrity=='ok'
    for track,seq,at,frames,checksum,pcm in chunks:
        assert len(pcm)==frames*2 and hashlib.sha256(pcm).hexdigest()==checksum
    saved={(r[0],r[1]):r[:5] for r in chunks}
    assert all(saved.get(k)==v for k,v in acknowledged.items()), 'Previously observed committed chunks disappeared'
last_end=[max((r[2]+r[3] for r in chunks if r[0]==t),default=0)/16000 for t in range(2)]
capture_wall=time.monotonic()-started if started is not None else None
report={'date_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'test_dir':str(root),'platform':os.uname().sysname,'duration_requested_s':a.seconds,'killed_after_s':a.kill_after,'capture_wall_s':capture_wall,'format':'16000 Hz mono PCM16 per track','tracks':2,'integrity':integrity,'checksums_valid':True,'previous_commits_preserved':True,'last_audio_end_s':last_end,'rpo_upper_bound_s':max(0,killed_elapsed-min(last_end)) if killed_elapsed is not None else None,'meeting':m,'telemetry':snapshots,'scope':'Native PipeWire with explicit selected route; virtual test source is not physical-microphone validation. Process-kill only; no power-loss, disconnect or marker-quality claim.'}
a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2))
print(json.dumps({k:v for k,v in report.items() if k not in ('meeting','telemetry')},indent=2))
