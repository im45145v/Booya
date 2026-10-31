# Cloud and Local Providers Integration

## Summary

This document summarizes the integration of cloud AI providers and local LLM support in Booya, with special support for Google Gemma models.

## Provider Integration Methods

### Method 1: Cloud API (Google Generative Language API)
**Google Gemma via Cloud API**
- **API Endpoint**: `https://generativelanguage.googleapis.com/v1beta/openai`
- **Capabilities**: LLM (text generation for guides, chapters)
- **Default Model**: `gemma-2-9b-it`
- **Authentication**: API Key via Google AI Studio
- **API Compatibility**: OpenAI-compatible chat completions endpoint

**How to Use**:
1. Get your API key from https://aistudio.google.com/apikey
2. Open Settings > Cloud & API keys in Booya
3. Paste your Gemini/Gemma API key
4. Select "Google Gemma" for the Writing (LLM) task
5. Accept the data sharing notice

**Notes**:
- Uses the same API endpoint as Gemini but with open-source Gemma models
- Data is sent to Google's servers for processing

### Method 2: Local Ollama (Recommended for Privacy)
**Google Gemma via Ollama (Local)**
- **Installation**: Free and runs entirely on your machine
- **Privacy**: Nothing leaves your computer
- **Auto-detection**: Booya automatically prefers Gemma models when available
- **Performance**: Depends on your hardware (GPU recommended)

**How to Install and Use**:
```bash
# Install Ollama (if not already installed)
# macOS: brew install ollama
# Or visit: https://ollama.ai

# Install a Gemma model (choose based on your RAM/GPU):
ollama pull gemma2:2b        # 2B params - Lightweight, fast, ~3GB RAM
ollama pull gemma2:9b        # 9B params - Better quality, ~8GB RAM
ollama pull gemma2:27b       # 27B params - Best quality, ~16GB RAM

# Or use Gemma 1 models:
ollama pull gemma:2b         # 2B params
ollama pull gemma:7b         # 7B params

# Start Ollama server (runs automatically on macOS)
ollama serve

# List installed models
ollama list
```

**Booya Auto-Detection**:
1. Booya checks `http://127.0.0.1:11434/api/tags` for installed models
2. If Gemma models are found, they are **automatically preferred**
3. If no Gemma, uses the first available Ollama model
4. Override with `BOOYA_LLM_MODEL` environment variable

**Advantages**:
- ✅ Complete privacy - no data leaves your machine
- ✅ No API costs or rate limits
- ✅ Works offline
- ✅ Faster for repeated use (no network latency)
- ✅ Automatic model updates via Ollama

**Disadvantages**:
- Requires local hardware (RAM/GPU)
- First download is ~2-10GB depending on model
- Slower inference on CPU-only systems

### Method 3: Custom OpenAI-Compatible Local Server
**Any local OpenAI-compatible server**
```bash
# Example: Use with LM Studio, llama.cpp, or other servers
export BOOYA_LLM_URL="http://localhost:1234/v1"
export BOOYA_LLM_MODEL="gemma-2-9b-it"
```

## Newly Integrated Provider (Cloud)

### 1. Google Gemma ✅

**Status**: Fully integrated (both cloud and local)

**Cloud API Details**:
- Same API as Google Gemini but with open-source Gemma models
- API key from Google AI Studio
- OpenAI-compatible endpoints

**Local Details**:
- Auto-detected via Ollama
- Preferred over other local models
- No API key needed

### 2. Backboard ✅

**Status**: Integrated (placeholder configuration)

**Details**:
- **API Endpoint**: `https://api.backboard.ai/v1` (placeholder)
- **Capabilities**: LLM
- **Default Model**: `backboard-1` (placeholder)
- **Authentication**: API Key
- **API Compatibility**: OpenAI-compatible

**How to Use**:
1. Get your API key from Backboard
2. Open Settings > Cloud & API keys in Booya
3. Paste your Backboard API key
4. Select "Backboard" for the Writing (LLM) task
5. Accept the data sharing notice

**Important**: The Backboard integration uses placeholder values and may need adjustment based on their actual API documentation.

### 3. ElevenLabs ✅ (Already Existed)

**Status**: Previously integrated, confirmed working

**Details**:
- **API Endpoint**: `https://api.elevenlabs.io/v1`
- **Capabilities**: STT (Speech-to-Text) and TTS (Text-to-Speech)
- **Default Models**: 
  - STT: `scribe_v1`
  - TTS: `eleven_multilingual_v2`
- **Authentication**: API Key
- **Voice Support**: Configurable voice selection

## Implementation Details

### Code Changes

1. **`crates/booya-ai/src/llm.rs`**:
   - Enhanced Ollama detection to prefer Gemma models
   - Added documentation for installing Gemma via Ollama
   - Added `gemma2:*` and `gemma:*` model preference logic
   - Auto-detection checks Ollama API tags endpoint

2. **`crates/booya-ai/src/providers.rs`**:
   - Added `Gemma` and `Backboard` to the `Provider` enum
   - Added provider metadata (name, tasks, base URL, terms, privacy, key URL, default models)
   - Updated the `ALL` providers list
   - Implemented `id()` method for new providers
   - Added provider info with OpenAI-compatible endpoints

3. **Documentation Updates**:
   - Updated `README.md` provider table
   - Updated `PRIVACY.md` to list new providers
   - Updated `docs/release-notes/v0.2.0.md`
   - Enhanced this document with Ollama instructions

4. **Tests**:
   - Added test for Gemma LLM cloud integration
   - Added test for Backboard LLM integration
   - Added tests for Gemma preference logic in Ollama detection

### Integration Architecture

Both cloud and local providers follow the existing patterns:
- **Cloud**: OpenAI-compatible endpoints with Bearer token auth
- **Local**: Automatic detection with preference for Gemma models
- Integrate with the existing consent and preview system (cloud only)
- Follow the same error handling and key redaction patterns (cloud only)

## Usage in Booya

### Cloud providers require explicit consent:
1. User must paste their own API key (stored in OS keychain)
2. User must enable the provider for a specific task (STT, LLM, or TTS)
3. User must accept a data-leaves-device notice (recorded with date)
4. Every call shows a preview of what will be sent before sending

### Local providers (Ollama) are automatically used:
1. No API key or consent needed
2. Automatically detected on startup
3. Gemma models are preferred when available
4. Completely private - nothing leaves your machine

## Environment Variables

For CLI/advanced usage:
- **Cloud Gemma**: `BOOYA_GEMMA_API_KEY` (shared with Gemini via Google AI Studio)
- **Backboard**: `BOOYA_BACKBOARD_API_KEY`
- **Local Override**: `BOOYA_LLM_MODEL` to force a specific Ollama model

Example:
```bash
# Force a specific local Gemma model
export BOOYA_LLM_MODEL="gemma2:27b"

# Or use a remote server
export BOOYA_LLM_URL="http://your-server:8080/v1"
export BOOYA_LLM_MODEL="gemma-2-9b-it"
export BOOYA_ALLOW_REMOTE_LLM=1  # Required for non-localhost URLs
```

## Provider Comparison

| Provider | Type | STT | LLM | TTS | Privacy | Cost |
|----------|------|-----|-----|-----|---------|------|
| OpenAI | Cloud | ✅ | ✅ | ✅ | Data to servers | Pay per use |
| Anthropic | Cloud | | ✅ | | Data to servers | Pay per use |
| Google Gemini | Cloud | | ✅ | | Data to servers | Pay per use |
| **Google Gemma (Cloud)** | Cloud | | ✅ | | Data to Google | Pay per use |
| **Google Gemma (Ollama)** | Local | | ✅ | | ✅ Complete | Free |
| Groq | Cloud | ✅ | ✅ | | Data to servers | Pay per use |
| OpenRouter | Cloud | | ✅ | | Data to servers | Pay per use |
| Deepgram | Cloud | ✅ | | | Data to servers | Pay per use |
| AssemblyAI | Cloud | ✅ | | | Data to servers | Pay per use |
| ElevenLabs | Cloud | ✅ | | ✅ | Data to servers | Pay per use |
| **Backboard** | Cloud | | ✅ | | Data to servers | Pay per use |
| Custom | Both | ✅ | ✅ | ✅ | Depends | Varies |

## Performance Comparison

### Gemma Models via Ollama (Local)

| Model | Parameters | RAM Required | Speed | Quality | Use Case |
|-------|-----------|--------------|-------|---------|----------|
| gemma2:2b | 2B | ~3GB | Very fast | Good | Quick drafts, limited RAM |
| gemma2:9b | 9B | ~8GB | Fast | Better | General purpose (recommended) |
| gemma2:27b | 27B | ~16GB | Moderate | Best | High quality, powerful hardware |
| gemma:2b | 2B | ~3GB | Very fast | Good | Legacy, smaller tasks |
| gemma:7b | 7B | ~6GB | Fast | Good | Legacy, general purpose |

### Recommendations:
- **Laptop with 8GB RAM**: Use `gemma2:2b` or `gemma2:9b` (may be slow)
- **Desktop with 16GB RAM**: Use `gemma2:9b` (optimal balance)
- **Workstation with 32GB+ RAM**: Use `gemma2:27b` (best quality)
- **Apple Silicon Mac**: All models work well due to unified memory

## Security Considerations

### Cloud Providers:
- API keys are never logged or stored in plain text
- Keys are stored in the OS keychain (desktop) or environment variables (CLI)
- Error messages redact any potential key exposure
- All remote endpoints must use HTTPS (except localhost)
- User consent is tracked per provider and task with timestamps

### Local Providers (Ollama):
- No API keys needed
- No network requests (except initial model download)
- No data leaves your machine
- No consent required (local processing)

## Troubleshooting

### Ollama not detected:
```bash
# Check if Ollama is running
curl http://127.0.0.1:11434/api/tags

# If not running, start it
ollama serve

# Or restart the service
# macOS: brew services restart ollama
# Linux: sudo systemctl restart ollama
```

### Gemma model not found:
```bash
# Pull the model first
ollama pull gemma2:9b

# Verify it's installed
ollama list | grep gemma
```

### Force Booya to use a specific model:
```bash
# Set environment variable before starting Booya
export BOOYA_LLM_MODEL="gemma2:27b"
```

### Performance issues:
```bash
# Check GPU usage (NVIDIA)
nvidia-smi

# On macOS, check Activity Monitor for GPU usage
# Consider using a smaller model if RAM is limited
ollama pull gemma2:2b
```

## Next Steps

For Backboard:
1. ✅ Integration scaffold added
2. ⏳ Verify actual API documentation and update endpoints/models as needed
3. ⏳ Test with real API key once available

For Gemma:
1. ✅ Cloud integration complete
2. ✅ Local Ollama integration complete with auto-detection
3. ✅ Preference logic for Gemma models implemented
4. ✅ Ready for production use

## References

- Google AI Studio: https://aistudio.google.com/apikey
- Gemma Documentation: https://ai.google.dev/gemini-api/docs/models/gemma
- Ollama: https://ollama.ai
- Ollama Gemma Models: https://ollama.ai/library/gemma
- Ollama Gemma2 Models: https://ollama.ai/library/gemma2
- ElevenLabs Documentation: https://elevenlabs.io/docs
- Backboard: https://backboard.ai (API docs TBD)
