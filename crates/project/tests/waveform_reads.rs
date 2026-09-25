/*
 *  waveform_reads.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Reading waveforms back out of a real project, at every resolution (§19, §37).
 *
 * MIT License
 *
 * Copyright (c) 2026 Stue Hunter
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 */

//! Reading waveforms back out of a real project, at every resolution (§19, §37).
//!
//! `vcw-signal`'s unit tests prove the arithmetic against hand-written triplets.
//! These prove the other three things, which need audio that was really written:
//! that the stored pyramid describes the audio underneath it, that every level
//! agrees with every other, and that what a draw costs is set by the span being
//! drawn rather than by how many samples are in the recording.

use std::time::Instant;

use vcw_project::persistence::{Config, Writer};
use vcw_project::waveform::{self, Rebuild, Shape};
use vcw_project::{Project, session};
use vcw_signal::waveform::{Column, Level, Request, Waveform};
use vcw_types::{CaptureInfo, CaptureMode, CaptureState, SampleRate, StorageFormat};

/// 16-bit stereo: the narrowest format, so a long capture stays a small file.
fn info(rate: u32) -> CaptureInfo {
    CaptureInfo::unverified(
        SampleRate(rate),
        2,
        StorageFormat::Int16,
        CaptureMode::Exclusive,
    )
}

/// Interleaved stereo from a function of frame and channel, in `i16`.
fn pcm(frames: u64, mut value: impl FnMut(u64, u16) -> i16) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames as usize * 4);
    for frame in 0..frames {
        for channel in 0..2u16 {
            out.extend_from_slice(&value(frame, channel).to_le_bytes());
        }
    }
    out
}

/// Writes one capture and hands back the project and its id.
fn capture(dir: &tempfile::TempDir, rate: u32, audio: &[u8]) -> (Project, i64) {
    let path = dir.path().join("wave.vcw");
    let project = Project::create(&path).expect("create");
    let info = info(rate);
    let mut writer = Writer::begin(project, &info, Config::default()).expect("begin");
    writer.push(audio).expect("push");
    let (outcome, project, _) = writer
        .finish_with_project(CaptureState::Finalised)
        .expect("finish");
    (project, outcome.capture_id)
}

/// Full scale in the first half of the capture, silence in the second.
///
/// Chosen because its waveform is knowable without doing any arithmetic: every
/// column in the loud half must read exactly +1 and -1 with an RMS of 1, and
/// every column in the quiet half must read exactly zero. A signal whose answer
/// had to be computed would be checking this code against itself.
fn half_loud(frames: u64) -> Vec<u8> {
    pcm(frames, |frame, _| {
        if frame >= frames / 2 {
            0
        } else if frame % 2 == 0 {
            i16::MAX
        } else {
            i16::MIN
        }
    })
}

/// A signal that is a function of the frame index alone.
///
/// [`half_loud`] is not: where it goes quiet depends on how long the capture
/// is, so the same span read out of two captures of different lengths would be
/// two different pieces of audio. This one reads the same out of any capture
/// that contains the frames.
fn positional(frames: u64) -> Vec<u8> {
    pcm(frames, |frame, channel| {
        let phase = (frame + u64::from(channel) * 500) % 1_000;
        ((phase as f64 / 1_000.0 - 0.5) * 2.0 * 32_000.0) as i16
    })
}

#[test]
fn a_known_signal_draws_the_shape_it_has() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Eight blocks, so the halves fall on a block boundary and no column has
    // to straddle the join.
    let frames = 12_000 * 8;
    let (project, id) = capture(&dir, 48_000, &half_loud(frames));

    // Eight columns, one per block, drawn from the block level.
    let request = Request::new(0, frames, 8);
    let shape = Shape::of(project.conn(), id).expect("shape");
    assert_eq!(request.level(shape.levels), Level::Block);
    let drawn = waveform::read(project.conn(), id, 0, &request).expect("read");

    assert_eq!(drawn.columns.len(), 8);
    assert_eq!(drawn.level, Level::Block);
    for (at, column) in drawn.columns.iter().enumerate() {
        assert_eq!(column.frames, 12_000, "column {at}");
        if at < 4 {
            // i16's full scale is asymmetric, which is why this is not 1.0
            // on both sides. See `vcw_signal::meter`.
            assert!((column.max - 0.999_97).abs() < 1e-4, "column {at} max");
            assert_eq!(column.min, -1.0, "column {at} min");
            assert!(column.rms > 0.999, "column {at} rms {}", column.rms);
        } else {
            assert_eq!(
                *column,
                Column {
                    frames: 12_000,
                    ..Column::EMPTY
                },
                "column {at}"
            );
        }
    }
    assert_eq!(drawn.covered, frames);
    assert_eq!(drawn.drawn(), Some(7), "silence is drawn, not absent");
}

/// The claim the pyramid rests on: a coarse read is not a worse read.
#[test]
fn every_level_gives_the_same_answer_for_the_same_span() {
    let dir = tempfile::tempdir().expect("tempdir");
    let frames = 12_000 * 4;
    // A ramp rather than a square wave, so min, max and rms are all different
    // from each other and a level that got any of the three wrong would show.
    let audio = pcm(frames, |frame, channel| {
        let phase = (frame + u64::from(channel) * 1_000) % 2_000;
        ((phase as f64 / 2_000.0 - 0.5) * 2.0 * 32_000.0) as i16
    });
    let (project, id) = capture(&dir, 48_000, &audio);
    let shape = Shape::of(project.conn(), id).expect("shape");

    // Four columns of one block each, so every level's triplets fall wholly
    // inside one column and the three reads are comparable exactly.
    let request = Request::new(0, frames, 4);
    let read =
        |level| waveform::read_at(project.conn(), id, 1, &request, &shape, level).expect("read");
    let samples = read(Level::Samples);
    let s256 = read(Level::Summary256);
    let block = read(Level::Block);

    for at in 0..4 {
        let (a, b, c) = (samples.columns[at], s256.columns[at], block.columns[at]);
        assert_eq!(a.frames, b.frames, "column {at} frames");
        assert_eq!(a.frames, c.frames, "column {at} frames");
        assert_eq!(a.min, b.min, "column {at} min at 256");
        assert_eq!(a.min, c.min, "column {at} min at block");
        assert_eq!(a.max, b.max, "column {at} max at 256");
        assert_eq!(a.max, c.max, "column {at} max at block");
        // RMS composes exactly, so the only difference allowed here is the
        // rounding of the f32 the pyramid was stored as.
        assert!(
            (a.rms - b.rms).abs() < 1e-6 && (a.rms - c.rms).abs() < 1e-6,
            "column {at}: samples {} vs 256 {} vs block {}",
            a.rms,
            b.rms,
            c.rms
        );
    }
}

/// §37, in the form the requirement actually states it: **sample count**.
///
/// The same ten seconds recorded at four times the rate is four times the
/// samples and four times the bytes. Zoomed out, it is not one row more to read
/// and not one blob to open, because a block is a duration and the whole-block
/// triplet lives in the row. This is the property that makes a 24/192 side no
/// more expensive to draw than a 16/44 one.
#[test]
fn a_zoomed_out_draw_costs_the_same_however_many_samples_are_under_it() {
    let dir48 = tempfile::tempdir().expect("tempdir");
    let dir192 = tempfile::tempdir().expect("tempdir");
    let (a, id_a) = capture(&dir48, 48_000, &half_loud(48_000 * 10));
    let (b, id_b) = capture(&dir192, 192_000, &half_loud(192_000 * 10));

    let bytes = |p: &Project| {
        p.conn()
            .query_row("SELECT SUM(LENGTH(samples)) FROM sampleblocks", [], |r| {
                r.get::<_, i64>(0)
            })
            .expect("bytes")
    };
    assert_eq!(bytes(&b), bytes(&a) * 4, "the premise of this test");

    let rows = |p: &Project, id: i64| {
        p.conn()
            .query_row(
                "SELECT COUNT(*) FROM capture_blocks WHERE capture_id = ?1 AND channel = 0",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .expect("rows")
    };
    assert_eq!(
        rows(&a, id_a),
        rows(&b, id_b),
        "the same duration must be the same number of blocks"
    );

    // 40 columns over ten seconds: 250 ms each, which is where the block level
    // starts to serve and blob reading stops.
    let draw = |p: &Project, id: i64, frames: u64| {
        let request = Request::new(0, frames, 40);
        let started = Instant::now();
        let drawn = waveform::read(p.conn(), id, 0, &request).expect("read");
        (drawn, started.elapsed())
    };
    let (low, at_48) = draw(&a, id_a, 48_000 * 10);
    let (high, at_192) = draw(&b, id_b, 192_000 * 10);

    assert_eq!(low.level, Level::Block);
    assert_eq!(high.level, Level::Block, "zoomed out, neither opens a blob");
    assert_eq!(low.columns.len(), high.columns.len());
    // The same picture of the same signal, from four times the audio.
    for at in 0..low.columns.len() {
        assert!(
            (low.columns[at].rms - high.columns[at].rms).abs() < 1e-3,
            "column {at}"
        );
    }
    assert!(at_48.as_millis() < 500, "48 kHz draw took {at_48:?}");
    assert!(at_192.as_millis() < 500, "192 kHz draw took {at_192:?}");
    println!("  10 s at 40 px: 48 kHz {at_48:?}, 192 kHz {at_192:?}");
}

/// The other half of §37, and the half a mid-zoom view depends on.
///
/// Between one sample and one block per column the read is proportional to the
/// samples in the *span* - that is unavoidable, since that is the detail being
/// asked for. What it must never be proportional to is the length of the
/// recording the span was cut out of, and this is where that is checked: the
/// same five seconds, drawn out of a ten-second capture and out of one twenty
/// times longer, costs the same because the index seeks straight to it.
#[test]
fn the_cost_is_set_by_the_span_and_not_by_the_recording_it_came_from() {
    let short = tempfile::tempdir().expect("tempdir");
    let long = tempfile::tempdir().expect("tempdir");
    let (a, id_a) = capture(&short, 48_000, &positional(48_000 * 10));
    let (b, id_b) = capture(&long, 48_000, &positional(48_000 * 200));

    // Five seconds at 1000 px: 240 frames a column, fine enough to need the
    // 256 level and therefore to open blobs. The expensive case, on purpose.
    let span = Request::new(48_000, 48_000 * 6, 1_000);
    let draw = |p: &Project, id: i64| {
        let started = Instant::now();
        let drawn = waveform::read(p.conn(), id, 0, &span).expect("read");
        (drawn, started.elapsed())
    };
    // Warm the page cache and the prepared statement for both, so what is
    // being compared is the read and not the first-touch cost.
    draw(&a, id_a);
    draw(&b, id_b);
    let (from_short, short_time) = draw(&a, id_a);
    let (from_long, long_time) = draw(&b, id_b);

    assert_eq!(from_short.level, Level::Samples);
    assert_eq!(from_short.columns.len(), from_long.columns.len());
    assert_eq!(
        from_short.covered, from_long.covered,
        "the same span must read the same number of frames out of both"
    );
    for at in 0..from_short.columns.len() {
        assert_eq!(from_short.columns[at], from_long.columns[at], "column {at}");
    }
    println!(
        "  5 s at 1000 px: from a 10 s capture {short_time:?}, \
         from a 200 s capture {long_time:?}"
    );
    // Twenty times the recording. A read that walked it would be twenty times
    // slower; the bound here is deliberately far looser than that, because a
    // tight timing assertion on a shared machine is a flaky test rather than a
    // strong one. The measured figures are printed above and quoted in
    // docs/STATUS.md.
    assert!(
        long_time < short_time * 8 + std::time::Duration::from_millis(50),
        "drawing out of the longer capture cost {long_time:?} against {short_time:?}"
    );
}

/// Zooming in reads the audio; zooming out never does.
#[test]
fn the_level_follows_the_zoom_and_the_cost_follows_the_level() {
    let dir = tempfile::tempdir().expect("tempdir");
    let frames = 12_000 * 20;
    let (project, id) = capture(&dir, 48_000, &half_loud(frames));
    let shape = Shape::of(project.conn(), id).expect("shape");

    // Whole capture at 500 px: 480 frames a column, too fine for a block.
    let whole = Request::new(0, frames, 500);
    assert_eq!(whole.level(shape.levels), Level::Summary256);
    // Whole capture at 20 px: one block a column.
    assert_eq!(
        Request::new(0, frames, 20).level(shape.levels),
        Level::Block
    );
    // A tenth of a second at 500 px: under ten frames a column, so the audio.
    assert_eq!(
        Request::new(0, 4_800, 500).level(shape.levels),
        Level::Samples
    );

    // And the zoomed-in read really is reading samples: a 200-frame window
    // over the square wave alternates every column.
    let close = Request::new(1_000, 1_020, 20);
    let drawn = waveform::read(project.conn(), id, 0, &close).expect("read");
    assert_eq!(drawn.level, Level::Samples);
    assert!(drawn.columns.iter().all(|c| c.frames == 1));
    assert_eq!(drawn.columns[0].max, drawn.columns[0].min, "one sample");
    assert_ne!(
        drawn.columns[0].max, drawn.columns[1].max,
        "consecutive samples of a square wave differ"
    );
}

/// §19's "regenerated from source PCM when required", exercised by destroying
/// the pyramid and asking for it back.
#[test]
fn the_pyramid_can_be_thrown_away_and_rebuilt_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let frames = 12_000 * 6 + 3_777;
    let audio = pcm(frames, |frame, channel| {
        (((frame * 37 + u64::from(channel) * 11) % 60_000) as i64 - 30_000) as i16
    });
    let (mut project, id) = capture(&dir, 48_000, &audio);

    let request = Request::new(0, frames, 96);
    let before: Vec<Waveform> = (0..2)
        .map(|ch| waveform::read(project.conn(), id, ch, &request).expect("read"))
        .collect();

    // Wipe every summary the writer computed. The audio is untouched, which is
    // the point: the pyramid is derived, not a second copy of the truth.
    project
        .conn()
        .execute(
            "UPDATE sampleblocks SET summin = 0, summax = 0, sumrms = 0, \
             summary256 = NULL, summary64k = NULL",
            [],
        )
        .expect("wipe");
    let wiped = waveform::read(project.conn(), id, 0, &request).expect("read");
    assert!(
        wiped.columns.iter().all(|c| c.max == 0.0),
        "the wipe must really have removed the drawing"
    );

    let done = waveform::rebuild(&mut project, id, Rebuild::All).expect("rebuild");
    assert_eq!(done.rewritten, done.examined);
    assert_eq!(done.silent, 0);
    assert!(done.examined >= 14, "seven blocks a channel, got {done:?}");

    for (ch, was) in before.iter().enumerate() {
        let now = waveform::read(project.conn(), id, ch as u16, &request).expect("read");
        assert_eq!(&now, was, "channel {ch} did not come back the same");
    }
}

/// A capture recorded with summaries off still draws, and can be filled in.
#[test]
fn a_capture_with_no_pyramid_draws_from_the_blocks_and_can_be_repaired() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bare.vcw");
    let project = Project::create(&path).expect("create");
    let info = info(48_000);
    let config = Config {
        summaries: false,
        ..Config::default()
    };
    let mut writer = Writer::begin(project, &info, config).expect("begin");
    let frames = 12_000 * 4;
    writer.push(&half_loud(frames)).expect("push");
    let (outcome, mut project, _) = writer
        .finish_with_project(CaptureState::Finalised)
        .expect("finish");
    let id = outcome.capture_id;

    // 256 would have been chosen, and there is no 256 level to read. The
    // whole-block triplet is always written, so the drawing degrades in
    // resolution rather than disappearing.
    // 100 columns of 480 frames: too coarse for the audio, too fine for a
    // block, so 256 is what would be read if there were a 256 level.
    let request = Request::new(0, frames, 100);
    let shape = Shape::of(project.conn(), id).expect("shape");
    assert_eq!(request.level(shape.levels), Level::Summary256);
    let coarse = waveform::read(project.conn(), id, 0, &request).expect("read");
    assert_eq!(coarse.covered, frames, "every frame is still accounted for");
    assert!(coarse.columns.iter().all(|c| !c.is_empty()));

    // Fill it in, and the same request gets the detail it asked for.
    let done = waveform::rebuild(&mut project, id, Rebuild::Missing).expect("rebuild");
    assert_eq!(done.rewritten, 8, "four blocks, two channels");
    let fine = waveform::read(project.conn(), id, 0, &request).expect("read");
    assert_eq!(fine.covered, frames);

    // The square wave's first half is flat at block resolution and flat at 256
    // too, so the proof that the detail arrived is the silent half's boundary:
    // at 400 columns the transition lands inside a column either way, but the
    // coarse read smears the whole block across all of its columns.
    let loud_coarse = coarse.columns.iter().filter(|c| c.max > 0.5).count();
    let loud_fine = fine.columns.iter().filter(|c| c.max > 0.5).count();
    assert!(
        loud_fine <= loud_coarse,
        "the finer level cannot claim more loud columns than the coarser one: \
         {loud_fine} against {loud_coarse}"
    );
    assert_eq!(loud_fine, 50, "half the columns, to the column");
}

/// Reading a capture that is not there says so, rather than drawing silence.
#[test]
fn an_unknown_capture_is_an_error_and_not_an_empty_picture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (project, id) = capture(&dir, 48_000, &half_loud(12_000));
    assert!(session::load(project.conn(), id).expect("load").is_some());

    let request = Request::new(0, 12_000, 10);
    match waveform::read(project.conn(), id + 1, 0, &request) {
        Err(vcw_project::Error::NoSuchCapture { capture_id }) => assert_eq!(capture_id, id + 1),
        other => panic!("expected NoSuchCapture, got {other:?}"),
    }

    // A channel that does not exist is a different thing: the capture is real,
    // so the honest answer is a picture with nothing in it.
    let empty = waveform::read(project.conn(), id, 9, &request).expect("read");
    assert_eq!(empty.columns.len(), 10);
    assert!(empty.columns.iter().all(Column::is_empty));
}

/// §20 draws two channels against one time axis, so they must agree on it.
#[test]
fn both_channels_come_back_on_the_same_axis() {
    let dir = tempfile::tempdir().expect("tempdir");
    let frames = 12_000 * 3;
    // Left loud, right quiet, so a swap would be visible.
    let audio = pcm(
        frames,
        |_, channel| {
            if channel == 0 { 30_000 } else { 3_000 }
        },
    );
    let (project, id) = capture(&dir, 48_000, &audio);

    let request = Request::new(0, frames, 60);
    let both = waveform::read_all(project.conn(), id, &request).expect("read all");
    assert_eq!(both.len(), 2);
    assert_eq!(both[0].level, both[1].level, "one level for one time axis");
    assert_eq!(both[0].frames_per_pixel, both[1].frames_per_pixel);
    assert!(both[0].peak() > 0.9);
    assert!(
        (both[1].peak() - 0.0916).abs() < 0.001,
        "{}",
        both[1].peak()
    );
}
