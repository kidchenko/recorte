//! GDI and Media Foundation: a `BitBlt` of the area thirty times a second,
//! each one handed to a sink writer, which encodes H.264 into an MP4. The
//! sound goes to the same writer as a second track, in AAC; see `audio`.
//!
//! GDI again, as for a snip (see `capture::win`): one call per frame, no D3D
//! device, and it leaves out the windows marked as not to be captured, which
//! is how the frame and the bar stay out of the film. It also leaves out the
//! pointer, so that is drawn on each frame by hand.
//!
//! Everything here lives on the recorder's own thread: Media Foundation is
//! started on it, the frames are taken and written on it, and the file is
//! closed on it when the stop flag is seen. The bar's buttons reach it as
//! flags it reads once a frame.

mod audio;

use super::Area;
use audio::Sound;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CombineRgn, CreateCompatibleDC, CreateDIBSection, CreateRectRgn,
    DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, SetWindowRgn, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, RGN_OR, SRCCOPY,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaType, IMFSample, IMFSinkWriter, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFMediaType_Audio, MFMediaType_Video, MFShutdown, MFStartup,
    MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MFSTARTUP_LITE,
    MF_API_VERSION, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, GetWindowRect, SetWindowPos, CURSORINFO,
    CURSOR_SHOWING, DI_NORMAL, HICON, ICONINFO, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

/// Put the corner of `window`'s pixels at `at` on the desktop, and show only
/// `shape` of it: rectangles from that corner. The rest is not there, to the
/// eye or to the mouse.
///
/// Both go by where the pixels really are, which need not be the window's
/// own corner: a window without a title bar can still have an unseen border.
pub fn clip(window: &tao::window::Window, at: (i32, i32), shape: &[(i32, i32, i32, i32)]) {
    use tao::platform::windows::WindowExtWindows;
    let hwnd = HWND(window.hwnd() as _);
    unsafe {
        let (mut outer, mut inner) = (RECT::default(), POINT::default());
        if GetWindowRect(hwnd, &mut outer).is_err() || !ClientToScreen(hwnd, &mut inner).as_bool() {
            return;
        }
        let (dx, dy) = (inner.x - outer.left, inner.y - outer.top);
        let region = CreateRectRgn(0, 0, 0, 0);
        for &(x, y, w, h) in shape {
            let part = CreateRectRgn(dx + x, dy + y, dx + x + w, dy + y + h);
            CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        // The window owns the region from here, unless it would not take it.
        if SetWindowRgn(hwnd, Some(region), false) == 0 {
            let _ = DeleteObject(region.into());
        }
        let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(hwnd, None, at.0 - dx, at.1 - dy, 0, 0, flags);
    }
}

const FPS: u32 = 30;
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);
/// How often a paused recorder looks whether to carry on.
const PAUSED: Duration = Duration::from_millis(20);
/// The sound's bytes a second once it is AAC: 160 kbit/s.
const AAC_BYTES: u32 = 20_000;
/// `MF_VERSION` in the headers: the SDK version over the API version.
const MF_VERSION: u32 = 2 << 16 | MF_API_VERSION;

pub struct Recorder {
    switches: Arc<Switches>,
    /// There until it is stopped.
    thread: Option<JoinHandle<Result<(), String>>>,
}

/// What the bar tells the recorder's thread.
#[derive(Default)]
struct Switches {
    stop: AtomicBool,
    paused: AtomicBool,
    mic: AtomicBool,
    system: AtomicBool,
}

impl Recorder {
    /// Start filming `area` into `path`, with the microphone and what the
    /// computer plays in it or not. Returns once the encoder has accepted
    /// the size and the file is open, so what can fail has.
    pub fn start(area: Area, path: &Path, mic: bool, system: bool) -> Result<Self, String> {
        let switches = Arc::new(Switches::default());
        switches.mic.store(mic, Ordering::Relaxed);
        switches.system.store(system, Ordering::Relaxed);
        let (ready, is_ready) = mpsc::channel();
        let (flags, file) = (switches.clone(), path.to_path_buf());
        let thread = std::thread::Builder::new()
            .name("recorte-record".into())
            .spawn(move || run(area, &file, &flags, ready))
            .map_err(|e| format!("could not start the recorder: {e}"))?;
        match is_ready.recv() {
            Ok(Ok(())) => Ok(Self {
                switches,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the recorder stopped before it started".into()),
        }
    }

    /// Hold the film, or carry on. What happens meanwhile is not in it.
    pub fn set_paused(&self, paused: bool) {
        self.switches.paused.store(paused, Ordering::Relaxed);
    }

    /// The microphone in the film from here on, or not.
    pub fn set_mic(&self, on: bool) {
        self.switches.mic.store(on, Ordering::Relaxed);
    }

    /// The same for what the computer plays.
    pub fn set_system(&self, on: bool) {
        self.switches.system.store(on, Ordering::Relaxed);
    }

    /// Has it stopped by itself? Only an error does that.
    pub fn is_over(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Stop filming and close the file.
    pub fn stop(mut self) -> Result<(), String> {
        self.switches.stop.store(true, Ordering::Relaxed);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|_| Err("the recorder crashed".into())),
            None => Ok(()),
        }
    }
}

impl Drop for Recorder {
    /// Let go without being stopped, it still stops, and closes the file
    /// by itself: a thread nobody can reach would film for ever.
    fn drop(&mut self) {
        self.switches.stop.store(true, Ordering::Relaxed);
    }
}

fn run(
    area: Area,
    path: &Path,
    switches: &Switches,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let mut film = match Film::open(area, path) {
        Ok(film) => film,
        Err(e) => {
            // Whatever the failed attempt left of the file.
            let _ = std::fs::remove_file(path);
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    // Opening the sound devices takes a moment too. Without them it films
    // all the same, in silence.
    let on = |flag: &AtomicBool| flag.load(Ordering::Relaxed);
    film.sound.skip(on(&switches.mic), on(&switches.system));
    let _ = ready.send(Ok(()));
    let filmed = film.roll(switches);
    // Close the file even after an error: what was filmed until then plays.
    let closed = unsafe { film.writer.Finalize() }.map_err(|e| format!("closing the video: {e}"));
    filmed.and(closed)
}

/// Fields are dropped in this order: Media Foundation and COM are shut down
/// last, after the writer and the sound devices that came from them are
/// released.
struct Film {
    writer: IMFSinkWriter,
    /// The writer's two tracks.
    picture: u32,
    track: u32,
    sound: Sound,
    screen: Screen,
    _mf: MediaFoundation,
}

impl Film {
    fn open(area: Area, path: &Path) -> Result<Self, String> {
        let mf = MediaFoundation::start()?;
        let screen = Screen::new(area)?;
        // A graphics card's encoder if there is one, for the CPU it saves.
        // Some refuse sizes the software encoder takes, so try that next.
        let (writer, picture, track) = unsafe {
            writer(path, area, true)
                .or_else(|e| {
                    traykit::linfo!("no hardware encoder for this ({e}); using software");
                    writer(path, area, false)
                })
                .map_err(|e| {
                    format!(
                        "could not set up a {}x{} video: {e}",
                        area.width, area.height
                    )
                })?
        };
        Ok(Self {
            writer,
            picture,
            track,
            sound: Sound::new(),
            screen,
            _mf: mf,
        })
    }

    /// Film until told to stop.
    fn roll(&mut self, switches: &Switches) -> Result<(), String> {
        let on = |flag: &AtomicBool| flag.load(Ordering::Relaxed);
        // The film's clock: now less this is how long the film is.
        let mut started = Instant::now();
        let mut next = started;
        while !on(&switches.stop) {
            if on(&switches.paused) {
                // A pause is cut out of the film: its clock stands still, so
                // what comes after follows straight on what came before.
                // The sound runs a little behind, so first bring it level,
                // or the last of it before the pause would be lost.
                let since = Instant::now();
                self.hear(started.elapsed(), switches, true)?;
                while on(&switches.paused) && !on(&switches.stop) {
                    self.sound.skip(on(&switches.mic), on(&switches.system));
                    std::thread::sleep(PAUSED);
                }
                started += since.elapsed();
                next = Instant::now();
                continue;
            }
            // Each frame is stamped with when it was really taken, so a slow
            // moment shows as a held picture and not as the film running
            // fast.
            let at = started.elapsed();
            // Writing blocks while the encoder is behind, which is what
            // paces the loop on a machine too slow for thirty frames a
            // second.
            unsafe {
                sample(self.screen.frame(), ticks(at), ticks(FRAME))
                    .and_then(|frame| self.writer.WriteSample(self.picture, &frame))
            }
            .map_err(|e| format!("writing the video: {e}"))?;
            self.hear(at, switches, false)?;
            next += FRAME;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                // Behind: carry on from here rather than race to catch up.
                next = now;
            }
        }
        // The sound runs a little behind the picture; bring it level.
        self.hear(started.elapsed(), switches, true)
    }

    /// Write the sound of the film up to `at`.
    fn hear(&mut self, at: Duration, switches: &Switches, last: bool) -> Result<(), String> {
        let mic = switches.mic.load(Ordering::Relaxed);
        let system = switches.system.load(Ordering::Relaxed);
        let (first, pcm) = self.sound.mix(at, mic, system, last);
        if pcm.is_empty() {
            return Ok(());
        }
        // Stamped by how many frames of sound went before, so the track has
        // no gaps and no overlaps whatever the clock did between two calls.
        let frames = (pcm.len() / audio::CHANNELS as usize) as u64;
        let time = |frames: u64| (frames * 10_000_000 / audio::RATE as u64) as i64;
        unsafe {
            let bytes = std::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 2);
            sample(bytes, time(first), time(frames))
                .and_then(|sample| self.writer.WriteSample(self.track, &sample))
        }
        .map_err(|e| format!("writing the sound: {e}"))
    }
}

/// A duration in Media Foundation's unit, 100 ns.
fn ticks(time: Duration) -> i64 {
    (time.as_nanos() / 100) as i64
}

/// `data` as a sample for the writer, at `time` for `lasting`.
unsafe fn sample(data: &[u8], time: i64, lasting: i64) -> windows::core::Result<IMFSample> {
    let buffer = MFCreateMemoryBuffer(data.len() as u32)?;
    let mut dst = std::ptr::null_mut();
    buffer.Lock(&mut dst, None, None)?;
    std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
    buffer.Unlock()?;
    buffer.SetCurrentLength(data.len() as u32)?;
    let sample = MFCreateSample()?;
    sample.AddBuffer(&buffer)?;
    sample.SetSampleTime(time)?;
    sample.SetSampleDuration(lasting)?;
    Ok(sample)
}

/// A sink writer for `path`, ready for frames and for sound, and its two
/// tracks. The picture: H.264 out, and in the 32-bit rows `Screen` produces;
/// the writer puts the colour conversion between. The sound: AAC out, and in
/// the samples `Sound` mixes.
unsafe fn writer(
    path: &Path,
    area: Area,
    hardware: bool,
) -> windows::core::Result<(IMFSinkWriter, u32, u32)> {
    let mut attributes: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attributes, 1)?;
    if let Some(a) = &attributes {
        a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32)?;
    }
    let url = HSTRING::from(path.as_os_str());
    let writer = MFCreateSinkWriterFromURL(&url, None, attributes.as_ref())?;

    let out = video_type(&MFVideoFormat_H264, area)?;
    out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate(area))?;
    let picture = writer.AddStream(&out)?;
    let input = video_type(&MFVideoFormat_RGB32, area)?;
    writer.SetInputMediaType(picture, &input, None)?;

    let out = audio_type(&MFAudioFormat_AAC)?;
    out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_BYTES)?;
    let track = writer.AddStream(&out)?;
    let input = audio_type(&MFAudioFormat_PCM)?;
    let align = audio::CHANNELS * 2;
    input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, align)?;
    input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, audio::RATE * align)?;
    input.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
    writer.SetInputMediaType(track, &input, None)?;

    writer.BeginWriting()?;
    Ok((writer, picture, track))
}

unsafe fn audio_type(format: &windows::core::GUID) -> windows::core::Result<IMFMediaType> {
    let t = MFCreateMediaType()?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    t.SetGUID(&MF_MT_SUBTYPE, format)?;
    t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio::RATE)?;
    t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio::CHANNELS)?;
    Ok(t)
}

unsafe fn video_type(
    format: &windows::core::GUID,
    area: Area,
) -> windows::core::Result<IMFMediaType> {
    let pair = |a: u32, b: u32| (a as u64) << 32 | b as u64;
    let t = MFCreateMediaType()?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    t.SetGUID(&MF_MT_SUBTYPE, format)?;
    t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    t.SetUINT64(&MF_MT_FRAME_SIZE, pair(area.width, area.height))?;
    t.SetUINT64(&MF_MT_FRAME_RATE, pair(FPS, 1))?;
    t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
    Ok(t)
}

/// Bits per second: an eighth of a bit per pixel per frame, which keeps
/// text sharp (8 Mbit/s at 1080p), within what any player takes.
fn bitrate(area: Area) -> u32 {
    let bits = area.width as u64 * area.height as u64 * FPS as u64 / 8;
    bits.clamp(1_000_000, 24_000_000) as u32
}

/// COM and Media Foundation, for as long as this lives.
struct MediaFoundation;

impl MediaFoundation {
    fn start() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| format!("could not start COM: {e}"))?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_LITE) {
                CoUninitialize();
                return Err(format!("could not start Media Foundation: {e}"));
            }
        }
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// The area of the screen, copied into the same bitmap for every frame.
struct Screen {
    area: Area,
    screen: HDC,
    mem: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    bits: *const u8,
}

impl Screen {
    fn new(area: Area) -> Result<Self, String> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.width as i32,
                // Positive: bottom-up rows, which is how Media Foundation
                // reads RGB unless told otherwise.
                biHeight: area.height as i32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            if screen.is_invalid() || mem.is_invalid() {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return Err("could not reach the screen".into());
            }
            let mut bits = std::ptr::null_mut();
            match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(dib) if !bits.is_null() => Ok(Self {
                    area,
                    screen,
                    mem,
                    dib,
                    old: SelectObject(mem, dib.into()),
                    bits: bits as *const u8,
                }),
                other => {
                    if let Ok(dib) = other {
                        let _ = DeleteObject(dib.into());
                    }
                    let _ = DeleteDC(mem);
                    ReleaseDC(None, screen);
                    Err("could not allocate the frame".into())
                }
            }
        }
    }

    /// The area as it is now, pointer included. The slice is the bitmap
    /// itself: the next call overwrites it.
    fn frame(&mut self) -> &[u8] {
        let Area {
            x,
            y,
            width,
            height,
        } = self.area;
        unsafe {
            // While the secure desktop is up (a UAC prompt, the lock screen)
            // this fails; the bitmap keeps the last frame, and the film holds
            // on it until the screen is back.
            let blit = BitBlt(
                self.mem,
                0,
                0,
                width as i32,
                height as i32,
                Some(self.screen),
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            );
            if blit.is_ok() {
                self.pointer();
            }
            std::slice::from_raw_parts(self.bits, width as usize * height as usize * 4)
        }
    }

    /// Draw the mouse pointer where it is, as it looks now.
    unsafe fn pointer(&self) {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        if GetCursorInfo(&mut info).is_err() || info.flags.0 & CURSOR_SHOWING.0 == 0 {
            return;
        }
        let cursor = HICON(info.hCursor.0);
        // For the hot spot: the point of the arrow, not its corner, is where
        // the pointer is.
        let mut icon = ICONINFO::default();
        if GetIconInfo(cursor, &mut icon).is_err() {
            return;
        }
        // `GetIconInfo` makes copies of the bitmaps, which are ours to free.
        let _ = DeleteObject(icon.hbmMask.into());
        let _ = DeleteObject(icon.hbmColor.into());
        let _ = DrawIconEx(
            self.mem,
            info.ptScreenPos.x - self.area.x - icon.xHotspot as i32,
            info.ptScreenPos.y - self.area.y - icon.yHotspot as i32,
            cursor,
            0,
            0,
            0,
            None,
            DI_NORMAL,
        );
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bitrate_follows_the_size_within_limits() {
        let area = |width, height| Area {
            x: 0,
            y: 0,
            width,
            height,
        };
        assert_eq!(bitrate(area(1920, 1080)), 7_776_000);
        assert_eq!(bitrate(area(64, 64)), 1_000_000);
        assert_eq!(bitrate(area(7680, 4320)), 24_000_000);
    }

    /// Films the top-left corner of the real screen for a second, with what
    /// the computer plays, pauses for a second and films one more with the
    /// microphone too. So it needs a desktop and the system's encoders: run
    /// it by hand, with `cargo test -- --ignored`. `RECORTE_KEEP_FILM=1`
    /// leaves the file, which should be two seconds long.
    #[test]
    #[ignore]
    fn two_seconds_of_screen_become_an_mp4() {
        let path = std::env::temp_dir().join("recorte-test.mp4");
        let area = Area {
            x: 0,
            y: 0,
            width: 640,
            height: 360,
        };
        let recorder = Recorder::start(area, &path, false, true).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        recorder.set_paused(true);
        std::thread::sleep(Duration::from_secs(1));
        recorder.set_mic(true);
        recorder.set_paused(false);
        std::thread::sleep(Duration::from_secs(1));
        assert!(!recorder.is_over());
        recorder.stop().unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 1_000, "{size} bytes is not a video");
        if std::env::var_os("RECORTE_KEEP_FILM").is_none() {
            let _ = std::fs::remove_file(&path);
        }
    }
}
