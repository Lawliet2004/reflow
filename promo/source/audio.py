"""Generate scene narration, original score, timed captions, and a stereo mix."""
import asyncio, json, subprocess, wave, math, re
from pathlib import Path
import numpy as np
import edge_tts

ROOT=Path(__file__).resolve().parents[1]
STORY=json.loads((ROOT/'source/story.json').read_text())
SR=48000

def probe(p):
    return float(subprocess.check_output(['ffprobe','-v','error','-show_entries','format=duration','-of','csv=p=0',str(p)]))

async def narrate():
    captions=[]
    for i,s in enumerate(STORY):
        dest=ROOT/f'assets/voice-{i:02d}.mp3'
        boundaries=[]
        meta=ROOT/f'assets/voice-{i:02d}.json'
        if not dest.exists() or not meta.exists():
            talk=edge_tts.Communicate(s['voice'],'en-US-AndrewNeural',rate='+6%',boundary='WordBoundary')
            with dest.open('wb') as f:
                async for chunk in talk.stream():
                    if chunk['type']=='audio': f.write(chunk['data'])
                    elif chunk['type']=='WordBoundary': boundaries.append(chunk)
            meta.write_text(json.dumps(boundaries))
        else: boundaries=json.loads(meta.read_text())
        duration=probe(dest)
        available=s['end']-s['start']-0.55
        speed=max(1.0,duration/available)
        wav=ROOT/f'assets/voice-{i:02d}.wav'
        subprocess.run(['ffmpeg','-y','-v','error','-i',str(dest),'-af',f'atempo={speed},afade=t=out:st={max(0,duration/speed-.12)}:d=0.12','-ar',str(SR),'-ac','1',str(wav)],check=True)
        s['voice_start']=s['start']+.22
        s['voice_duration']=probe(wav)
        # Short word-aligned subtitle groups: easy to read on small displays.
        # Restore punctuation from the script without changing spoken timing.
        source_words=s['voice'].split()
        if len(source_words)==len(boundaries):
            for b,w in zip(boundaries,source_words): b['text']=w
        sentences=[];current=[]
        for b in boundaries:
            current.append(b)
            if re.search(r'[.!?]$',b['text']):
                sentences.append(current);current=[]
        if current:sentences.append(current)
        groups=[]
        for sentence in sentences:
            count=math.ceil(len(sentence)/7)
            for part in range(count):
                groups.append(sentence[round(part*len(sentence)/count):round((part+1)*len(sentence)/count)])
        for group in groups:
            start=s['voice_start']+group[0]['offset']/1e7/speed
            end=s['voice_start']+(group[-1]['offset']+group[-1]['duration'])/1e7/speed
            text=' '.join(x['text'] for x in group)
            captions.append({'start':start,'end':min(s['end']-.1,end+.08),'text':text})
        print(f'Voice {i+1}/10: {duration:.2f}s, fitted {s["voice_duration"]:.2f}s',flush=True)
    for i in range(len(captions)-1):
        captions[i]['end']=min(captions[i]['end'],captions[i+1]['start']-.015)
    (ROOT/'assets/captions.json').write_text(json.dumps(captions))
    (ROOT/'assets/timeline.json').write_text(json.dumps(STORY))
    return captions

def make_score():
    # Original 96 BPM electronic / felt-key score. No third-party music samples.
    n=90*SR
    score=np.zeros((n,2),dtype=np.float32)
    rng=np.random.default_rng(29)
    beat=60/96
    chords=[(50,57,60,64),(46,53,57,60),(53,60,64,67),(48,55,59,62)]
    def add(start,sound,vol=1,pan=0):
        at=int(start*SR)
        if at>=n: return
        length=min(len(sound),n-at)
        score[at:at+length,0]+=sound[:length]*vol*math.sqrt((1-pan)/2)
        score[at:at+length,1]+=sound[:length]*vol*math.sqrt((1+pan)/2)
    def note(midi,duration,kind='key'):
        t=np.arange(int(SR*duration))/SR
        f=440*2**((midi-69)/12)
        if kind=='pad':
            s=(np.sin(2*np.pi*f*t)+.22*np.sin(2*np.pi*f*2*t)+.12*np.sin(2*np.pi*f*1.002*t))
            e=np.minimum(t/.7,1)*np.minimum((duration-t)/1.3,1)*.14
        elif kind=='bass':
            s=np.sin(2*np.pi*f*t)+.1*np.sin(2*np.pi*f*2*t)
            e=np.minimum(t/.015,1)*np.exp(-t*3)*.4
        else:
            s=np.sin(2*np.pi*f*t)+.25*np.sin(2*np.pi*f*2*t)*np.exp(-t*3)+.08*np.sin(2*np.pi*f*3*t)
            e=np.minimum(t/.009,1)*np.exp(-t*2.5)*np.minimum((duration-t)/.1,1)*.3
        return (s*e).astype(np.float32)
    for bar in range(36):
        at=bar*beat*4
        chord=chords[bar%4]
        for j,m in enumerate(chord): add(at,note(m+12,beat*4+.8,'pad'),.3,(j-1.5)/3)
        for k in range(8):
            m=chord[[0,2,1,3,2,1,3,2][k]]+24
            if at+k*beat/2>88: continue
            add(at+k*beat/2,note(m,1.5),.17 if bar<3 else .24,(-1 if k%2 else 1)*.45)
        if at>7:
            for k in [0,2]:
                add(at+k*beat,note(chord[0]-12,.65,'bass'),.43)
                t=np.arange(int(SR*.24))/SR
                kick=np.sin(2*np.pi*(47*t+55*.028*(1-np.exp(-t/.028))))*np.exp(-t*24)*.3
                add(at+k*beat,kick,.55)
            for k in [1,3]:
                t=np.arange(int(SR*.16))/SR
                noise=rng.normal(0,1,len(t)).astype(np.float32)
                noise[1:]-=noise[:-1]*.88
                add(at+k*beat,noise*np.exp(-t*45)*.06,.3,.12)
            if at<83:
                for k in range(8):
                    t=np.arange(int(SR*.065))/SR
                    noise=rng.normal(0,1,len(t)).astype(np.float32)
                    noise[1:]-=noise[:-1]*.95
                    add(at+k*beat/2,noise*np.exp(-t*85)*.027,.45,(-1 if k%2 else 1)*.5)
    # Soft impact + airy sweep into each chapter.
    for s in STORY[1:]:
        t=np.arange(int(SR*.6))/SR
        noise=rng.normal(0,1,len(t)).astype(np.float32)
        filtered=np.convolve(noise,np.ones(20)/20,mode='same')
        add(s['start']-.4,filtered*np.sin(np.pi*t/.6)**2*.085,.4,0)
        add(s['start'],note(76,.9),.11,-.3)
    envelope=np.minimum(np.arange(n)/SR/2,1)*np.minimum((90-np.arange(n)/SR)/2.3,1)
    score*=envelope[:,None]
    return score

def write_wav(p,data):
    data=np.clip(data,-1,1)
    with wave.open(str(p),'wb') as w:
        w.setnchannels(2 if data.ndim==2 else 1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes((data*32767).astype('<i2').tobytes())

async def main():
    captions=await narrate()
    music=make_score()
    voice=np.zeros(90*SR,dtype=np.float32)
    duck=np.ones(90*SR,dtype=np.float32)
    for i,s in enumerate(STORY):
        with wave.open(str(ROOT/f'assets/voice-{i:02d}.wav'),'rb') as w:
            a=np.frombuffer(w.readframes(w.getnframes()),dtype='<i2').astype(np.float32)/32768
        a*=.65/max(.01,float(np.max(np.abs(a))))
        at=int(s['voice_start']*SR)
        voice[at:at+len(a)]+=a
        begin=max(0,at-int(.12*SR));end=min(len(duck),at+len(a)+int(.2*SR))
        duck[begin:end]=.50
    # Smooth automatic music ducking under the voice.
    duck=np.convolve(duck[::480],np.ones(15)/15,mode='same')
    duck=np.interp(np.arange(len(voice))/480,np.arange(len(duck)),duck)
    music*=1.4
    write_wav(ROOT/'assets/music-original.wav',music)
    write_wav(ROOT/'assets/narration.wav',voice)
    mix=music*duck[:,None]+voice[:,None]
    write_wav(ROOT/'assets/mix.wav',mix)
    def stamp(t):
        ms=round(t*1000);return f'{ms//3600000:02}:{ms//60000%60:02}:{ms//1000%60:02},{ms%1000:03}'
    (ROOT/'output/reflow-promo.en.srt').write_text('\n\n'.join(f'{i+1}\n{stamp(c["start"])} --> {stamp(c["end"])}\n{c["text"]}' for i,c in enumerate(captions))+'\n')
    print('Original music, narration, captions, and mix saved.',flush=True)

if __name__=='__main__': asyncio.run(main())
