# Configuration reference

## Config file locations

`attend` loads configuration from two sources, merged together:

| Source | Path | Scope |
|--------|------|-------|
| **Project** | `.attend/config.toml` in the current directory or any parent | Per-project |
| **Global** | `~/.config/attend/config.toml` | All projects |

When both sources define the same field, project config takes precedence for
scalar values. Arrays (like `include_dirs`) are concatenated, with project
entries first.

If multiple `.attend/config.toml` files exist in the directory hierarchy (e.g.,
in both the project root and a parent workspace), closer files take precedence
for scalar values.

## Fields

All fields are optional.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `engine` | `"parakeet"` \| `"whisper"` | `"parakeet"` | Transcription engine |
| `model` | path string | auto-downloaded | Custom model path. When omitted, the default model for the engine is downloaded automatically on first use. |
| `include_dirs` | array of path strings | `[]` | Additional directories visible to the agent during narration. Context from outside the project directory (and these directories) is redacted. |
| `archive_retention` | duration string | `"7d"` | How long to keep delivered narrations as a safety net. Set to `"forever"` to disable pruning. |
| `clipboard_capture` | boolean | `true` | Capture clipboard changes (text and images) during narration. |
| `daemon_idle_timeout` | duration string | `"5m"` | How long the recording daemon idles before auto-exiting. Set to `"forever"` to keep the daemon running indefinitely. |

Duration strings use the format `<number><unit>`, where unit is `s` (seconds),
`m` (minutes), `h` (hours), or `d` (days).

## Transcription engines

Two speech-to-text engines are available, both running entirely on your machine:

| Engine | Default model | Download size | Notes |
|--------|---------------|---------------|-------|
| `parakeet` | [Parakeet TDT 0.6B](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) (ONNX) | ~1.2 GB | Better quality, multilingual, faster |
| `whisper` | [Whisper Small](https://huggingface.co/ggerganov/whisper.cpp) (GGML) | ~466 MB | Smaller download, English-focused |

Models are downloaded from [Hugging Face](https://huggingface.co/models) and
cached locally. Checksums for models are embedded in the `attend` binary to pin
the exact model data. No audio ever leaves your machine.

## Example

```toml
engine = "parakeet"
model = "/path/to/custom/model"
include_dirs = ["/path/to/other/project"]
archive_retention = "7d"
clipboard_capture = true
daemon_idle_timeout = "5m"
```
