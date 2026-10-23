# Reflow — Speak freely.

90-second landscape promotional film for YouTube, LinkedIn, and social feeds.

## Deliverables

- `output/reflow-promo-90s-1080p.mp4`: 1920 × 1080, 16:9, 30 fps; H.264 High / AAC stereo, captions burned in, fast-start web playback. Exact duration: 90 seconds.
- `output/reflow-promo-thumbnail.jpg`: matching Full HD cover image.
- `output/reflow-promo.en.srt`: English subtitles for accessibility or reuse. The main video already has visible captions; do not enable a second visible caption track if you want to avoid duplicate captions.
- `output/storyboard.jpg`: all ten scenes at a glance.
- `source/`: editable story, original audio composition and video renderer.
- `assets/`: product screenshots, synthetic narration, original music, audio mix and timing data.

## Creative direction

Warm paper, editorial serif typography, ink blue accents and an on-device privacy chapter in charcoal. The story moves from an immediate writing hook into an illustrated hotkey workflow, cleanup, writing styles, vocabulary, imported audio, Notes and History, and a GitHub call to action. Short captions work with sound off.

The product screenshots use Reflow’s actual React components with fictional sample data and simulated installed-model state in a temporary browser session. Workflow graphics, processing bars and examples are illustrative; they are not performance benchmarks or a live recording of speech recognition. No production application source or user data was modified.

Narration: synthetic English voice, Microsoft Edge TTS `en-US-AndrewNeural`. Original instrumental score synthesized specifically for this video; no stock tracks, third-party music samples, testimonials or invented performance statistics. Reflow logo comes from this repository. Typography uses installed URW P052, IBM Plex Sans/Mono and Space Grotesk fonts.

The offline claim refers to recognition and cleanup after the initial model/runtime downloads. Optional network features are outside this claim. The end card points to the Git repository configured in this checkout.

## Reproduce

Requires Python 3, Pillow, NumPy, edge-tts, FFmpeg, Node, Playwright and Chrome. This workspace’s local environment is `promo/.venv`.

1. Start Reflow’s Vite development server on port 5178: `npm run dev -- --host 127.0.0.1 --port 5178`.
2. Capture seeded product views: `node promo/source/capture.mjs`.
3. Generate narration, original score and subtitles: `promo/.venv/bin/python promo/source/audio.py`. Existing narration files are reused. Delete the corresponding `assets/voice-XX.mp3` and `.json` after editing a narration line to regenerate it.
4. Render the storyboard only: `promo/.venv/bin/python promo/source/render.py --preview`.
5. Render a full-resolution frame for inspection: `promo/.venv/bin/python promo/source/render.py --frame 31`.
6. Render the complete film: `promo/.venv/bin/python promo/source/render.py`.

The capture script uses the bundled Playwright path available on this machine; adjust it and the font paths in `render.py` when moving to another workstation.

## Suggested post

Your next idea shouldn’t have to wait for your keyboard.

Meet Reflow: local-first desktop dictation that turns natural speech into words you can use. Dictate into your apps, clean up your writing, add your vocabulary, and turn recordings into text — with speech recognition and cleanup running on your computer.

Download the models once. Then speak freely, offline.

Explore Reflow: https://github.com/Lawliet2004/reflow

#Reflow #VoiceToText #LocalAI #Productivity #Privacy
