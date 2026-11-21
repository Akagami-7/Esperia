# Testing & Development Strategy — Resonance

**Document Version:** 1.0.0  
**Status:** Approved / Active Specification  
**Scope:** Workspace-Wide Quality Assurance, CI/CD, and Development Standards  

---

## 1. Quality Assurance Philosophy

Resonance is a real-time multimedia workstation. A software failure does not simply manifest as an error dialog—it can produce high-amplitude audio pops that damage hearing or studio monitors, or drop takes during irreplaceable recording sessions.

Our testing strategy follows a rigorous multi-tiered pyramid:
1. **Deterministic Unit Testing:** Fast, pure DSP math, timebase calculations, and project serialization tests.
2. **Property-Based Testing:** Fuzzing audio boundary conditions (zeros, infinities, NaNs, buffer edges).
3. **Real-Time Safety Verification:** Static and dynamic enforcement of zero-allocation and lock-free contracts in audio paths.
4. **Integration & Offline Rendering Tests:** Comparing offline bounced audio bit-for-bit against known golden masters.
5. **Multi-Platform CI & Hardware Validation:** Automated verification on macOS, Linux, and Windows.

---

## 2. Testing Pyramid & Test Types

```
         /\
        /  \        End-to-End System Tests (GUI + Audio Stream + Persistence)
       /----\
      /      \      Headless Audio Graph & Integration Tests (Multi-track Summing, Offline Bounce)
     /--------\
    /          \    Property-Based Tests (Buffer bounds, NaN resilience, Timebase overflow)
   /------------\
  /              \  Unit Tests (DSP Filters, Panning Laws, Ringbuffers, Serializers)
 /----------------\
```

### 2.1 Unit Tests
- **Target:** Core math in `crates/audio-engine` and device configuration in `crates/audio-io`.
- **Requirements:**
  - Test constant-power panning at $-1.0$, $0.0$, and $+1.0$.
  - Test parameter interpolation curves across block transitions.
  - Verify JSON serialization and deserialization of `.resonance` files.
  - Test atomic SPSC ring buffer push, pop, wrapping, and overflow behaviors.

### 2.2 Property-Based & Fuzz Testing (`proptest`)
- Generate randomized sequences of buffer sizes (e.g. $1$ to $4096$ frames), sample rates ($22,050\text{ Hz}$ to $192,000\text{ Hz}$), and non-aligned clip positions.
- **Invariants under test:**
  - No generated audio sample shall ever evaluate to `NaN` or `+Infinity` / `-Infinity`.
  - Summing $N$ tracks of silence must produce exact mathematical silence ($0.0$).
  - Seeking to any frame index within a clip must never trigger an out-of-bounds index panic.

### 2.3 Real-Time Safety Testing
- **Assertion:** No heap allocations during audio block evaluation.
- **Verification Tooling:**
  - Custom debug memory allocator that panics if `alloc()` or `dealloc()` is called from within a thread tagged as the audio callback thread.
  - Thread sanitizer (`TSAN`) checks to ensure zero data races between UI thread and engine.

### 2.4 Offline Render Golden Master Tests
- An integration test creates a synthetic project with 4 tracks, audio clips, panning, and gain changes.
- The project is rendered offline to a master WAV.
- The output waveform is compared sample-by-sample against a verified golden master fixture, asserting maximum deviation $< 1.0 \times 10^{-6}$.

---

## 3. Git Conventions & Branching Strategy

### 3.1 Branching Model
- **`main`:** The primary trunk. Must always pass all CI checks, compile cleanly, and be in a releasable state.
- **Feature Branches:** Named `feature/<topic>`, `fix/<issue-description>`, or `perf/<target>`.
- All changes land on `main` via pull requests with required approvals and green CI checks.

### 3.2 Conventional Commits Specification
All commit messages must adhere to the Conventional Commits 1.0.0 specification:
```
<type>(<optional scope>): <description>

[optional body]

[optional footer(s)]
```

#### Allowed Types:
- `feat`: A new user-facing feature or capability.
- `fix`: A bug fix.
- `docs`: Documentation-only changes.
- `refactor`: Code change that neither fixes a bug nor adds a feature.
- `perf`: A code change that improves performance or decreases latency.
- `test`: Adding or correcting tests.
- `chore`: Maintenance tasks, dependency updates, build tooling.

---

## 4. Continuous Integration (CI) Pipeline

Every pull request and push to `main` executes the following automated pipeline matrix across **macOS (latest)**, **Ubuntu (latest)**, and **Windows (latest)**:

```yaml
name: CI Pipeline

jobs:
  check:
    name: Lint, Format, Build & Test
    strategy:
      matrix:
        os: [macos-latest, ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - name: Code Formatting Check
        run: cargo fmt --all --check
      - name: Workspace Check
        run: cargo check --workspace --all-targets
      - name: Clippy Linter Check
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Run Test Suite
        run: cargo test --workspace
```

---

## 5. Release Criteria & Quality Gates

A release build candidate (e.g., `v0.1.0`) is only approved when it satisfies all of the following quality gates:

1. **Compilation & Warning Cleanliness:** Zero errors, zero warnings across all targets on macOS, Windows, and Linux.
2. **Deterministic Test Pass Rate:** 100% pass rate across all unit, integration, and property-based tests.
3. **No Audio Dropouts (XRuns):** Standard 30-minute stress test playing 32 tracks at 128 samples buffer size produces zero audio dropouts on reference hardware.
4. **Memory Leak Free:** Zero memory growth over 60 minutes of looping playback.
5. **Project Persistence Fidelity:** Round-trip project save, close, reload, and offline bounce matches initial render bit-for-bit.

