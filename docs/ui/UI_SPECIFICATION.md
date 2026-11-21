# UI/UX Specification — Resonance

**Document Version:** 1.0.0  
**Status:** Approved / Active Specification  
**Design Target:** Desktop Workspace (macOS, Windows, Linux)  

---

## 1. Design System & Aesthetics

Resonance adheres to a **hardware-inspired, high-contrast dark studio aesthetic**. The interface is engineered for prolonged sessions in low-light studio environments, prioritizing legibility, clear spatial hierarchy, and zero visual clutter.

### 1.1 Color Palette
- **Background Deep (`#121316`):** Root window background and canvas void.
- **Surface Level 1 (`#1a1c23`):** Panels, track headers, sidebar backgrounds.
- **Surface Level 2 (`#242731`):** Active panels, timeline lanes, mixer channels.
- **Border / Divider (`#2e323e`):** 1px subtle panel boundaries.
- **Accent Primary (`#4a88ff`):** Transport play state, selection highlights, active indicators.
- **Accent Record (`#ff3b30`):** Arming state, record button, active recording clips.
- **Accent Solo (`#ffcc00`):** Solo button active state.
- **Accent Mute (`#ff5e3a`):** Mute button active state.
- **Text Primary (`#f0f2f5`):** High-contrast labels, readouts, track names.
- **Text Secondary (`#8e95a5`):** Secondary values, timecode units, inactive controls.

### 1.2 Typography & Spacing
- **Interface Font:** Inter / System UI Sans (clean grotesque sans-serif).
- **Monospace Font:** JetBrains Mono / SF Mono (timecode, sample counters, BPM readouts, dBFS meters).
- **Grid Unit:** 4px base increment (standard paddings: 4px, 8px, 12px, 16px).

---

## 2. Workspace Panel Architecture

```
+----------------------------------------------------------------------------------------------------+
| 01. HEADER: Project Title • Status [Saved] • CPU Meter: 4% • DSP Latency: 2.7ms • Audio Device     |
+----------------------------------------------------------------------------------------------------+
| 02. TRANSPORT: [|<] [Play] [Stop] [Rec] [Loop] | 120.00 BPM | 4/4 | 00:01:24.125 | Bar: 004.2.000 |
+------------------+-------------------------------------------------------------+-------------------+
| 03. BROWSER      | 04. TIMELINE / ARRANGEMENT VIEW                             | 06. INSPECTOR     |
| • Project Files  | Ruler: 1.1   1.2   1.3   2.1   2.2   2.3   3.1   3.2         | Track: Drums      |
| • Samples        | [Track 1: Drums] [=== Clip A ===]   [=== Clip B ===]        | • Gain: -2.5 dB   |
| • Instruments    | [Track 2: Bass ]       [========== Clip C ==========]       | • Pan: Center     |
| • Templates      | [Track 3: Synth] [=============== Clip D ===============]       | • Color: Cyan     |
|                  |                                                             | • Source: WAV     |
+------------------+-------------------------------------------------------------+-------------------+
| 05. MIXER CONSOLE DOCK                                                                             |
| [ Trk 1: Drums ] | [ Trk 2: Bass  ] | [ Trk 3: Synth ] | ... | [ MASTER STEREO BUS ]               |
| [M][S][R]        | [M][S][R]        | [M][S][R]        |     | [Dim][Mono]                         |
| ( Pan Pot )      | ( Pan Pot )      | ( Pan Pot )      |     | ( Balance )                         |
| |||| [Fader]     | |||| [Fader]     | |||| [Fader]     |     | |||| [Master Fader]                 |
| -2.5 dB          | -6.0 dB          | 0.0 dB           |     | 0.0 dB  [PEAK: -0.2 dBFS]           |
+----------------------------------------------------------------------------------------------------+
```

---

## 3. Panel Functional Breakdown

### 3.1 Header Bar
- **Project Name & Save State:** Shows file name with an asterisk `*` when dirty.
- **Audio Engine Status:** Real-time indicator displaying active sample rate (e.g., `48.0 kHz`), buffer size (`128 samples`), and hardware driver name.
- **DSP Load Meter:** Smooth gauge showing percentage of available real-time audio callback time consumed.

### 3.2 Transport Bar
- **Controls:** Return-to-Zero (`|<`), Play/Pause, Stop, Record Arm, Loop Toggle.
- **Tempo Control:** Numeric field with drag-to-scrub and direct text input for BPM ($20.00 - 300.00$).
- **Time Signature:** Fractional selector ($4/4$, $3/4$, $6/8$, etc.).
- **Dual Time Readout:**
  - Musical Time: `BARS : BEATS : SIXTEENTHS : TICKS` (e.g. `008 . 3 . 02 . 040`).
  - Absolute Time: `HOURS : MINUTES : SECONDS . MILLISECONDS` (e.g. `00 : 02 : 14 . 520`).

### 3.3 Asset Browser
- Tree view allowing navigation of project-local audio folders and user sample libraries.
- Drag-and-drop support: Dragging a `.wav` file into the arrangement automatically creates a new track or drops the clip into an existing lane.

### 3.4 Timeline & Arrangement View
- **Timeline Ruler:** Top horizontal strip showing bar numbers, beat subdivisions, and loop range indicators.
- **Track Headers:** Left column per track containing:
  - Track color badge and editable track title.
  - Quick action buttons: Mute (`M`), Solo (`S`), Record Arm (`R`).
  - Track volume slider and pan mini-knob.
- **Track Lanes:** Horizontal lanes holding audio clips.
- **Audio Clips:**
  - Visual waveform display showing true bipolar peak envelopes.
  - Clip header with clip title and lock toggle.
  - Left and right trim handles for non-destructive boundary trimming.
  - Fade-in and fade-out envelope drag pins at top-left and top-right corners.
- **Playhead:** Vertical red hairline traversing the timeline with sample-accurate positioning.

### 3.5 Mixer Console
- Collapsible bottom dock or toggled full-height mixer view.
- **Channel Strips:**
  - Input selector and phase invert toggle.
  - Rotary pan potentiometer with center notch.
  - Large vertical gain fader calibrated from $-\infty\text{ dB}$ to $+6.0\text{ dB}$.
  - High-resolution dual stereo LED peak meters with persistent red over-ceiling clip light.
  - Numerical gain readout.
- **Master Channel:** Dedicated master strip routed to the primary stereo audio outputs.

### 3.6 Inspector Pane
- Context-sensitive sidebar displaying detailed properties of the currently selected track or clip:
  - For Clips: Source file path, length in samples/bars, source offset, fade curves, pitch shift, reverse toggle.
  - For Tracks: Input device routing, output bus routing, channel color, automation parameters.

---

## 4. Control Dynamics & Micro-Interactions

### 4.1 Rotary Knobs
- **Interaction Model:** Vertical drag (drag up to increase, drag down to decrease).
- **Fine Adjustment:** Holding `Shift` reduces adjustment speed by a factor of 5 for surgical control.
- **Reset to Default:** Double-click returns knob to its neutral default position (e.g. Center for Pan, $0\text{ dB}$ for Gain).
- **Direct Entry:** Right-click or `Alt`-click opens a direct numerical text input popover.

### 4.2 Timeline Navigation & Zoom
- **Horizontal Zoom:** `Ctrl + Mouse Wheel` (or trackpad pinch) zooms centered at mouse cursor position.
- **Vertical Zoom:** `Ctrl + Shift + Mouse Wheel` increases track lane heights.
- **Horizontal Scrolling:** `Shift + Mouse Wheel` or standard horizontal trackpad swipe.
- **Scrubbing:** Dragging directly inside the timeline ruler scrubs the playhead with audio preview.

---

## 5. Keyboard Shortcuts Specification

| Action | macOS Shortcut | Windows / Linux Shortcut | Context |
| :--- | :--- | :--- | :--- |
| **Play / Pause** | `Space` | `Space` | Global |
| **Stop & Return to Zero** | `Return` or `0 (Numpad)` | `Return` or `0 (Numpad)` | Global |
| **Toggle Recording** | `R` or `* (Numpad)` | `R` or `* (Numpad)` | Global |
| **Toggle Loop Mode** | `L` or `/` | `L` or `/` | Global |
| **Split Clip at Playhead** | `Cmd + E` | `Ctrl + E` | Arrangement |
| **Duplicate Selection** | `Cmd + D` | `Ctrl + D` | Arrangement |
| **Delete Selected Clips** | `Backspace` / `Delete` | `Delete` | Arrangement |
| **Select All** | `Cmd + A` | `Ctrl + A` | Global |
| **Undo** | `Cmd + Z` | `Ctrl + Z` | Global |
| **Redo** | `Cmd + Shift + Z` | `Ctrl + Y` / `Ctrl + Shift + Z` | Global |
| **Save Project** | `Cmd + S` | `Ctrl + S` | Global |
| **Save Project As...** | `Cmd + Shift + S` | `Ctrl + Shift + S` | Global |
| **Export Master Audio** | `Cmd + Shift + E` | `Ctrl + Shift + E` | Global |
| **Toggle Mixer Pane** | `M` or `Cmd + 2` | `M` or `Ctrl + 2` | Global |
| **Zoom to Fit Selection** | `Z` | `Z` | Arrangement |
| **Snap to Grid Toggle** | `Cmd + G` | `Ctrl + G` | Arrangement |

