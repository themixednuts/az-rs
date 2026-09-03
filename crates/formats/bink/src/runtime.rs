//! Runtime access to RAD's native Bink DLL.
//!
//! This module keeps the FFI surface deliberately small: load `bink2w64.dll`,
//! open a video, decode the current frame, copy it into a CPU RGBA buffer, then
//! advance.

/// Audio operation applied after a video is opened with selected tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinkTrackMix {
    Volume {
        track_id: u32,
        volume: i32,
    },
    SpeakerVolumes {
        track_id: u32,
        speaker_ids: Vec<i32>,
        volumes: Vec<i32>,
    },
}

/// Project-supplied audio selection derived from a probe video's track metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinkAudioPlan {
    pub track_ids: Vec<u32>,
    pub mixes: Vec<BinkTrackMix>,
    pub sound_enabled: bool,
}

#[cfg(windows)]
mod platform {
    use std::ffi::{CStr, CString};
    use std::fmt;
    use std::os::raw::{c_char, c_void};
    use std::path::{Path, PathBuf};
    use std::ptr::NonNull;
    use std::sync::Arc;

    use thiserror::Error;
    use tracing::{debug, trace};

    use super::{BinkAudioPlan, BinkTrackMix};

    /// Bink surface type for 32-bit reversed RGBA (`RGBARGBA...`) output.
    pub const BINK_SURFACE_32RA: u32 = 6;
    const BINK_FRAME_BUFFERS_BYTES: usize = 0x218;

    type BinkOpenFn = unsafe extern "system" fn(*const c_char, u32) -> *mut BinkHandle;
    type BinkCloseFn = unsafe extern "system" fn(*mut BinkHandle);
    type BinkDoFrameFn = unsafe extern "system" fn(*mut BinkHandle) -> i32;
    type BinkNextFrameFn = unsafe extern "system" fn(*mut BinkHandle);
    type BinkWaitFn = unsafe extern "system" fn(*mut BinkHandle) -> i32;
    type BinkGetFrameBuffersInfoFn = unsafe extern "system" fn(*mut BinkHandle, *mut c_void);
    type BinkAllocateFrameBuffersFn =
        unsafe extern "system" fn(*mut BinkHandle, *mut c_void, u32) -> i32;
    type BinkRegisterFrameBuffersFn = unsafe extern "system" fn(*mut BinkHandle, *mut c_void);
    type BinkCopyToBufferFn =
        unsafe extern "system" fn(*mut BinkHandle, *mut c_void, i32, u32, u32, u32, u32);
    type BinkGetErrorFn = unsafe extern "system" fn() -> *const c_char;
    type BinkSetSoundSystemFn = unsafe extern "system" fn(*const c_void, usize) -> i32;
    type BinkSetSoundSystem2Fn = unsafe extern "system" fn(*const c_void, usize, usize) -> i32;
    type BinkGetTrackIDFn = unsafe extern "system" fn(*mut BinkHandle, u32) -> u32;
    type BinkGetTrackTypeFn = unsafe extern "system" fn(*mut BinkHandle, u32) -> u32;
    type BinkGetTrackMaxSizeFn = unsafe extern "system" fn(*mut BinkHandle, u32) -> u32;
    type BinkSetSoundTrackFn = unsafe extern "system" fn(u32, *const u32);
    type BinkSetSoundOnOffFn = unsafe extern "system" fn(*mut BinkHandle, i32);
    type BinkSetVolumeFn = unsafe extern "system" fn(*mut BinkHandle, u32, i32);
    type BinkSetSpeakerVolumesFn =
        unsafe extern "system" fn(*mut BinkHandle, u32, *const i32, *const i32, u32);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(path: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    /// Public prefix of RAD's `BINK` handle layout.
    ///
    /// The native SDK exposes width/height/frame counters as the first fields of
    /// the handle. We intentionally model only that stable prefix and use Bink
    /// API calls for all mutation.
    #[repr(C)]
    pub struct BinkHandle {
        width: u32,
        height: u32,
        frame_count: u32,
        current_frame: u32,
        last_frame: u32,
        frame_rate: u32,
        frame_rate_divisor: u32,
        read_error: u32,
        open_flags: u32,
        bink_type: u32,
        size: u32,
        frame_size: u32,
        sound_size: u32,
        frame_change_percent: u32,
        audio_track_count: u32,
    }

    #[derive(Clone, Copy)]
    struct BinkFns {
        open: BinkOpenFn,
        close: BinkCloseFn,
        do_frame: BinkDoFrameFn,
        next_frame: BinkNextFrameFn,
        wait: BinkWaitFn,
        get_frame_buffers_info: BinkGetFrameBuffersInfoFn,
        allocate_frame_buffers: BinkAllocateFrameBuffersFn,
        register_frame_buffers: BinkRegisterFrameBuffersFn,
        copy_to_buffer: BinkCopyToBufferFn,
        get_error: BinkGetErrorFn,
        set_sound_system: BinkSetSoundSystemFn,
        set_sound_system2: BinkSetSoundSystem2Fn,
        open_direct_sound: *const c_void,
        open_xaudio2: *const c_void,
        get_track_id: BinkGetTrackIDFn,
        get_track_type: BinkGetTrackTypeFn,
        get_track_max_size: BinkGetTrackMaxSizeFn,
        set_sound_track: BinkSetSoundTrackFn,
        set_sound_on_off: BinkSetSoundOnOffFn,
        set_volume: BinkSetVolumeFn,
        set_speaker_volumes: BinkSetSpeakerVolumesFn,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BinkSoundSystem {
        XAudio2,
        DirectSound,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BinkSoundSystemStatus {
        pub selected: BinkSoundSystem,
        pub xaudio2_result: i32,
        pub direct_sound_result: Option<i32>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BinkTrackInfo {
        pub index: u32,
        pub id: u32,
        pub track_type: u32,
        pub max_size: u32,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BinkAudioInfo {
        pub open_flags: u32,
        pub track_count: u32,
        pub track_ids: Vec<u32>,
        pub track_info: Vec<BinkTrackInfo>,
        pub sound_enabled: bool,
    }

    #[repr(align(8))]
    struct BinkFrameBuffers {
        bytes: [u8; BINK_FRAME_BUFFERS_BYTES],
    }

    impl BinkFrameBuffers {
        const fn new() -> Self {
            Self {
                bytes: [0; BINK_FRAME_BUFFERS_BYTES],
            }
        }

        const fn as_mut_ptr(&mut self) -> *mut c_void {
            self.bytes.as_mut_ptr().cast()
        }
    }

    /// Loaded Bink runtime library.
    pub struct BinkRuntime {
        module: NonNull<c_void>,
        fns: BinkFns,
        sound_system: BinkSoundSystemStatus,
    }

    unsafe impl Send for BinkRuntime {}
    unsafe impl Sync for BinkRuntime {}

    impl fmt::Debug for BinkRuntime {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("BinkRuntime")
                .field("module", &self.module)
                .field("sound_system", &self.sound_system.selected)
                .field("xaudio2_result", &self.sound_system.xaudio2_result)
                .field(
                    "direct_sound_result",
                    &self.sound_system.direct_sound_result,
                )
                .finish_non_exhaustive()
        }
    }

    impl Drop for BinkRuntime {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.module.as_ptr());
            }
        }
    }

    /// Open video handle owned by a loaded Bink runtime.
    pub struct BinkVideo {
        runtime: Arc<BinkRuntime>,
        handle: NonNull<BinkHandle>,
        path: PathBuf,
        audio_info: BinkAudioInfo,
        frame_buffers: Option<Box<BinkFrameBuffers>>,
    }

    impl fmt::Debug for BinkVideo {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("BinkVideo")
                .field("path", &self.path)
                .field("width", &self.width())
                .field("height", &self.height())
                .field("frame_count", &self.frame_count())
                .field("current_frame", &self.current_frame())
                .field("audio_info", &self.audio_info)
                // `runtime` and `frame_buffers` are deliberately omitted: the
                // first is a shared DLL binding table, the second a raw decode
                // scratch buffer. Neither is useful in a video's Debug output.
                .finish_non_exhaustive()
        }
    }

    impl Drop for BinkVideo {
        fn drop(&mut self) {
            unsafe {
                (self.runtime.fns.close)(self.handle.as_ptr());
            }
        }
    }

    #[derive(Debug, Error)]
    pub enum RuntimeError {
        #[error("Bink DLL path contains an interior NUL byte: {path:?}")]
        DllPathContainsNul { path: PathBuf },
        #[error("Bink video path contains an interior NUL byte: {path:?}")]
        VideoPathContainsNul { path: PathBuf },
        #[error("failed to load Bink DLL from {path}")]
        DllLoad { path: String },
        #[error("Bink DLL is missing exported symbol {name}")]
        MissingSymbol { name: &'static str },
        #[error("BinkOpen failed for {path:?}: {bink_error}")]
        OpenFailed { path: PathBuf, bink_error: String },
        #[error("invalid Bink frame dimensions {width}x{height}")]
        InvalidDimensions { width: u32, height: u32 },
        #[error("Bink frame buffer size mismatch, expected {expected} bytes, got {actual}")]
        FrameBufferSize { expected: usize, actual: usize },
        #[error("BinkAllocateFrameBuffers failed for {path:?}: {bink_error}")]
        FrameBufferAllocation { path: PathBuf, bink_error: String },
        #[error(
            "Bink speaker and volume lists differ in length: {speakers} speakers, {volumes} volumes"
        )]
        SpeakerVolumeCountMismatch { speakers: usize, volumes: usize },
    }

    impl BinkRuntime {
        /// Load `bink2w64.dll` using the normal Windows DLL search path.
        ///
        /// # Errors
        ///
        /// Returns [`RuntimeError::DllLoad`] if `bink2w64.dll` is not on the
        /// search path or `LoadLibraryA` rejects it, or
        /// [`RuntimeError::MissingSymbol`] if the DLL loads but does not
        /// export one of the Bink entry points this wrapper binds.
        pub fn load_default() -> Result<Arc<Self>, RuntimeError> {
            let name = CString::new("bink2w64.dll").map_err(|_| RuntimeError::DllLoad {
                path: "bink2w64.dll".to_string(),
            })?;
            Self::load_c_string(name.as_c_str(), "bink2w64.dll".to_string())
        }

        /// Load Bink from an explicit DLL path.
        ///
        /// # Errors
        ///
        /// Returns [`RuntimeError::DllPathContainsNul`] if `path` contains an
        /// interior NUL and so cannot become a C string,
        /// [`RuntimeError::DllLoad`] if `LoadLibraryA` cannot load it, or
        /// [`RuntimeError::MissingSymbol`] if the DLL loads but does not
        /// export one of the Bink entry points this wrapper binds.
        pub fn load_from(path: impl AsRef<Path>) -> Result<Arc<Self>, RuntimeError> {
            let path = path.as_ref();
            let path_text = path.as_os_str().to_string_lossy().into_owned();
            let path_c = CString::new(path_text.as_bytes()).map_err(|_| {
                RuntimeError::DllPathContainsNul {
                    path: path.to_path_buf(),
                }
            })?;
            Self::load_c_string(path_c.as_c_str(), path_text)
        }

        fn load_c_string(path: &CStr, display_path: String) -> Result<Arc<Self>, RuntimeError> {
            let module = unsafe { LoadLibraryA(path.as_ptr()) };
            let module =
                NonNull::new(module).ok_or(RuntimeError::DllLoad { path: display_path })?;

            let fns = unsafe {
                BinkFns {
                    open: load_symbol(module, b"BinkOpen\0", "BinkOpen")?,
                    close: load_symbol(module, b"BinkClose\0", "BinkClose")?,
                    do_frame: load_symbol(module, b"BinkDoFrame\0", "BinkDoFrame")?,
                    next_frame: load_symbol(module, b"BinkNextFrame\0", "BinkNextFrame")?,
                    wait: load_symbol(module, b"BinkWait\0", "BinkWait")?,
                    get_frame_buffers_info: load_symbol(
                        module,
                        b"BinkGetFrameBuffersInfo\0",
                        "BinkGetFrameBuffersInfo",
                    )?,
                    allocate_frame_buffers: load_symbol(
                        module,
                        b"BinkAllocateFrameBuffers\0",
                        "BinkAllocateFrameBuffers",
                    )?,
                    register_frame_buffers: load_symbol(
                        module,
                        b"BinkRegisterFrameBuffers\0",
                        "BinkRegisterFrameBuffers",
                    )?,
                    copy_to_buffer: load_symbol(module, b"BinkCopyToBuffer\0", "BinkCopyToBuffer")?,
                    get_error: load_symbol(module, b"BinkGetError\0", "BinkGetError")?,
                    set_sound_system: load_symbol(
                        module,
                        b"BinkSetSoundSystem\0",
                        "BinkSetSoundSystem",
                    )?,
                    set_sound_system2: load_symbol(
                        module,
                        b"BinkSetSoundSystem2\0",
                        "BinkSetSoundSystem2",
                    )?,
                    open_direct_sound: load_symbol(
                        module,
                        b"BinkOpenDirectSound\0",
                        "BinkOpenDirectSound",
                    )?,
                    open_xaudio2: load_symbol(module, b"BinkOpenXAudio2\0", "BinkOpenXAudio2")?,
                    get_track_id: load_symbol(module, b"BinkGetTrackID\0", "BinkGetTrackID")?,
                    get_track_type: load_symbol(module, b"BinkGetTrackType\0", "BinkGetTrackType")?,
                    get_track_max_size: load_symbol(
                        module,
                        b"BinkGetTrackMaxSize\0",
                        "BinkGetTrackMaxSize",
                    )?,
                    set_sound_track: load_symbol(
                        module,
                        b"BinkSetSoundTrack\0",
                        "BinkSetSoundTrack",
                    )?,
                    set_sound_on_off: load_symbol(
                        module,
                        b"BinkSetSoundOnOff\0",
                        "BinkSetSoundOnOff",
                    )?,
                    set_volume: load_symbol(module, b"BinkSetVolume\0", "BinkSetVolume")?,
                    set_speaker_volumes: load_symbol(
                        module,
                        b"BinkSetSpeakerVolumes\0",
                        "BinkSetSpeakerVolumes",
                    )?,
                }
            };

            let sound_system = unsafe { configure_native_sound_system(&fns) };

            Ok(Arc::new(Self {
                module,
                fns,
                sound_system,
            }))
        }

        #[must_use]
        pub const fn sound_system(&self) -> BinkSoundSystem {
            self.sound_system.selected
        }

        #[must_use]
        pub const fn sound_system_status(&self) -> BinkSoundSystemStatus {
            self.sound_system
        }

        /// Select the sound tracks used by subsequently opened videos.
        ///
        /// Bink applies this selection process-wide. Callers that choose tracks
        /// from container metadata should open a probe handle, inspect its
        /// [`BinkAudioInfo`], close it, select the tracks, and then reopen the
        /// video with the required playback flags.
        pub fn set_sound_tracks(&self, track_ids: &[u32]) {
            unsafe {
                trace!(
                    target: "bink::runtime",
                    track_ids = ?track_ids,
                    "calling BinkSetSoundTrack"
                );
                (self.fns.set_sound_track)(count_as_u32(track_ids.len()), track_ids.as_ptr());
            }
        }

        /// Open a Bink video with explicit `BinkOpen` flags.
        ///
        /// # Errors
        ///
        /// Returns [`RuntimeError::VideoPathContainsNul`] if `path` contains
        /// an interior NUL, or [`RuntimeError::OpenFailed`] carrying Bink's
        /// own error text if `BinkOpen` returns a null handle — a missing
        /// file, an unreadable one, or a payload Bink does not accept.
        pub fn open(
            self: Arc<Self>,
            path: impl AsRef<Path>,
            flags: u32,
        ) -> Result<BinkVideo, RuntimeError> {
            let path = path.as_ref();
            let handle = self.open_handle(path, flags)?;
            let track_info = unsafe { track_info_from_handle(&self.fns, handle) };
            let track_ids = track_ids_from_info(&track_info);
            let track_count = count_as_u32(track_ids.len());

            Ok(BinkVideo {
                runtime: self,
                handle,
                path: path.to_path_buf(),
                audio_info: BinkAudioInfo {
                    open_flags: flags,
                    track_count,
                    track_ids,
                    track_info,
                    sound_enabled: false,
                },
                frame_buffers: None,
            })
        }

        /// Probe a video's tracks, let project code select an audio plan, then
        /// reopen and configure the selected tracks when necessary.
        ///
        /// Bink's track selection is process-wide. Callers must serialize this
        /// operation with other video opens that change the selected tracks.
        ///
        /// # Errors
        ///
        /// Returns an error when either open fails or a speaker-volume mix has
        /// different speaker and volume counts.
        pub fn open_with_audio_plan(
            self: Arc<Self>,
            path: impl AsRef<Path>,
            probe_flags: u32,
            playback_flags: u32,
            planner: impl FnOnce(&BinkAudioInfo) -> BinkAudioPlan,
        ) -> Result<BinkVideo, RuntimeError> {
            let path = path.as_ref();
            let probe = self.clone().open(path, probe_flags)?;
            let plan = planner(probe.audio_info());
            let mut video = if plan.track_ids.is_empty() {
                probe
            } else {
                drop(probe);
                self.set_sound_tracks(&plan.track_ids);
                self.open(path, playback_flags)?
            };

            for mix in plan.mixes {
                match mix {
                    BinkTrackMix::Volume { track_id, volume } => {
                        video.set_track_volume(track_id, volume);
                    }
                    BinkTrackMix::SpeakerVolumes {
                        track_id,
                        speaker_ids,
                        volumes,
                    } => video.set_speaker_volumes(track_id, &speaker_ids, &volumes)?,
                }
            }
            video.set_sound_enabled(plan.sound_enabled);
            Ok(video)
        }

        fn open_handle(
            &self,
            path: &Path,
            flags: u32,
        ) -> Result<NonNull<BinkHandle>, RuntimeError> {
            let path_c = path_to_cstring(path)?;
            let handle = unsafe { (self.fns.open)(path_c.as_ptr(), flags) };
            NonNull::new(handle).ok_or_else(|| RuntimeError::OpenFailed {
                path: path.to_path_buf(),
                bink_error: self
                    .last_error()
                    .unwrap_or_else(|| "Bink did not report an error string".to_string()),
            })
        }

        fn last_error(&self) -> Option<String> {
            let ptr = unsafe { (self.fns.get_error)() };
            if ptr.is_null() {
                return None;
            }

            let text = unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .trim()
                .to_string();
            (!text.is_empty()).then_some(text)
        }
    }

    impl BinkVideo {
        #[must_use]
        pub fn path(&self) -> &Path {
            &self.path
        }

        #[must_use]
        pub const fn width(&self) -> u32 {
            unsafe { self.handle.as_ref().width }
        }

        #[must_use]
        pub const fn height(&self) -> u32 {
            unsafe { self.handle.as_ref().height }
        }

        #[must_use]
        pub const fn frame_count(&self) -> u32 {
            unsafe { self.handle.as_ref().frame_count }
        }

        #[must_use]
        pub const fn current_frame(&self) -> u32 {
            unsafe { self.handle.as_ref().current_frame }
        }

        #[must_use]
        pub const fn sound_size(&self) -> u32 {
            unsafe { self.handle.as_ref().sound_size }
        }

        #[must_use]
        pub const fn audio_info(&self) -> &BinkAudioInfo {
            &self.audio_info
        }

        pub fn set_sound_enabled(&mut self, enabled: bool) {
            unsafe {
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?self.handle.as_ptr(),
                    enabled,
                    sound_size = self.sound_size(),
                    "calling BinkSetSoundOnOff"
                );
                (self.runtime.fns.set_sound_on_off)(self.handle.as_ptr(), i32::from(enabled));
            }
            self.audio_info.sound_enabled = enabled;
        }

        /// Set the native volume for one selected audio track.
        pub fn set_track_volume(&mut self, track_id: u32, volume: i32) {
            unsafe {
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?self.handle.as_ptr(),
                    track_id,
                    volume,
                    "calling BinkSetVolume"
                );
                (self.runtime.fns.set_volume)(self.handle.as_ptr(), track_id, volume);
            }
        }

        /// Route one audio track to explicit speakers with per-speaker volumes.
        ///
        /// # Errors
        ///
        /// Returns [`RuntimeError::SpeakerVolumeCountMismatch`] when the two
        /// slices do not describe the same number of speaker assignments.
        pub fn set_speaker_volumes(
            &mut self,
            track_id: u32,
            speaker_ids: &[i32],
            volumes: &[i32],
        ) -> Result<(), RuntimeError> {
            if speaker_ids.len() != volumes.len() {
                return Err(RuntimeError::SpeakerVolumeCountMismatch {
                    speakers: speaker_ids.len(),
                    volumes: volumes.len(),
                });
            }
            unsafe {
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?self.handle.as_ptr(),
                    track_id,
                    speaker_ids = ?speaker_ids,
                    volumes = ?volumes,
                    "calling BinkSetSpeakerVolumes"
                );
                (self.runtime.fns.set_speaker_volumes)(
                    self.handle.as_ptr(),
                    track_id,
                    speaker_ids.as_ptr(),
                    volumes.as_ptr(),
                    count_as_u32(speaker_ids.len()),
                );
            }
            Ok(())
        }

        #[must_use]
        pub fn should_wait(&self) -> bool {
            unsafe { (self.runtime.fns.wait)(self.handle.as_ptr()) != 0 }
        }

        /// Decode the next frame into `dest` as tightly packed RGBA.
        ///
        /// # Errors
        ///
        /// Returns [`RuntimeError::InvalidDimensions`] if the video reports a
        /// width or height whose RGBA byte count does not fit in `usize`, or a
        /// row pitch that does not fit in `i32`;
        /// [`RuntimeError::FrameBufferSize`] if `dest` is not exactly
        /// `width * height * 4` bytes; and
        /// [`RuntimeError::FrameBufferAllocation`] if Bink cannot allocate its
        /// own frame buffers on the first call.
        pub fn decode_next_frame_rgba(&mut self, dest: &mut [u8]) -> Result<(), RuntimeError> {
            let width = self.width();
            let height = self.height();
            let expected = frame_len_rgba(width, height)?;
            if dest.len() != expected {
                return Err(RuntimeError::FrameBufferSize {
                    expected,
                    actual: dest.len(),
                });
            }

            let pitch = i32::try_from(width.saturating_mul(4))
                .map_err(|_| RuntimeError::InvalidDimensions { width, height })?;

            self.ensure_frame_buffers_registered()?;

            unsafe {
                let handle = self.handle.as_ptr();
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    width,
                    height,
                    pitch,
                    dest_len = dest.len(),
                    current_frame = self.current_frame(),
                    "calling BinkDoFrame"
                );
                (self.runtime.fns.do_frame)(handle);
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    current_frame = self.current_frame(),
                    "returned from BinkDoFrame"
                );
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    dest = ?dest.as_mut_ptr(),
                    pitch,
                    height,
                    surface = BINK_SURFACE_32RA,
                    "calling BinkCopyToBuffer"
                );
                (self.runtime.fns.copy_to_buffer)(
                    handle,
                    dest.as_mut_ptr().cast(),
                    pitch,
                    height,
                    0,
                    0,
                    BINK_SURFACE_32RA,
                );
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    current_frame = self.current_frame(),
                    "returned from BinkCopyToBuffer"
                );
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    current_frame = self.current_frame(),
                    "calling BinkNextFrame"
                );
                (self.runtime.fns.next_frame)(handle);
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    current_frame = self.current_frame(),
                    "returned from BinkNextFrame"
                );
            }

            Ok(())
        }

        fn ensure_frame_buffers_registered(&mut self) -> Result<(), RuntimeError> {
            if self.frame_buffers.is_some() {
                return Ok(());
            }

            let mut frame_buffers = Box::new(BinkFrameBuffers::new());
            let handle = self.handle.as_ptr();
            let frame_buffers_ptr = frame_buffers.as_mut_ptr();
            unsafe {
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    frame_buffers = ?frame_buffers_ptr,
                    frame_buffers_bytes = BINK_FRAME_BUFFERS_BYTES,
                    "calling BinkGetFrameBuffersInfo"
                );
                (self.runtime.fns.get_frame_buffers_info)(handle, frame_buffers_ptr);
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    frame_buffers = ?frame_buffers_ptr,
                    "calling BinkAllocateFrameBuffers"
                );
                let allocated =
                    (self.runtime.fns.allocate_frame_buffers)(handle, frame_buffers_ptr, 0);
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    frame_buffers = ?frame_buffers_ptr,
                    allocated,
                    "returned from BinkAllocateFrameBuffers"
                );
                if allocated == 0 {
                    return Err(RuntimeError::FrameBufferAllocation {
                        path: self.path.clone(),
                        bink_error: self
                            .runtime
                            .last_error()
                            .unwrap_or_else(|| "Bink did not report an error string".to_string()),
                    });
                }
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    frame_buffers = ?frame_buffers_ptr,
                    "calling BinkRegisterFrameBuffers"
                );
                (self.runtime.fns.register_frame_buffers)(handle, frame_buffers_ptr);
                trace!(
                    target: "bink::runtime",
                    path = %self.path.display(),
                    handle = ?handle,
                    frame_buffers = ?frame_buffers_ptr,
                    "returned from BinkRegisterFrameBuffers"
                );
            }

            self.frame_buffers = Some(frame_buffers);
            Ok(())
        }
    }

    fn path_to_cstring(path: &Path) -> Result<CString, RuntimeError> {
        let text = path.as_os_str().to_string_lossy();
        CString::new(text.as_bytes()).map_err(|_| RuntimeError::VideoPathContainsNul {
            path: path.to_path_buf(),
        })
    }

    fn frame_len_rgba(width: u32, height: u32) -> Result<usize, RuntimeError> {
        if width == 0 || height == 0 {
            return Err(RuntimeError::InvalidDimensions { width, height });
        }

        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(RuntimeError::InvalidDimensions { width, height })
    }

    unsafe fn load_symbol<T: Copy>(
        module: NonNull<c_void>,
        name: &'static [u8],
        display_name: &'static str,
    ) -> Result<T, RuntimeError> {
        let ptr = unsafe { GetProcAddress(module.as_ptr(), name.as_ptr().cast()) };
        if ptr.is_null() {
            return Err(RuntimeError::MissingSymbol { name: display_name });
        }
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    unsafe fn track_info_from_handle(
        fns: &BinkFns,
        handle: NonNull<BinkHandle>,
    ) -> Vec<BinkTrackInfo> {
        let track_count = unsafe { handle.as_ref().audio_track_count };
        let mut track_info = Vec::with_capacity(track_count as usize);
        for index in 0..track_count {
            track_info.push(BinkTrackInfo {
                index,
                id: unsafe { (fns.get_track_id)(handle.as_ptr(), index) },
                track_type: unsafe { (fns.get_track_type)(handle.as_ptr(), index) },
                max_size: unsafe { (fns.get_track_max_size)(handle.as_ptr(), index) },
            });
        }
        track_info
    }

    fn track_ids_from_info(track_info: &[BinkTrackInfo]) -> Vec<u32> {
        track_info.iter().map(|track| track.id).collect()
    }

    /// Narrow a track or speaker count to the `u32` the Bink C API takes.
    ///
    /// Bink videos carry bounded track and speaker lists, so the count always
    /// fits. The `debug_assert!` catches violations in tests; release builds
    /// saturate rather than wrap.
    fn count_as_u32(count: usize) -> u32 {
        debug_assert!(
            u32::try_from(count).is_ok(),
            "Bink track or speaker count {count} does not fit in u32"
        );
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    unsafe fn configure_native_sound_system(fns: &BinkFns) -> BinkSoundSystemStatus {
        let xaudio2_result = unsafe { (fns.set_sound_system2)(fns.open_xaudio2, 0, 0) };
        debug!(
            target: "bink::runtime",
            xaudio2_result,
            "BinkSetSoundSystem2(BinkOpenXAudio2, 0, 0)"
        );
        if xaudio2_result != 0 {
            BinkSoundSystemStatus {
                selected: BinkSoundSystem::XAudio2,
                xaudio2_result,
                direct_sound_result: None,
            }
        } else {
            let direct_sound_result = unsafe { (fns.set_sound_system)(fns.open_direct_sound, 0) };
            debug!(
                target: "bink::runtime",
                xaudio2_result,
                direct_sound_result,
                "BinkSetSoundSystem(BinkOpenDirectSound, 0) fallback"
            );
            BinkSoundSystemStatus {
                selected: BinkSoundSystem::DirectSound,
                xaudio2_result,
                direct_sound_result: Some(direct_sound_result),
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::fmt;
    use std::path::Path;
    use std::sync::Arc;

    use thiserror::Error;

    use super::BinkAudioPlan;

    #[derive(Debug, Error)]
    pub enum RuntimeError {
        #[error("native Bink runtime is only available on Windows")]
        UnsupportedPlatform,
        #[error(
            "Bink speaker and volume lists differ in length: {speakers} speakers, {volumes} volumes"
        )]
        SpeakerVolumeCountMismatch { speakers: usize, volumes: usize },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BinkSoundSystem {
        Unsupported,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BinkSoundSystemStatus {
        pub selected: BinkSoundSystem,
        pub xaudio2_result: i32,
        pub direct_sound_result: Option<i32>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BinkTrackInfo {
        pub index: u32,
        pub id: u32,
        pub track_type: u32,
        pub max_size: u32,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BinkAudioInfo {
        pub open_flags: u32,
        pub track_count: u32,
        pub track_ids: Vec<u32>,
        pub track_info: Vec<BinkTrackInfo>,
        pub sound_enabled: bool,
    }

    pub struct BinkRuntime;

    impl fmt::Debug for BinkRuntime {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("BinkRuntime").finish_non_exhaustive()
        }
    }

    pub struct BinkVideo;

    impl fmt::Debug for BinkVideo {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("BinkVideo").finish_non_exhaustive()
        }
    }

    impl BinkRuntime {
        pub fn load_default() -> Result<Arc<Self>, RuntimeError> {
            Err(RuntimeError::UnsupportedPlatform)
        }

        pub fn load_from(_path: impl AsRef<Path>) -> Result<Arc<Self>, RuntimeError> {
            Err(RuntimeError::UnsupportedPlatform)
        }

        pub fn open(
            self: Arc<Self>,
            _path: impl AsRef<Path>,
            _flags: u32,
        ) -> Result<BinkVideo, RuntimeError> {
            Err(RuntimeError::UnsupportedPlatform)
        }

        pub fn open_with_audio_plan(
            self: Arc<Self>,
            _path: impl AsRef<Path>,
            _probe_flags: u32,
            _playback_flags: u32,
            _planner: impl FnOnce(&BinkAudioInfo) -> BinkAudioPlan,
        ) -> Result<BinkVideo, RuntimeError> {
            Err(RuntimeError::UnsupportedPlatform)
        }

        pub fn sound_system(&self) -> BinkSoundSystem {
            BinkSoundSystem::Unsupported
        }

        pub fn sound_system_status(&self) -> BinkSoundSystemStatus {
            BinkSoundSystemStatus {
                selected: BinkSoundSystem::Unsupported,
                xaudio2_result: 0,
                direct_sound_result: None,
            }
        }

        pub fn set_sound_tracks(&self, _track_ids: &[u32]) {}
    }

    impl BinkVideo {
        pub fn path(&self) -> &Path {
            Path::new("")
        }

        pub fn width(&self) -> u32 {
            0
        }

        pub fn height(&self) -> u32 {
            0
        }

        pub fn frame_count(&self) -> u32 {
            0
        }

        pub fn current_frame(&self) -> u32 {
            0
        }

        pub fn sound_size(&self) -> u32 {
            0
        }

        pub fn audio_info(&self) -> &BinkAudioInfo {
            static INFO: BinkAudioInfo = BinkAudioInfo {
                open_flags: 0,
                track_count: 0,
                track_ids: Vec::new(),
                track_info: Vec::new(),
                sound_enabled: false,
            };
            &INFO
        }

        pub fn set_sound_enabled(&mut self, _enabled: bool) {}

        pub fn set_track_volume(&mut self, _track_id: u32, _volume: i32) {}

        pub fn set_speaker_volumes(
            &mut self,
            _track_id: u32,
            speaker_ids: &[i32],
            volumes: &[i32],
        ) -> Result<(), RuntimeError> {
            if speaker_ids.len() != volumes.len() {
                return Err(RuntimeError::SpeakerVolumeCountMismatch {
                    speakers: speaker_ids.len(),
                    volumes: volumes.len(),
                });
            }
            Err(RuntimeError::UnsupportedPlatform)
        }

        pub fn should_wait(&self) -> bool {
            true
        }

        pub fn decode_next_frame_rgba(&mut self, _dest: &mut [u8]) -> Result<(), RuntimeError> {
            Err(RuntimeError::UnsupportedPlatform)
        }
    }
}

pub use platform::*;
