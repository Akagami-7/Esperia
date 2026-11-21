# ADR-0006: Self-Contained Project Storage Format

## Status
Accepted

## Context
A DAW project file stores arrangement structure, track configurations, mixer parameter states, automation, and references to audio assets.

## Problem
Many legacy DAWs use opaque binary file formats or absolute filesystem paths to referenced samples. If a project is moved to another computer, or if samples are relocated, the project breaks with "missing files" errors. Furthermore, corrupting a single byte in an opaque binary blob often destroys the entire session.

## Alternatives
1. **Opaque Proprietary Binary Blob:** Compact and fast, but completely unreadable outside the application and vulnerable to total corruption.
2. **Single Monolithic Zip/SQLite Container:** Packs all audio and state into one file, but large projects (tens of gigabytes) suffer from sluggish save times and high memory overhead during repackaging.
3. **Directory Bundle with JSON Manifest & Audio Pool:** A directory structure containing a human-readable, schema-versioned JSON/MessagePack manifest (`project.json`), a local `audio/` pool, and disposable waveform `cache/` directories.

## Chosen Decision
We adopt a **self-contained directory bundle format** (`MySong.resonance/`):
- `project.json`: Clean, versioned manifest describing tracks, clips, and mixer states.
- `audio/`: Copied or referenced local audio files.
- `cache/`: Disposable waveform peak overviews (never required to open or reconstruct the project).
- Atomic saves via temporary file write followed by atomic filesystem rename.

## Consequences
- **Positive:** Human-readable, Git-friendly, resilient against file corruption, and trivially portable across platforms without broken sample links.
- **Negative:** Multiple files on disk rather than a single portable `.zip` icon (can be rendered as a package bundle on macOS via Finder Info.plist).

## Future Reconsideration Conditions
Optional binary chunk packing (e.g. MessagePack or CBOR) for ultra-dense automation curves if JSON parsing becomes a bottleneck on multi-gigabyte project manifests.

