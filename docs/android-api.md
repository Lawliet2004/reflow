# Reflow LAN API (Android companion)

Reflow keeps Qwen3-ASR on the **Windows or Linux desktop**. The Android app is a remote microphone.

## Enable

Desktop → Settings → **Android / LAN API** → Enable LAN API.

Default bind is `0.0.0.0:7840` (LAN). Pair with the 6-digit code or the QR (`reflow://pair?...`).

Headless:

```bash
reflow --api --bind 0.0.0.0:7840
```

## Auth

1. `POST /v1/pair` with `{ "code", "device_name" }` → `{ "token", "server_name", "port" }`
2. All other routes: `Authorization: Bearer <token>`

## HTTP

| Method | Path | Auth |
|---|---|---|
| GET | `/v1/health` | no |
| GET | `/v1/status` | yes |
| POST | `/v1/pair` | pairing code |
| GET | `/v1/history` | yes |
| GET | `/v1/history/search?q=` | yes |
| DELETE | `/v1/history/:id` | yes |
| DELETE | `/v1/history` | yes |
| POST | `/v1/inject` `{ "text" }` | yes |
| POST | `/v1/transcribe` multipart `file` | yes |
| GET | `/v1/stream` WebSocket | yes |

## WebSocket `/v1/stream`

Client text:

```json
{"type":"start","language":"auto","format":"pcm_s16le","sample_rate":16000,"inject":false}
{"type":"stop"}
{"type":"cancel"}
```

Client binary: 16-bit little-endian PCM (prefer 16 kHz mono).

Server text: `ready`, `partial`, `final`, `error`.

Audio never leaves the computer that runs Reflow.
