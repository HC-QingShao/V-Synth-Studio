//! Minimal Standard MIDI File writer.
//!
//! Byte-for-byte equivalent to what `mido.MidiFile` produces for the layout
//! `inference/callbacks.py:SaveCombinedMidiFileCallback.flush` builds:
//!
//! ```text
//! MThd  len=6  format=0  ntrks=1  division=480      (mido's default tpb)
//! MTrk  len=.. set_tempo(500000)  note_on/note_off...
//! ```
//!
//! Timing: the upstream callback stores tick counts as `onset_seconds * tempo *
//! 8` while writing `set_tempo(bpm2tempo(120))`, i.e. 500000 us/beat. Together
//! with mido's default 480 ticks/beat that is exactly **960 ticks per second**,
//! which is what this module uses (`TICKS_PER_SECOND`). Keeping the arithmetic
//! in ticks-per-second form means the numbers in the file are the seconds from
//! the model, only scaled once.

/// `mido.MidiFile` default `ticks_per_beat`.
pub const TICKS_PER_BEAT: u16 = 480;
/// 120 BPM at 480 tpb => 480 * (120 / 60) = 960 ticks per second.
pub const TICKS_PER_SECOND: f64 = TICKS_PER_BEAT as f64 * 2.0;
/// `mido.bpm2tempo(120)` in microseconds per beat.
pub const TEMPO_US_PER_BEAT: u32 = 500_000;

/// One transcribed note. `pitch` is a MIDI semitone number (A4 = 69) that may
/// be fractional before rounding, matching upstream's `round(note.pitch)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub onset: f64,
    pub offset: f64,
    pub pitch: f32,
}

/// `SaveCombinedFileCallback.save_file`'s de-overlap pass.
///
/// Notes are sorted by `(onset, offset, pitch)` and then walked in order,
/// clamping each note to start no earlier than the previous note's end and
/// dropping anything that collapses to zero or negative length. This is what
/// makes the upstream output monophonic, which is a property of the *callback*,
/// not of the model -- keep it if mono output is wanted, skip it for polyphony.
pub fn deoverlap_mono(mut notes: Vec<Note>) -> Vec<Note> {
    notes.sort_by(|a, b| {
        a.onset
            .partial_cmp(&b.onset)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.offset
                    .partial_cmp(&b.offset)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                a.pitch
                    .partial_cmp(&b.pitch)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    let mut out: Vec<Note> = Vec::with_capacity(notes.len());
    let mut last_time = 0f64;
    for mut n in notes {
        n.onset = n.onset.max(last_time);
        n.offset = n.offset.max(n.onset);
        if n.offset <= n.onset {
            continue;
        }
        last_time = n.offset;
        out.push(n);
    }
    out
}

/// Encode a variable-length quantity the way SMF delta times require.
fn write_vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 5];
    let mut i = 4usize;
    buf[i] = (v & 0x7f) as u8;
    v >>= 7;
    while v > 0 {
        i -= 1;
        buf[i] = ((v & 0x7f) as u8) | 0x80;
        v >>= 7;
    }
    out.extend_from_slice(&buf[i..]);
}

/// Build a format-0, single-track MIDI file.
///
/// Notes are expected to already be de-overlapped; callers that want the
/// upstream mono behaviour should run [`deoverlap_mono`] first.
pub fn write_midi(notes: &[Note]) -> Vec<u8> {
    let mut track: Vec<u8> = Vec::new();

    // set_tempo, delta 0
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xff, 0x51, 0x03]);
    track.extend_from_slice(&TEMPO_US_PER_BEAT.to_be_bytes()[1..]);

    let mut last_tick: i64 = 0;
    for n in notes {
        let onset_tick = (n.onset * TICKS_PER_SECOND).round() as i64;
        let offset_tick = (n.offset * TICKS_PER_SECOND).round() as i64;
        if offset_tick <= onset_tick {
            continue;
        }
        let pitch = (n.pitch.round().clamp(0.0, 127.0)) as u8;
        // note_on, then note_off; both on channel 0.
        write_vlq(&mut track, (onset_tick - last_tick).max(0) as u32);
        track.extend_from_slice(&[0x90, pitch, 0x40]);
        write_vlq(&mut track, (offset_tick - onset_tick).max(0) as u32);
        track.extend_from_slice(&[0x80, pitch, 0x00]);
        last_tick = offset_tick;
    }

    // end_of_track
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xff, 0x2f, 0x00]);

    let mut out = Vec::with_capacity(22 + track.len());
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // format 0
    out.extend_from_slice(&1u16.to_be_bytes()); // one track
    out.extend_from_slice(&TICKS_PER_BEAT.to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(&track);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_encoding_matches_smf() {
        let mut v = Vec::new();
        write_vlq(&mut v, 0);
        assert_eq!(v, vec![0x00]);
        v.clear();
        write_vlq(&mut v, 127);
        assert_eq!(v, vec![0x7f]);
        v.clear();
        write_vlq(&mut v, 128);
        assert_eq!(v, vec![0x81, 0x00]);
        v.clear();
        write_vlq(&mut v, 960);
        assert_eq!(v, vec![0x87, 0x40]);
        v.clear();
        write_vlq(&mut v, 0x0fffffff);
        assert_eq!(v, vec![0xff, 0xff, 0xff, 0x7f]);
    }

    #[test]
    fn midi_header_is_wellformed() {
        let bytes = write_midi(&[Note { onset: 0.0, offset: 0.5, pitch: 60.0 }]);
        assert_eq!(&bytes[0..4], b"MThd");
        assert_eq!(&bytes[4..8], &6u32.to_be_bytes());
        assert_eq!(&bytes[8..10], &0u16.to_be_bytes());
        assert_eq!(&bytes[10..12], &1u16.to_be_bytes());
        assert_eq!(&bytes[12..14], &480u16.to_be_bytes());
        assert_eq!(&bytes[14..18], b"MTrk");
        let tlen = u32::from_be_bytes(bytes[18..22].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 22 + tlen);
        // set_tempo meta: delta 0x00, 0xff, 0x51, length 0x03, then 500000
        // (= 0x07A120) big-endian.
        assert_eq!(&bytes[22..25], &[0x00, 0xff, 0x51]);
        assert_eq!(&bytes[25..29], &[0x03, 0x07, 0xa1, 0x20]);
    }

    #[test]
    fn one_second_note_lands_on_960_ticks() {
        let bytes = write_midi(&[Note { onset: 0.0, offset: 1.0, pitch: 69.0 }]);
        // after the 7-byte tempo meta: 0x00 0x90 0x45 0x40 0x87 0x40 0x80 0x45 0x00
        let body = &bytes[22..];
        assert_eq!(body[7], 0x00); // note_on delta 0
        assert_eq!(body[8], 0x90);
        assert_eq!(body[9], 69);
        assert_eq!(body[11], 0x87); // 960 as VLQ
        assert_eq!(body[12], 0x40);
        assert_eq!(body[13], 0x80);
    }

    #[test]
    fn deoverlap_makes_notes_monophonic() {
        let notes = vec![
            Note { onset: 0.0, offset: 0.5, pitch: 60.0 },
            Note { onset: 0.2, offset: 0.8, pitch: 64.0 },
        ];
        let out = deoverlap_mono(notes);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].onset, 0.5); // pushed to start where the first ended
    }
}
