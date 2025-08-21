# Live Translation (Rust + GPT-5)

**Finish speaking → instantly translated.**
Built with Rust for steady low latency. Powered by GPT-5 for context-aware accuracy.

<p align="center">
  <em>Indonesian → 日本語 on the fly • Q&A in Deutsch • Recap in English • One click to 한국어 • Also: العربية, Français, Nederlands, Русский, Español</em>
</p>

https://www.youtube.com/watch?v=0PVihxcbK1g demo video
---

## ✨ What this project does

In a mixed-language room—Japan in front, Europe in the middle, the Middle East at the back—you speak **Indonesian**.
As your first sentence ends, the **Japanese** text appears instantly on the screen.
A **German** engineer asks a question—keep answering in your language; **Deutsch** captions flow without pause.
Moderator wants a universal recap? Switch to **English**. A participant asks for **한국어**—click, done.
No device juggling. No awkward “start/stop”.

**Why it works:**

* **Rust** keeps the audio → text → translation pipeline tight and predictable.
* **GPT** reads context, preserves technical terms and tone, so translations feel natural.
* **Auto end-of-utterance**: as soon as you finish speaking, it’s translated—no button mashing.

---

## 🔑 Features

* **True live captions**: low, steady latency from mic → screen.
* **Multi-language output**: render one or many target languages at once.
* **Context & glossary-aware** (via system prompts + per-session vocabulary).
* **Auto end-of-speech detection** (simple VAD) — no manual start/stop.
* **Web UI**: browser mic capture, real-time caption panel, quick language switcher.
* **Stateless API**: easy to embed into your own presenter/meeting tools.
* **Production-minded**: structured logs, graceful shutdown, configurable timeouts.

---

## 🧱 Architecture (High-Level)

```
[Browser Mic]
   |
   |  PCM chunks over WebSocket
   v
[Rust Server]
  ├─ VAD (end-of-utterance detection)
  ├─ ASR (speech → text) via GPT-5
  ├─ MT  (text → target languages) via GPT-5
  └─ Caption bus (fan-out to connected clients)
   |
   v
[Web Clients / Screens]  ←— subscribe → render captions in real time
```

---

## 🛠 Tech Stack

* **Language:** Rust (async with Tokio)
* **Web:** Axum or similar (HTTP + WebSocket)
* **Audio I/O:** Web Audio API (getUserMedia) → WS → server
* **VAD:** lightweight energy-based detector (pluggable)
* **LLM:** GPT-5 (ASR + translation)
* **Build/Run:** Cargo

> You can swap/extend VAD, ASR, or MT layers if you prefer different providers.

---

## 🚀 Quick Start

### Requirements

* Rust (stable)
* Node/npm (only if you rebuild the sample web client)
* An OpenAI API key with GPT-5 access

### 1) Configure environment

Create a `.env` in project root:

```bash
OPENAI_API_KEY=sk-...
# Comma-separated targets; change on UI anytime
TARGET_LANGS=ja,de,en,ko,ar,fr,nl,ru,es
# Default assumed spoken language (used for ASR hints)
SOURCE_LANG=id
# Optional model names if you have specific variants
ASR_MODEL=gpt-5-transcribe
MT_MODEL=gpt-5-translate
# Server
BIND_ADDR=0.0.0.0:8787
```

### 2) Run the server

```bash
cargo run --release
```

Server starts at `http://localhost:8787`.

### 3) Open the demo web UI

* Visit `http://localhost:8787/`
* Allow microphone access
* Choose target languages; start speaking Indonesian
* Watch captions appear at the end of each utterance

---

## 🧩 How it works (Pipeline Details)

1. **Audio stream**: Browser sends 16-bit PCM fragments via WebSocket.
2. **VAD**: Server groups fragments into utterances (end-of-speech).
3. **ASR**: On boundary, audio chunk → GPT-5 ASR → source text.
4. **Translate**: Source text → multiple target languages (parallel fan-out).
5. **Deliver**: Each connected client (stage screen, audience device, recorder) receives caption payloads.

**Latency controls:**

* Smaller audio frames + early VAD triggers = snappier feel.
* Back-pressure guards keep queues healthy under load.
* Per-language fan-out runs concurrently.

---

## 🧪 Local Test (CLI)

You can also test with a WAV file:

```bash
cargo run --release -- \
  --wav ./samples/indonesian_short.wav \
  --targets ja,en
```

The program will print the recognized Indonesian text and the translations.

---

## 🌐 Minimal Web Client (snippet)

```html
<script>
(async () => {
  const ws = new WebSocket(`ws://${location.host}/ws`);
  ws.binaryType = "arraybuffer";

  // Send control message (select targets)
  ws.onopen = () => ws.send(JSON.stringify({
    type: "control",
    targets: ["ja","en","de"],
    source: "id",
    glossary: [] // optional domain terms
  }));

  // Receive captions
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.type === "caption") {
      // { utterance_id, source_text, translations: {ja:"...",en:"..."} }
      renderCaption(msg);
    }
  };

  // Capture mic
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const ctx = new AudioContext();
  const src = ctx.createMediaStreamSource(stream);
  const proc = ctx.createScriptProcessor(4096, 1, 1);
  src.connect(proc); proc.connect(ctx.destination);
  proc.onaudioprocess = (e) => {
    const pcm = e.inputBuffer.getChannelData(0);
    // Float32 [-1,1] → Int16 PCM
    const buf = new Int16Array(pcm.length);
    for (let i = 0; i < pcm.length; i++) buf[i] = Math.max(-1, Math.min(1, pcm[i])) * 0x7fff;
    ws.send(buf.buffer);
  };
})();
</script>
```

> In production you’ll likely use `AudioWorklet`, sequence numbers, and back-pressure.

---

## ⚙️ Configuration

| Key              | Meaning                                | Default            |
| ---------------- | -------------------------------------- | ------------------ |
| `TARGET_LANGS`   | Comma list of ISO codes (ja,de,en,...) | `ja,de,en,ko`      |
| `SOURCE_LANG`    | Hint for ASR source language           | `id`               |
| `ASR_MODEL`      | ASR model id                           | `gpt-5-transcribe` |
| `MT_MODEL`       | Translation model id                   | `gpt-5-translate`  |
| `BIND_ADDR`      | Server bind address                    | `0.0.0.0:8787`     |
| `MAX_UTTER_MS`   | Utterance hard cap (ms)                | `6000`             |
| `MIN_SILENCE_MS` | Silence to close utterance (ms)        | `300`              |

---

## 💸 Costs & Limits

* Live captions call ASR and translation per utterance.
* Shorter utterances feel faster but increase request count.
* Consider batching ultra-short phrases with a small delay buffer.

---

## 🔐 Privacy & Security

* Audio is processed in memory; no persistence by default.
* Server logs contain only timing + size metadata unless you enable text logging.
* Bring your own auth/restrictions for production deployments (tokens, origin allowlist, rate limits).

---

## 🗺️ Roadmap

* [ ] Speaker labels/diarization (meeting mode)
* [ ] Per-language screen styling & large-font stage mode
* [ ] Translation memory + domain glossary upload
* [ ] Optional TTS output per language
* [ ] Recording & export (SRT/VTT)

---

## 🤝 Contributing

Issues and PRs are welcome!
Please:

1. Describe your use case and environment.
2. Add tests for core logic.
3. Keep latency meters green. 🙂

---

## 📄 License


---

## 🙏 Acknowledgements

* Rust community for rock-solid async foundations.
* OpenAI GPT for context-aware ASR + translation.

---

### TL;DR (for your README header)

> **Live Translation — finish speaking, instantly translated.**
> Rust for speed and stability. GPT-5 for accuracy and tone.
> Perfect for conferences, onboarding, classrooms, and cross-country teams.

---
