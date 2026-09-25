# ADR-0005: Concurrency model

**Status:** Accepted 2026-09-25.
**Requirements:** §10, §11, §35, §36, §37, §4.5 · **Work package:** WP-07 · **Decision:** D8

## Context

§36 lists eleven workers and says the application "may use dedicated threads and Tokio
as appropriate", and that task failure "shall be isolated wherever possible". That is
permission, not a design. The plan's D8 row proposed the shape - Tokio for network I/O
only, dedicated OS threads on the real-time path - and WP-07 is where the transport
had to be built, so it is where the shape stops being a proposal.

Three facts from the work so far constrain it more than taste does:

- A `vcw_audio::capture::Capture` is **`!Send`**. CPAL's stream handle cannot cross a
  thread, so whichever thread opens a device must be the thread that keeps it for the
  life of the capture. This is a fact about the platform APIs, not a preference.
- WP-05 measured the writer: a 90-minute soak at 192 kHz costs about 3% of one core,
  and the ring never goes above a few blocks. Throughput is not the problem the design
  has to solve; **commit granularity and crash loss** are (D3, WP-06).
- S3 measured the IPC boundary and found it free at §35's payload sizes. The cost that
  matters on the UI side is main-thread *occupancy*, which is a rendering decision
  (D6), not a threading one.

## Decision

**Dedicated OS threads on the capture path, message passing between them, and no async
runtime anywhere near audio or SQLite. Tokio enters only when network I/O does.**

Concretely, as built in `vcw-core`:

| Thread | Owns | Started by | Ends when |
|--------|------|-----------|-----------|
| Device callback | The CPAL stream, the ring's producer | The OS audio stack | The stream is dropped |
| Writer | The SQLite connection, the ring's consumer | `persistence::spawn_on` | Told to stop, or the producer goes |
| Engine | The transport state machine and the live `Recorder` | `Engine::start` | `Shutdown`, or every sender drops |
| Caller | Whatever drives it: the CLI, a test, Tauri's command handler | - | - |

The engine thread is not an implementation detail that could have been a mutex. Because
`Capture` is `!Send`, the thread that takes `Arm` has to be the thread that holds the
device, and therefore the thread that takes every later command about it.

### Commands in, events out

§35 already splits the boundary that way, so the threading follows it rather than
inventing a second one. Commands go in on a `std::sync::mpsc::Sender<Command>`; events
come out through a `Bus` that fans out to any number of `Receiver`s. A `Command`
carries a *description* of what to open (`Setup`), never a device, which is what lets
the same enum cross a process boundary in the Tauri shell (WP-15).

`Bus::publish` cannot fail the engine. A subscriber that has gone away is pruned on the
next publish and takes nothing with it - that is §36's "task failure shall be isolated"
at the smallest scale it applies.

### The transport is a local variable

The state machine lives on the engine thread's stack and is moved through the loop by
value. There is no `Arc<Mutex<Machine>>`, and there cannot be a sixth "in transition"
phase: between any two statements the transport is exactly one of §11's five. The
typestate encoding (which is the *other* half of WP-07's exit criterion) only works at
all because a single thread owns it - a shared, locked transport would have to hand out
`&mut` and would lose the consuming transitions that make illegal moves unrepresentable.

### No async on the real-time path

No `async fn`, no executor and no `.await` between the device callback and a committed
block. The reasons are cumulative rather than ideological:

- An audio callback must not block, allocate or wait on a scheduler it does not control.
  `rtrb` hands frames over without any of the three.
- `rusqlite` is synchronous by design (D2, ADR-0002), and `synchronous=FULL` means the
  writer *wants* to block in `fsync` - on its own thread, where blocking is correct.
- An executor between the two would add a scheduling decision to every commit and buy
  nothing: the measured cost is 3% of a core, so there is no concurrency to win.

### Where Tokio does belong

Discogs and MusicBrainz lookups (§24, §25), artwork fetches, and any batched export
that turns out to be I/O bound. All of it is above `vcw-core` in the dependency graph
and none of it touches a device or the capture writer. Today the workspace has no
`tokio` dependency at all, and it stays that way until WP-12 needs one.

### Polling, not a heartbeat thread

The engine loop uses `recv_timeout(100 ms)`, so one thread serves both the commands and
the periodic `recording-position` event. A second timer thread would have needed a lock
on the transport, which is precisely what this design is buying its way out of.

## What this rules out

- **A single async runtime for the whole application.** Rejected: it would put an
  executor on the path §10 is about, and it cannot hold a `!Send` stream anyway.
- **A shared, locked transport.** Rejected: it defeats the type-level guarantee and
  buys nothing, since only one thread can hold the device.
- **A thread per §36 worker, started eagerly.** Threads are started by the thing that
  needs them and end when it does. The writer thread exists only for the life of a
  capture; the engine thread only for the life of a session.
- **Blocking the caller.** `Engine::send` returns as soon as the command is queued.
  What happened comes back as an event, on the same stream the UI and the log are
  reading, so no two observers can disagree about the order things happened in.

## Consequences

- The CLI, the test suite and the Tauri shell all drive the transport identically:
  send a command, read events. `vcw session` (WP-07) is the proof, and
  `crates/cli/tests/session_from_cli.rs` runs a whole capture through the shipped
  binary with no frontend compiled at all.
- A panic on the engine thread kills the transport and not the process. The `Closed`
  event is published unconditionally on the way out, so a consumer blocked on the
  stream is never left waiting - the one guarantee a fan-out bus has to make.
- Elevated thread priority, which the D8 row proposed, is **not** implemented and has
  not been needed: the 192 kHz soak showed no overruns at ordinary priority on any rig
  tested so far. It stays available for a platform that needs it rather than being
  applied speculatively.
- Playback (WP-10), the meter and waveform workers (WP-09) and the detector (WP-11)
  join this model rather than extending it: each is a thread that owns its own state
  and speaks to the rest through the same command/event boundary.
