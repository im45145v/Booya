//! DemoLens local AI. Everything here runs on the user's machine with open
//! weight models that are downloaded on first use. No cloud APIs.
//!
//! * `models`     list, download and delete models (sizes shown up front)
//! * `transcribe` speech to text with whisper.cpp, word level timestamps
//! * `redact`     find emails, keys, card and phone numbers on screen (OCR)
//! * `llm`        talk to a local LLM (Ollama, llama.cpp server, or a managed
//!                llama.cpp runtime) to write titles, chapters, guides, posts
//! * `docs`       the "demo to docs" pipeline with a no-model fallback

pub mod docs;
pub mod interactive;
pub mod llm;
pub mod models;
pub mod pipeline;
pub mod providers;
pub mod redact;
pub mod s3;
pub mod status;
pub mod transcribe;
