//! The play queue: which track comes next, with shuffle and repeat. Lives in
//! Rust so playback keeps advancing while the window (and its WebView) is closed.

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::library::Track;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

pub struct Queue {
    tracks: Vec<Track>,
    /// Which list the queue came from, e.g. "liked" or a playlist ID.
    pub source_id: String,
    /// That list's name, for "playing from".
    pub source_name: String,
    /// Indices into `tracks` (playable only), in play order.
    order: Vec<usize>,
    /// Position in `order` of the current track.
    pos: usize,
}

/// Play order over `members` (indices into the tracks); with shuffle, `first`
/// leads and the rest are mixed, otherwise they go in list order.
fn build_order(mut members: Vec<usize>, first: usize, shuffle: bool) -> Vec<usize> {
    if !shuffle {
        members.sort_unstable();
        return members;
    }
    let mut rest: Vec<usize> = members.into_iter().filter(|&i| i != first).collect();
    rest.shuffle(&mut rand::rng());
    std::iter::once(first).chain(rest).collect()
}

impl Queue {
    /// Starts at `index`, or the next playable track after it. `None` if nothing is playable.
    pub fn new(tracks: Vec<Track>, index: usize, source_id: String, shuffle: bool) -> Option<Self> {
        let start = (index..tracks.len()).find(|&i| tracks[i].playable)?;
        let playable = tracks.iter().enumerate().filter(|(_, t)| t.playable).map(|(i, _)| i).collect();
        let order = build_order(playable, start, shuffle);
        let pos = order.iter().position(|&i| i == start)?;
        Some(Self {
            tracks,
            source_name: source_id.clone(),
            source_id,
            order,
            pos,
        })
    }

    /// Number of playable tracks.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Playable tracks still to come after the current one.
    pub fn remaining(&self) -> usize {
        self.order.len() - self.pos - 1
    }

    /// Every track URI in the queue, in play order.
    pub fn uris(&self) -> Vec<String> {
        self.order.iter().map(|&i| self.tracks[i].uri.clone()).collect()
    }

    /// Adds tracks to the end, skipping unplayable ones and ones already queued (radio top-ups).
    pub fn append(&mut self, tracks: Vec<Track>) {
        let queued: std::collections::HashSet<String> = self.tracks.iter().map(|t| t.uri.clone()).collect();
        for track in tracks {
            if track.playable && !queued.contains(&track.uri) {
                self.order.push(self.tracks.len());
                self.tracks.push(track);
            }
        }
    }

    /// Puts tracks right after the current one, in the given order. Unplayable
    /// ones are skipped; tracks already queued are queued again.
    pub fn insert_next(&mut self, tracks: Vec<Track>) {
        let at = self.pos + 1;
        let start = self.tracks.len();
        self.tracks.extend(tracks.into_iter().filter(|t| t.playable));
        self.order.splice(at..at, start..self.tracks.len());
    }

    /// Drops the track `steps` places after the current one (1 = next).
    pub fn remove_upcoming(&mut self, steps: usize) -> bool {
        let pos = self.pos + steps;
        if steps == 0 || pos >= self.order.len() {
            return false;
        }
        self.order.remove(pos);
        true
    }

    pub fn current(&self) -> &Track {
        &self.tracks[self.order[self.pos]]
    }

    /// Where the queue goes after the current track, without moving. `auto` is
    /// true when the current track ended on its own (repeat-one replays it).
    fn next_pos(&self, repeat: Repeat, auto: bool) -> Option<usize> {
        if auto && repeat == Repeat::One {
            Some(self.pos)
        } else if self.pos + 1 < self.order.len() {
            Some(self.pos + 1)
        } else if repeat != Repeat::Off {
            Some(0)
        } else {
            None
        }
    }

    /// The track that would play next, for preloading.
    pub fn peek_next(&self, repeat: Repeat) -> Option<&Track> {
        self.next_pos(repeat, true).map(|pos| &self.tracks[self.order[pos]])
    }

    /// Moves to the next track. `None` means the queue has ended.
    pub fn advance(&mut self, repeat: Repeat, shuffle: bool, auto: bool) -> Option<&Track> {
        let pos = self.next_pos(repeat, auto)?;
        if pos == 0 && self.pos + 1 == self.order.len() && shuffle {
            // Wrapping around with shuffle: mix again for the next round.
            self.order.shuffle(&mut rand::rng());
        }
        self.pos = pos;
        Some(self.current())
    }

    /// Moves to the previous track. `None` at the start (unless repeating all).
    pub fn back(&mut self, repeat: Repeat) -> Option<&Track> {
        if self.pos > 0 {
            self.pos -= 1;
        } else if repeat == Repeat::All {
            self.pos = self.order.len() - 1;
        } else {
            return None;
        }
        Some(self.current())
    }

    /// The tracks after the current one, in play order.
    pub fn upcoming(&self, limit: usize) -> Vec<Track> {
        self.order[self.pos + 1..]
            .iter()
            .take(limit)
            .map(|&i| self.tracks[i].clone())
            .collect()
    }

    /// Skips ahead to the track `steps` places after the current one (1 = next).
    pub fn jump(&mut self, steps: usize) -> Option<&Track> {
        let pos = self.pos + steps;
        if steps == 0 || pos >= self.order.len() {
            return None;
        }
        self.pos = pos;
        Some(self.current())
    }

    /// Re-orders around the current track so playback continues seamlessly.
    pub fn set_shuffle(&mut self, shuffle: bool) {
        let current = self.order[self.pos];
        self.order = build_order(self.order.clone(), current, shuffle);
        self.pos = self.order.iter().position(|&i| i == current).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(playable: &[bool]) -> Vec<Track> {
        playable
            .iter()
            .enumerate()
            .map(|(i, &p)| Track::for_test(&format!("t{i}"), p))
            .collect()
    }

    fn uri(track: Option<&Track>) -> Option<&str> {
        track.map(|t| t.uri.as_str())
    }

    #[test]
    fn starts_at_next_playable_track() {
        let q = Queue::new(tracks(&[true, false, true]), 1, "s".into(), false).unwrap();
        assert_eq!(q.current().uri, "t2");
        assert!(Queue::new(tracks(&[false, false]), 0, "s".into(), false).is_none());
    }

    #[test]
    fn advances_skipping_unplayable_and_stops_at_end() {
        let mut q = Queue::new(tracks(&[true, false, true]), 0, "s".into(), false).unwrap();
        assert_eq!(uri(q.advance(Repeat::Off, false, true)), Some("t2"));
        assert_eq!(uri(q.advance(Repeat::Off, false, true)), None);
        assert_eq!(q.current().uri, "t2");
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_replays_only_when_auto() {
        let mut q = Queue::new(tracks(&[true, true]), 1, "s".into(), false).unwrap();
        assert_eq!(uri(q.advance(Repeat::All, false, true)), Some("t0"));
        assert_eq!(uri(q.advance(Repeat::One, false, true)), Some("t0"));
        assert_eq!(uri(q.advance(Repeat::One, false, false)), Some("t1"));
    }

    #[test]
    fn back_stops_at_start_unless_repeating_all() {
        let mut q = Queue::new(tracks(&[true, true, true]), 0, "s".into(), false).unwrap();
        assert_eq!(uri(q.back(Repeat::Off)), None);
        assert_eq!(uri(q.back(Repeat::All)), Some("t2"));
        assert_eq!(uri(q.back(Repeat::Off)), Some("t1"));
    }

    #[test]
    fn shuffle_keeps_current_first_and_covers_every_playable_track() {
        let mut q = Queue::new(tracks(&[true, true, false, true, true]), 3, "s".into(), true).unwrap();
        assert_eq!(q.current().uri, "t3");
        let mut seen = vec![q.current().uri.clone()];
        while let Some(t) = q.advance(Repeat::Off, true, true) {
            seen.push(t.uri.clone());
        }
        seen.sort();
        assert_eq!(seen, ["t0", "t1", "t3", "t4"]);
    }

    #[test]
    fn turning_shuffle_off_keeps_the_current_track() {
        let mut q = Queue::new(tracks(&[true, true, true, true]), 2, "s".into(), true).unwrap();
        q.set_shuffle(false);
        assert_eq!(q.current().uri, "t2");
        assert_eq!(uri(q.advance(Repeat::Off, false, false)), Some("t3"));
    }

    #[test]
    fn lists_upcoming_tracks_in_play_order() {
        let q = Queue::new(tracks(&[true, true, false, true, true]), 1, "s".into(), false).unwrap();
        let upcoming: Vec<_> = q.upcoming(10).into_iter().map(|t| t.uri).collect();
        assert_eq!(upcoming, ["t3", "t4"]);
        assert_eq!(q.upcoming(1).len(), 1);
    }

    #[test]
    fn jumps_ahead_within_the_queue_only() {
        let mut q = Queue::new(tracks(&[true, true, true]), 0, "s".into(), false).unwrap();
        assert_eq!(uri(q.jump(2)), Some("t2"));
        assert_eq!(uri(q.jump(1)), None);
        assert_eq!(uri(q.jump(0)), None);
        assert_eq!(q.current().uri, "t2");
    }

    #[test]
    fn appends_new_playable_tracks_after_the_current_order() {
        let mut q = Queue::new(tracks(&[true, true]), 0, "radio".into(), false).unwrap();
        let mut more = tracks(&[true, false, true]);
        more[0].uri = "t1".into(); // already queued
        more[2].uri = "x".into();
        q.append(more);
        assert_eq!(q.uris(), ["t0", "t1", "x"]);
        assert_eq!(q.remaining(), 2);
        assert_eq!(uri(q.jump(2)), Some("x"));
    }

    #[test]
    fn inserts_tracks_to_play_next_in_order() {
        let mut q = Queue::new(tracks(&[true, true, true]), 0, "s".into(), false).unwrap();
        let mut more = tracks(&[true, false, true]);
        more[0].uri = "x".into();
        more[2].uri = "y".into();
        q.insert_next(more);
        assert_eq!(q.uris(), ["t0", "x", "y", "t1", "t2"]);
        assert_eq!(uri(q.advance(Repeat::Off, false, true)), Some("x"));
    }

    #[test]
    fn removes_upcoming_tracks_and_keeps_them_out_when_reshuffling() {
        let mut q = Queue::new(tracks(&[true, true, true, true]), 0, "s".into(), false).unwrap();
        assert!(q.remove_upcoming(2));
        assert!(!q.remove_upcoming(0));
        assert!(!q.remove_upcoming(3));
        assert_eq!(q.uris(), ["t0", "t1", "t3"]);
        q.set_shuffle(true);
        q.set_shuffle(false);
        assert_eq!(q.uris(), ["t0", "t1", "t3"]);
    }

    #[test]
    fn peeks_without_moving() {
        let q = Queue::new(tracks(&[true, true]), 0, "s".into(), false).unwrap();
        assert_eq!(uri(q.peek_next(Repeat::Off)), Some("t1"));
        assert_eq!(q.current().uri, "t0");
    }
}
