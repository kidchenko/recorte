//! The sound of a recording: what the computer plays and what the microphone
//! hears, mixed into one stereo track.
//!
//! Both come from WASAPI, which is asked to convert whatever the devices run
//! at to the one format the film uses, so mixing is adding two numbers. What
//! the computer plays is its default output device opened as a loopback.
//!
//! The recorder's thread asks for the mix once a frame, by the film's clock:
//! "the sound up to now". That clock, not the devices', decides how much
//! sound there is, so the track is as long as the picture whatever the
//! devices do. A loopback is silent by sending nothing at all, and a device
//! can be unplugged; both come out as silence. To have a few frames' worth
//! of slack for sound that arrives late, the mix runs a little behind now,
//! and each source keeps that much waiting.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use traykit::lwarn;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// Frames a second, and samples a frame: 16 bits each, left then right.
pub const RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;
const WIDE: usize = CHANNELS as usize;
/// `WAVE_FORMAT_PCM` in the headers.
const PCM: u16 = 1;

/// How far behind now the mix runs, in frames: a tenth of a second, three
/// of the recorder's turns.
const LAG: u64 = RATE as u64 / 10;
/// The most a source keeps waiting beyond what the mix is behind by, in
/// samples: half a second. More than that is a device whose clock runs
/// ahead of the film's.
const MOST: usize = RATE as usize / 2 * WIDE;
/// How long until a device that would not open is tried again.
const RETRY: Duration = Duration::from_secs(3);
/// How much a device holds between two visits, in units of 100 ns: a second.
const HOLD: i64 = 10_000_000;

/// What one source has heard and the mix has not yet taken.
#[derive(Default)]
struct Queue(VecDeque<i16>);

impl Queue {
    /// `heard` arrived just now, when the mix is `behind` frames short of
    /// now.
    fn push(&mut self, heard: &[i16], behind: u64) {
        if heard.is_empty() {
            return;
        }
        if self.0.is_empty() {
            // The first sound, or the first after a silence: it ends now,
            // so that much silence goes before it.
            let frames = (heard.len() / WIDE) as u64;
            let silence = behind.saturating_sub(frames) as usize * WIDE;
            self.0.extend(std::iter::repeat_n(0, silence));
        }
        self.0.extend(heard);
        // After a slow moment the mix is far behind and wants all of this;
        // only what is more than it could want is a clock running ahead.
        let wanted = behind.max(LAG) as usize * WIDE;
        if self.0.len() > wanted + MOST {
            self.0.drain(..self.0.len() - wanted);
        }
    }

    /// Add the next of it to `out`; where there is none, `out` is left.
    fn mix_into(&mut self, out: &mut [i16]) {
        let have = out.len().min(self.0.len());
        for (o, s) in out.iter_mut().zip(self.0.drain(..have)) {
            *o = o.saturating_add(s);
        }
    }
}

/// A device being listened to. Dropped, it is let go.
struct Device {
    client: IAudioClient,
    capture: IAudioCaptureClient,
}

impl Device {
    /// The default device for `flow`: the microphone for `eCapture`, and
    /// for `eRender` what the speakers are sent.
    unsafe fn open(flow: EDataFlow) -> windows::core::Result<Self> {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(flow, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let align = (WIDE * 2) as u16;
        let format = WAVEFORMATEX {
            wFormatTag: PCM,
            nChannels: CHANNELS as u16,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * align as u32,
            nBlockAlign: align,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut flags =
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        if flow == eRender {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, HOLD, 0, &format, None)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok(Self { client, capture })
    }

    /// Everything it has heard since the last time, onto the end of `into`.
    unsafe fn read(&self, into: &mut Vec<i16>) -> windows::core::Result<()> {
        while self.capture.GetNextPacketSize()? > 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
            self.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let samples = frames as usize * WIDE;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                into.resize(into.len() + samples, 0);
            } else {
                // The device's own buffer, laid out as the format asked for.
                into.extend_from_slice(std::slice::from_raw_parts(data as *const i16, samples));
            }
            self.capture.ReleaseBuffer(frames)?;
        }
        Ok(())
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

/// The microphone, or what the computer plays: its device while it is
/// wanted, and what that has heard.
struct Source {
    flow: EDataFlow,
    name: &'static str,
    device: Option<Device>,
    /// It could not be opened, or stopped working (unplugged, say): when to
    /// try again, by then perhaps with another default device, and whether
    /// the log has it already.
    again: Option<Instant>,
    complained: bool,
    queue: Queue,
}

impl Source {
    fn new(flow: EDataFlow, name: &'static str) -> Self {
        Self {
            flow,
            name,
            device: None,
            again: None,
            complained: false,
            queue: Queue::default(),
        }
    }

    /// Open the device if it is `wanted` and let it go if not, which is
    /// what takes the microphone's light off the taskbar. Then take what it
    /// has heard: for the mix, or with `keep` off, to throw away.
    fn listen(&mut self, wanted: bool, keep: bool, behind: u64, heard: &mut Vec<i16>) {
        if !wanted {
            self.device = None;
            self.again = None;
            self.complained = false;
            self.queue.0.clear();
            return;
        }
        let now = Instant::now();
        if self.device.is_none() && self.again.is_none_or(|at| now >= at) {
            match unsafe { Device::open(self.flow) } {
                Ok(device) => {
                    self.device = Some(device);
                    self.again = None;
                    self.complained = false;
                }
                Err(e) => self.fail(now, format!("recording without {}: {e}", self.name)),
            }
        }
        let Some(device) = &self.device else {
            return;
        };
        heard.clear();
        if let Err(e) = unsafe { device.read(heard) } {
            self.fail(now, format!("{} stopped: {e}", self.name));
        } else if keep {
            self.queue.push(heard, behind);
        } else {
            self.queue.0.clear();
        }
    }
}

impl Source {
    /// Without its device until it can be opened again. Said once in the
    /// log, not at every try.
    fn fail(&mut self, now: Instant, why: String) {
        if !self.complained {
            lwarn!("{why}");
            self.complained = true;
        }
        self.device = None;
        self.again = Some(now + RETRY);
    }
}

pub struct Sound {
    mic: Source,
    system: Source,
    /// Frames of the track mixed so far.
    written: u64,
    heard: Vec<i16>,
    mixed: Vec<i16>,
}

impl Sound {
    pub fn new() -> Self {
        Self {
            mic: Source::new(eCapture, "the microphone"),
            system: Source::new(eRender, "the computer's sound"),
            written: 0,
            heard: Vec::new(),
            mixed: Vec::new(),
        }
    }

    /// The next of the track, with the film `at` this long: the frame it
    /// starts at and its samples, which may be none. `last` brings it right
    /// up to now, for the end of the film.
    pub fn mix(&mut self, at: Duration, mic: bool, system: bool, last: bool) -> (u64, &[i16]) {
        let now = frames_in(at);
        let behind = now.saturating_sub(self.written);
        self.mic.listen(mic, true, behind, &mut self.heard);
        self.system.listen(system, true, behind, &mut self.heard);

        let upto = if last { now } else { now.saturating_sub(LAG) };
        let frames = upto.saturating_sub(self.written);
        self.mixed.clear();
        self.mixed.resize(frames as usize * WIDE, 0);
        self.mic.queue.mix_into(&mut self.mixed);
        self.system.queue.mix_into(&mut self.mixed);
        let first = self.written;
        self.written += frames;
        (first, &self.mixed)
    }

    /// Have the devices as wanted, and forget what they have heard: before
    /// the film starts, and while it is paused.
    pub fn skip(&mut self, mic: bool, system: bool) {
        self.mic.listen(mic, false, 0, &mut self.heard);
        self.system.listen(system, false, 0, &mut self.heard);
    }
}

/// How many frames of sound a film this long has.
fn frames_in(time: Duration) -> u64 {
    (time.as_nanos() * RATE as u128 / 1_000_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_is_counted_by_the_clock() {
        assert_eq!(frames_in(Duration::from_secs(2)), 96_000);
        assert_eq!(frames_in(Duration::from_millis(33)), 1_584);
    }

    #[test]
    fn the_first_sound_ends_now() {
        let mut q = Queue::default();
        // Two frames heard when the mix is five behind: three of silence.
        q.push(&[1, 1, 2, 2], 5);
        assert_eq!(q.0, [0, 0, 0, 0, 0, 0, 1, 1, 2, 2]);
        // What comes next follows on, with no silence between.
        q.push(&[3, 3], 9);
        assert_eq!(q.0.len(), 12);
        // More heard than the mix is behind: none to add.
        let mut q = Queue::default();
        q.push(&[1, 1, 2, 2], 1);
        assert_eq!(q.0, [1, 1, 2, 2]);
    }

    #[test]
    fn two_sources_add_up_and_a_short_one_is_silence() {
        let (mut a, mut b) = (Queue::default(), Queue::default());
        a.push(&[100, -100, i16::MAX, i16::MIN], 0);
        b.push(&[5, 5, 1, -1, 7, 7], 0);
        let mut out = [0; 8];
        a.mix_into(&mut out);
        b.mix_into(&mut out);
        // Too loud stays at the limit and does not wrap around.
        assert_eq!(out, [105, -95, i16::MAX, i16::MIN, 7, 7, 0, 0]);
        assert!(a.0.is_empty() && b.0.is_empty());
    }

    #[test]
    fn a_source_that_runs_ahead_loses_its_oldest() {
        let lag = LAG as usize * WIDE;
        let mut q = Queue::default();
        q.push(&vec![1; lag + MOST], 0);
        assert_eq!(q.0.len(), lag + MOST);
        q.push(&[2, 2], 0);
        assert_eq!(q.0.len(), lag);
        assert_eq!(q.0.back(), Some(&2));
        // After a slow moment the mix wants a second of it: all is kept.
        let mut q = Queue::default();
        q.push(&vec![1; RATE as usize * WIDE], RATE as u64);
        assert_eq!(q.0.len(), RATE as usize * WIDE);
    }
}
