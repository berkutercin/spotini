//! Audio output: librespot's rodio sink, with the decoding thread's priority
//! raised so it keeps up while the CPU is busy, and reopened when Windows'
//! default output device changes (rodio otherwise stays on the device it
//! opened, e.g. the speakers after switching to headphones).
//!
//! Stutter had two causes, both measured with `examples/stutter_probe.rs`:
//! - cpal's output thread never got its priority boost (fixed by cpal's
//!   `audio_thread_priority` feature, see Cargo.toml);
//! - librespot's player thread, which decodes and writes to this sink, ran at
//!   normal priority and fell behind the ~0.5 s output queue under load.

use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};
use librespot_playback::audio_backend::{self, Sink, SinkBuilder, SinkResult};
use librespot_playback::config::AudioFormat;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;

/// How often playback checks whether the default device changed.
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

struct PrioritySink {
    make_sink: SinkBuilder,
    inner: Box<dyn Sink>,
    /// The default device's name when `inner` was opened.
    device: Option<String>,
    checked_at: Instant,
    boosted: bool,
}

impl PrioritySink {
    /// Reopens the output on the new default device if it changed.
    fn follow_default_device(&mut self) {
        self.checked_at = Instant::now();
        let device = default_device_name();
        // No device at all (everything unplugged): keep the old stream rather than fail.
        if device.is_none() || device == self.device {
            return;
        }
        log::info!("default output changed to {device:?}; reopening");
        self.inner = (self.make_sink)(None, AudioFormat::default());
        self.device = device;
    }
}

impl Sink for PrioritySink {
    fn start(&mut self) -> SinkResult<()> {
        self.follow_default_device();
        self.inner.start()
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.inner.stop()
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        // `write` runs on librespot's player thread; boost it the first time we see it.
        if !self.boosted {
            boost_current_thread();
            self.boosted = true;
        }
        if self.checked_at.elapsed() >= DEVICE_CHECK_INTERVAL {
            self.follow_default_device();
        }
        self.inner.write(packet, converter)
    }
}

/// The default (rodio) output wrapped so the player thread gets a higher priority.
pub fn open() -> Box<dyn Sink> {
    // rodio is librespot's default backend and always compiled in.
    let make_sink = audio_backend::find(None).expect("librespot has a default audio backend");
    Box::new(PrioritySink {
        make_sink,
        device: default_device_name(),
        inner: make_sink(None, AudioFormat::default()),
        checked_at: Instant::now(),
        boosted: false,
    })
}

/// The name of the system's default output device, the one rodio opens.
fn default_device_name() -> Option<String> {
    cpal::default_host().default_output_device()?.name().ok()
}

/// Raises the calling thread above normal priority.
pub fn boost_current_thread() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
        // SAFETY: GetCurrentThread returns a pseudo-handle that is always valid for the calling thread.
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } == 0 {
            log::warn!("could not raise the player thread's priority");
        }
    }
}
