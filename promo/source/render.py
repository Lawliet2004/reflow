"""Deterministic 90-second Reflow film, 1920x1080 / 30 fps.

Product screenshots come from the unmodified React UI with sample data.
Workflow graphics are illustrative. All motion, typography and score are original.
"""
import argparse, functools, json, math, subprocess, time
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont, ImageFilter
import numpy as np

ROOT=Path(__file__).resolve().parents[1]
W,H,FPS=1920,1080,30
PAPER='#f4efe6'; SHEET='#fbf8f2'; INK='#29231e'; MUTED='#776c60'
LINE='#d9cebd'; BLUE='#2a6690'; SOFT='#e4edf0'; GREEN='#3d7955'; DARK='#1a2025'
FONTS={
 'serif':'/usr/share/fonts/opentype/urw-base35/P052-Roman.otf',
 'italic':'/usr/share/fonts/opentype/urw-base35/P052-Italic.otf',
 'sans':'/usr/share/fonts/truetype/ibm-plex/IBMPlexSans-Regular.ttf',
 'medium':'/usr/share/fonts/truetype/ibm-plex/IBMPlexSans-Medium.ttf',
 'bold':'/usr/share/fonts/truetype/space-grotesk/SpaceGrotesk-Bold.ttf',
 'mono':'/usr/share/fonts/truetype/ibm-plex/IBMPlexMono-Regular.ttf'}

@functools.lru_cache(256)
def font(size,face='sans'):return ImageFont.truetype(FONTS[face],round(size))
def ease(x):
 x=max(0,min(1,x));return 1-(1-x)**3
def smooth(x):
 x=max(0,min(1,x));return x*x*(3-2*x)
def text(im,xy,s,size=32,fill=INK,face='sans',anchor=None):
 ImageDraw.Draw(im).text(xy,s,font=font(size,face),fill=fill,anchor=anchor,spacing=8)
def width(s,size,face='sans'):return font(size,face).getlength(s)
def wrap(s,size,maxw,face='sans'):
 words=s.split();lines=[];line=''
 for word in words:
  trial=(line+' '+word).strip()
  if width(trial,size,face)>maxw and line:lines.append(line);line=word
  else:line=trial
 if line:lines.append(line)
 return lines
def para(im,xy,s,size=32,maxw=750,fill=INK,face='sans',leading=1.4):
 x,y=xy
 for block in s.split('\n'):
  for row in wrap(block,size,maxw,face):text(im,(x,y),row,size,fill,face);y+=size*leading
  if not block:y+=size*leading
 return y
def rounded(im,box,fill=SHEET,r=24,outline=None,lw=1):
 ImageDraw.Draw(im).rounded_rectangle(tuple(round(v) for v in box),radius=r,fill=fill,outline=outline,width=lw)
def line(im,coords,fill=LINE,lw=2):ImageDraw.Draw(im).line(coords,fill=fill,width=lw)
def circle(im,xy,r,fill,outline=None,lw=2):
 x,y=xy;ImageDraw.Draw(im).ellipse((x-r,y-r,x+r,y+r),fill=fill,outline=outline,width=lw)
def check(im,x,y,fill=GREEN,size=16):line(im,[(x-size*.5,y),(x-size*.1,y+size*.4),(x+size*.7,y-size*.55)],fill,4)
def arrow(im,x,y,length=75,fill=BLUE):
 line(im,[(x,y),(x+length,y)],fill,3);line(im,[(x+length-11,y-9),(x+length,y),(x+length-11,y+9)],fill,3)

rng=np.random.default_rng(7)
noise=rng.normal(0,.6,(H,W,1))
base=np.array([244,239,230])[None,None,:]
BG=Image.fromarray(np.clip(base+noise,0,255).astype('uint8'),'RGB')
DBG=Image.new('RGB',(W,H),DARK)
logo=Image.open(ROOT.parent/'src/assets/logo.png').convert('RGBA')
SCREENS={}
for name in ['home','home-empty','notes','history','dictionary','settings']:
 p=ROOT/f'assets/{name}.png'
 if p.exists():
  a=Image.open(p).convert('RGB');SCREENS[name]=a.crop((0,36,a.width,a.height))
STORY=json.loads((ROOT/'source/story.json').read_text())
CAPTIONS=[]

def logo_at(im,x,y,size):im.paste(logo.resize((size,size),Image.Resampling.LANCZOS),(round(x),round(y)),logo.resize((size,size),Image.Resampling.LANCZOS))
def brand(im,index,dark=False):
 col=SHEET if dark else INK
 logo_at(im,92,49,39);text(im,(145,48),'Reflow',31,col,'medium')
 text(im,(1824,55),f'{index+1:02d} / 10',21,'#bcb8b0' if dark else MUTED,'mono',anchor='ra')
 line(im,[(94,105),(1826,105)],'#3b4248' if dark else LINE,1)
def title(im,s,local,sub=None):
 off=round((1-ease(local/.7))*28)
 text(im,(96,142+off),s,83,INK,'serif')
 if sub:text(im,(100,249+off),sub,29,MUTED)

@functools.lru_cache(80)
def screen_asset(name,w):
 src=SCREENS[name];h=round(src.height*w/src.width)
 out=src.resize((w,h),Image.Resampling.LANCZOS).convert('RGBA')
 mask=Image.new('L',out.size,0);ImageDraw.Draw(mask).rounded_rectangle((0,0,w-1,h-1),radius=20,fill=255)
 out.putalpha(mask)
 shadow=Image.new('RGBA',(w+90,h+100))
 ImageDraw.Draw(shadow).rounded_rectangle((35,30,w+55,h+50),radius=23,fill=(39,29,17,40))
 shadow=shadow.filter(ImageFilter.GaussianBlur(16));shadow.alpha_composite(out,(45,28))
 return shadow
def screen(im,name,x,y,w):
 asset=screen_asset(name,round(w));im.paste(asset,(round(x)-45,round(y)-28),asset)
def waveform(im,x,y,t,w=450,active=True,color=BLUE,bars=45,height=72):
 for k in range(bars):
  v=(.32+.68*abs(math.sin(k*.69+t*4)*math.cos(k*.21-t*2))) if active else .10
  v*=.45+.55*math.sin(math.pi*(k+.5)/bars)**.7
  hh=max(5,height*v)
  rounded(im,(x+k*w/bars,y-hh/2,x+k*w/bars+max(3,w/bars*.47),y+hh/2),color,3)
def pill(im,x,y,label,active=False,size=25):
 ww=width(label,size)+42
 rounded(im,(x,y,x+ww,y+49),INK if active else '#e9e1d5',12)
 text(im,(x+21,y+7),label,size,SHEET if active else MUTED)
 return ww
def mic(im,x,y,scale=1,color=INK):
 d=ImageDraw.Draw(im);s=scale
 d.rounded_rectangle((x-9*s,y-20*s,x+9*s,y+7*s),radius=9*s,outline=color,width=max(2,round(3*s)))
 d.arc((x-17*s,y-15*s,x+17*s,y+15*s),0,180,fill=color,width=max(2,round(3*s)))
 line(im,[(x,y+15*s),(x,y+24*s)],color,round(3*s));line(im,[(x-8*s,y+24*s),(x+8*s,y+24*s)],color,round(3*s))

def shot0(t):
 im=BG.copy();brand(im,0)
 second=t>=3.2
 a=ease((t-3.2 if second else t)/.7)
 x=98+int((1-a)*35)
 text(im,(x,212),'Let your words' if second else 'Your ideas',126,INK,'serif')
 text(im,(x,355),'keep up.' if second else 'move fast.',126,BLUE,'italic')
 text(im,(103,562),'Less friction between thinking and writing.',33,MUTED)
 # An abstract thought becomes a composed manuscript.
 rounded(im,(1090,232,1780,795),SHEET,22,LINE)
 text(im,(1140,275),'A thought, taking shape.',34,INK,'serif')
 line(im,[(1140,338),(1730,338)],LINE,1)
 if not second:
  fragments=['an idea','the next chapter','a better way','the right words']
  for k,s in enumerate(fragments):
   px=1150+int(math.sin(t*.9+k)*22);py=391+k*76
   text(im,(px,py),s,41,BLUE if k==int(t)%4 else '#b1a696','italic')
 else:
  words='Make space for the ideas that arrive when you stop watching the cursor.'
  n=int(min(1,(t-3.2)/2.0)*len(words))
  para(im,(1140,385),words[:n],43,550,INK,'serif',1.45)
  waveform(im,1160,724,t,500,height=48)
 return im

def shot1(t):
 im=BG.copy();brand(im,1)
 logo_at(im,98,204,125)
 text(im,(96,365),'Meet Reflow.',100,INK,'serif')
 para(im,(101,511),'Your voice.\nYour words.\nYour computer.',43,520,INK,'sans',1.5)
 line(im,[(102,747),(527,747)],LINE,2)
 text(im,(101,780),'Local-first desktop dictation',26,BLUE,'medium')
 screen(im,'home',725+int((1-ease(t/.9))*100),204,1080)
 return im

def shot2(t):
 im=BG.copy();brand(im,2);title(im,'Hold. Speak. Release.',t,'From your thought to the app you’re already using.')
 phase=0 if t<1.3 else 1 if t<4.7 else 2
 state=0 if t<4.7 else 1 if t<5.7 else 2
 steps=['01  Hold your shortcut','02  Speak naturally','03  Release to insert']
 for k,s in enumerate(steps):
  xx=100+k*577
  rounded(im,(xx,325,xx+550,396),INK if k==phase else '#e9e1d5',13)
  text(im,(xx+25,343),s,28,SHEET if k==phase else MUTED,'medium')
 rounded(im,(280,443,1640,806),SHEET,22,LINE)
 text(im,(328,475),'New message',24,MUTED)
 line(im,[(328,524),(1591,524)],LINE,1)
 text(im,(328,548),'Subject: Tomorrow’s review',27,INK,'medium')
 if state==2:
  para(im,(328,612),'The launch plan is ready. Let’s review the final details together tomorrow morning.',43,1200,INK,'serif',1.42)
  rounded(im,(1438,738,1590,784),SOFT,10);text(im,(1460,746),'Inserted',23,BLUE);check(im,1560,761,BLUE,10)
 else:
  text(im,(328,614),'|',45,BLUE,'serif')
  text(im,(360,620),'Your next sentence starts with your voice.',34,'#a89d90','italic')
 # Reflow HUD. Recognition happens after release, never as fake live text.
 rounded(im,(629,832,1291,916),DARK,28)
 mic(im,675,870,.7,SHEET)
 waveform(im,718,874,t,270,state==0,'#e8e1d5',30,44)
 text(im,(1026,854),['Listening','Transcribing','Ready'][state],26,SHEET,'medium')
 text(im,(100,921),'Illustrative workflow · shortcut shown: Ctrl + Shift + Space',19,MUTED)
 return im

def shot3(t):
 im=BG.copy();brand(im,3);title(im,'Your voice. A little clearer.',t,'Keep the idea. Lose the rough edges.')
 a=ease((t-1.7)/1.0)
 for x in [100,1010]:rounded(im,(x,344,x+810,754),SHEET,22,LINE)
 text(im,(142,376),'AS SPOKEN',22,MUTED,'mono')
 raw='Um, the launch plan is, is ready and uh let’s review the final details together tomorrow morning.'
 para(im,(142,445),raw,43,710,INK,'serif',1.48)
 text(im,(1052,376),'WITH REFLOW',22,BLUE,'mono')
 if a>0:
  layer=Image.new('RGBA',im.size)
  para(layer,(1052,445+int((1-a)*20)),'The launch plan is ready. Let’s review the final details together tomorrow morning.',43,710,INK,'serif',1.48)
  layer.putalpha(layer.getchannel('A').point(lambda v:int(v*a)));im.paste(layer,(0,0),layer)
  check(im,1760,405,BLUE,16)
 arrow(im,932,548,50)
 xpos=100
 for k,s in enumerate(['Original','Minimal','Natural','Structured']):
  xpos+=pill(im,xpos,805,s,k==(0 if t<1.7 else 2),27)+15
 text(im,(104,884),'Natural and Structured cleanup use an optional local language model.',24,MUTED)
 return im

def shot4(t):
 im=BG.copy();brand(im,4);title(im,'One voice. Every context.',t,'Give each thought the right shape.')
 labels=['Email','Chat','Notes'];phase=min(2,int(t/3.2))
 examples=[('A little more considered.','Hi Maya,\n\nThe draft is ready for your review. Could you share your feedback tomorrow?\n\nThanks, Alex'),('A little more conversational.','Hey Maya! The draft’s ready. Can you take a look tomorrow?'),('A little more organized.','Next steps\n\n• Review the latest draft\n• Collect feedback\n• Share the final version')]
 for k,s in enumerate(labels):
  yy=353+k*139
  rounded(im,(100,yy,570,yy+105),INK if k==phase else '#e9e1d5',16)
  text(im,(133,yy+24),s,40,SHEET if k==phase else MUTED,'serif')
  if k==phase:arrow(im,492,yy+53,33,SHEET)
 rounded(im,(650,343,1820,853),SHEET,24,LINE)
 text(im,(698,377),examples[phase][0],28,BLUE,'medium')
 line(im,[(698,431),(1770,431)],LINE,1)
 y=467
 for block in examples[phase][1].split('\n'):
  y=para(im,(698,y),block,41,1040,INK,'serif',1.32) if block else y+20
 text(im,(100,903),'Illustrative writing styles',21,MUTED)
 return im

def shot5(t):
 im=BG.copy();brand(im,5);title(im,'Make it your language.',t,'Your vocabulary. Your shortcuts. Your way of working.')
 pairs=[('git hub','GitHub'),('vs code','VS Code'),('my signature','Best, Alex')]
 for k,(a,b) in enumerate(pairs):
  yy=365+k*145
  text(im,(102,yy),a,39,MUTED,'serif')
  arrow(im,375,yy+28,58)
  aa=ease((t-k*.65)/.65)
  if aa>0:text(im,(475+int((1-aa)*20),yy),b,42,BLUE,'serif')
 text(im,(103,838),'Dictionary + spoken snippets',27,INK,'medium')
 screen(im,'dictionary',880,342,925)
 return im

def shot6(t):
 im=BG.copy();brand(im,6);title(im,'More than live dictation.',t,'Turn recordings into something you can use.')
 rounded(im,(101,350,1819,727),SHEET,24,LINE)
 circle(im,(212,448),48,SOFT);mic(im,212,446,1.1,BLUE)
 text(im,(296,402),'Your next big idea.m4a',42,INK,'serif')
 text(im,(299,470),'Local audio transcription',26,MUTED)
 waveform(im,178,608,0,1540,True,BLUE,118,96)
 progress=smooth((t-.6)/4)
 line(im,[(178,694),(178+1540*progress,694)],BLUE,5)
 xx=550
 for k,s in enumerate(['TXT','SRT','VTT']):
  yy=779+int((1-ease((t-2-k*.3)/.8))*25)
  rounded(im,(xx,yy,xx+238,yy+86),INK if t>3+k*.4 else '#e9e1d5',14)
  text(im,(xx+119,yy+20),s,35,SHEET if t>3+k*.4 else MUTED,'mono',anchor='ma');xx+=290
 text(im,(101,904),'WAV · MP3 · M4A / AAC · FLAC · Ogg',24,MUTED)
 return im

def shot7(t):
 im=BG.copy();brand(im,7);title(im,'One less thought to lose.',t,'Capture now. Find it when it matters.')
 # Two authentic app views, presented as a moving desk of manuscripts.
 screen(im,'history',100,360,1010)
 screen(im,'notes',850+int((1-ease(t/.8))*60),424,970)
 rounded(im,(112,325,302,376),INK,10);text(im,(137,334),'History',27,SHEET,'medium')
 rounded(im,(862,389,1029,440),INK,10);text(im,(888,398),'Notes',27,SHEET,'medium')
 return im

def shot8(t):
 im=DBG.copy();brand(im,8,True)
 text(im,(97,151),'Your words stay yours.',89,SHEET,'serif')
 text(im,(102,268),'Speech recognition and cleanup run on your computer.',31,'#bcb8b0')
 # Local processing is represented inside the monitor boundary.
 d=ImageDraw.Draw(im)
 rounded(im,(100,370,1067,809),'#252d32',25,'#515b61',2)
 rounded(im,(124,394,1043,762),'#1a2025',13)
 waveform(im,208,563,t,738,True,'#b6d9e8',57,122)
 text(im,(583,683),'Processed on this device',28,'#d7e5ea','medium',anchor='ma')
 line(im,[(482,810),(482,848),(684,848),(684,810)],'#515b61',7)
 line(im,[(413,855),(752,855)],'#667177',9)
 items=[('Local speech recognition','Your audio stays on this computer.'),('Local writing cleanup','Optional models, running on your device.'),('Works offline','After the initial model and runtime downloads.')]
 for k,(head,desc) in enumerate(items):
  yy=392+k*147
  circle(im,(1184,yy+25),21,'#2d443c');check(im,1182,yy+25,'#a5d2b4',14)
  text(im,(1230,yy),head,32,SHEET,'medium')
  para(im,(1230,yy+52),desc,25,570,'#bcb8b0')
 return im

def shot9(t):
 im=BG.copy();brand(im,9)
 logo_at(im,871,170+int((1-ease(t/.8))*25),178)
 text(im,(960,388),'Speak freely.',132,INK,'serif',anchor='ma')
 text(im,(960,556),'More room for your ideas.',43,BLUE,'italic',anchor='ma')
 rounded(im,(649,679,1271,759),INK,16)
 text(im,(960,696),'Explore Reflow on GitHub',33,SHEET,'medium',anchor='ma')
 text(im,(960,800),'github.com/Lawliet2004/reflow',29,MUTED,'mono',anchor='ma')
 return im

SHOTS=[shot0,shot1,shot2,shot3,shot4,shot5,shot6,shot7,shot8,shot9]

def subtitle(im,t):
 c=next((c for c in CAPTIONS if c['start']<=t<c['end']),None)
 if not c:return
 label=c['text'];size=31
 if width(label,size)>1640:size=27
 ww=width(label,size)+56
 rounded(im,((W-ww)/2,975,(W+ww)/2,1036),'#292d30',12)
 text(im,(W/2,986),label,size,'#fffaf2','medium',anchor='ma')

def frame(t):
 i=next((k for k,s in enumerate(STORY) if s['start']<=t<s['end']),9)
 local=t-STORY[i]['start'];im=SHOTS[i](local)
 if i and local<.40:
  old=SHOTS[i-1](STORY[i-1]['end']-STORY[i-1]['start']-.001)
  im=Image.blend(old,im,smooth(local/.4))
 subtitle(im,t)
 # Thin reading-progress line, outside caption and title safe areas.
 d=ImageDraw.Draw(im);d.rectangle((0,H-5,int(W*t/90),H),fill=BLUE)
 # Very short opening and closing fades, with a held, readable end card.
 if t<.25:im=Image.blend(Image.new('RGB',(W,H),PAPER),im,ease(t/.25))
 if t>89.65:im=Image.blend(im,Image.new('RGB',(W,H),PAPER),smooth((t-89.65)/.35))
 return im

def contact():
 out=Image.new('RGB',(W*2//3,H*5//3),PAPER)
 for i,s in enumerate(STORY):
  t=s['start']+min(4,(s['end']-s['start'])/2)
  im=frame(t).resize((640,360),Image.Resampling.LANCZOS)
  out.paste(im,((i%2)*640,(i//2)*360))
 out.save(ROOT/'output/storyboard.jpg',quality=90)
 # Purpose-designed cover, readable in a social feed.
 im=BG.copy();logo_at(im,100,88,80);text(im,(202,96),'Reflow',49,INK,'medium')
 text(im,(96,256),'Speak',150,INK,'serif');text(im,(96,421),'freely.',150,BLUE,'italic')
 para(im,(105,667),'Your voice.\nYour words.\nYour computer.',44,550,INK,'sans',1.4)
 screen(im,'home',761,207,1080)
 im.save(ROOT/'output/reflow-promo-thumbnail.jpg',quality=96)
 print('Storyboard and cover saved.',flush=True)

def render():
 out=ROOT/'output/reflow-promo-90s-1080p.mp4'
 cmd=['ffmpeg','-y','-hide_banner','-loglevel','warning','-f','rawvideo','-vcodec','rawvideo','-pix_fmt','rgb24','-s',f'{W}x{H}','-r',str(FPS),'-i','pipe:0','-i',str(ROOT/'assets/mix.wav'),'-map','0:v:0','-map','1:a:0','-c:v','libx264','-preset','fast','-crf','18','-pix_fmt','yuv420p','-profile:v','high','-level','4.1','-c:a','aac','-b:a','256k','-af','loudnorm=I=-16:TP=-1.5:LRA=9','-ar','48000','-t','90','-movflags','+faststart','-metadata','title=Reflow — Speak freely.','-metadata','comment=Original promotional film. Product screens use sample data. Workflow animations are illustrative.',str(out)]
 p=subprocess.Popen(cmd,stdin=subprocess.PIPE)
 start=time.time()
 try:
  for n in range(90*FPS):
   p.stdin.write(frame(n/FPS).tobytes())
   if n%150==0:print(f'Render {n//FPS:02d}/90 sec · elapsed {time.time()-start:.0f}s',flush=True)
  p.stdin.close();code=p.wait()
  if code:raise RuntimeError(f'ffmpeg exited {code}')
 except BaseException:
  p.kill();raise
 print(f'Export saved: {out}',flush=True)

if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--preview',action='store_true');parser.add_argument('--frame',type=float);args=parser.parse_args()
 cp=ROOT/'assets/captions.json'
 if cp.exists():CAPTIONS=json.loads(cp.read_text())
 if args.frame is not None:
  frame(args.frame).save(ROOT/'output/check-frame.png')
 elif args.preview:contact()
 else:contact();render()
