# Booya privacy notice

Last updated: 28 October 2026. Applies to Booya 0.2.0 and later.

## Who we are
Booya is open source software maintained by Ashish Malla, India. Contact: open a private report at https://github.com/im45145v/Booya/security/advisories/new, or email im45145v@users.noreply.github.com via GitHub.

## What we collect
Nothing. The maintainer does not receive any data from the app. There is no account, telemetry, analytics, crash reporting or update check.

## What the app stores on your computer
- Screen recordings, and microphone, system audio or webcam only if you turn them on for a recording.
- An event log (`*.events.jsonl`) with cursor positions and clicks. If you turn on key capture for a recording, it also stores which shortcuts and named keys you pressed and when you typed. A letter, digit or symbol is stored only when it is part of a shortcut, meaning Ctrl, Alt (Option), Cmd or Win (Super) was held with it (for example Cmd+W or Ctrl+T). Plain typing, with or without Shift, is stored only as "typing" with no letters, so what you wrote is never recorded.
- Projects, transcripts, captions, a transcript cache and generated docs you create.
- Settings, and the cloud provider choices with the date you turned each one on.
- API keys and storage secrets, only in your operating system keychain.

Recordings are saved under `Videos/Booya/` (private folders, readable only by you on Linux and macOS) or where you choose. Settings > Privacy can delete old recordings automatically, delete one recording, or delete all Booya data.

## Network connections
- **Model downloads** from huggingface.co, github.com and an Amazon S3 bucket, only when you click download. Each file is checked against a SHA-256 hash. These services see your IP address under their own privacy policies.
- **Optional remote LLM:** only if you set `BOOYA_LLM_URL` to another machine and `BOOYA_ALLOW_REMOTE_LLM=1`. It must use https, and the app asks before sending the transcript.
- **Optional cloud AI** (OpenAI, Anthropic, Google Gemini, Google Gemma, Groq, OpenRouter, Deepgram, AssemblyAI, ElevenLabs, Backboard or an OpenAI compatible endpoint): off by default. You turn each provider on per task (speech to text, writing, text to speech) with your own key, and see what will be sent, to which endpoint, and how big it is, before each call. Speech to text sends the audio track only; writing sends the transcript text; text to speech sends the voiceover text. Video and event logs are never sent. The provider processes the data under your agreement with it and may store it outside your country.
- **Optional upload** to your own S3 or Cloudflare R2 bucket: off by default. Objects are private and shared with a signed link that expires (24 hours by default, 7 days at most). Event logs and project files are never uploaded.

## Your responsibilities
If you record other people or their data for work, you are the controller (GDPR) or data fiduciary (DPDP Act 2023) for that recording. Tell people, get consent where needed, and keep recordings only as long as you need them. Purely personal or household use falls outside GDPR (Art 2(2)(c)) and the DPDP Act (Section 3(c)(i)). Auto blur helps but can miss text: watch every export before sharing it.

## Children
Booya is not aimed at children and collects no data from anyone.

## Changes and questions
Changes are listed in the release notes. Questions or security issues: see "Who we are" above.
