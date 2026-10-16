//! A short tone played when a highlight word arrives.
//!
//! Linux playback opens the default ALSA device through `libasound.so.2`.
//! `open_playback` is the init call (`snd_pcm_open`). macOS uses
//! `AudioServicesPlayAlertSound`. Windows uses `Beep`.

use std::sync::atomic::{AtomicBool, Ordering};

const RATE: u32 = 22_050;
const HZ: u32 = 880;
const MILLIS: u32 = 140;

static BUSY: AtomicBool = AtomicBool::new(false);

pub fn play() {
    let samples = tone(RATE, HZ, MILLIS);
    spawn_samples(samples, RATE);
}

pub fn play_file(path: &str) {
    let path = path.to_string();
    if path.is_empty()
        || BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        play_path(&path);
        BUSY.store(false, Ordering::Release);
    });
}

fn spawn_samples(samples: Vec<i16>, rate: u32) {
    if BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        play_samples(&samples, rate);
        BUSY.store(false, Ordering::Release);
    });
}

fn play_path(path: &str) {
    #[cfg(target_os = "linux")]
    if let Ok(bytes) = std::fs::read(path)
        && let Some((rate, samples)) = crate::wav::decode(&bytes)
    {
        play_samples(&samples, rate);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("afplay").arg(path).status();
    }
    #[cfg(windows)]
    play_windows_file(path);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = path;
}

pub fn tone(rate: u32, hz: u32, millis: u32) -> Vec<i16> {
    let count = (u64::from(rate) * u64::from(millis) / 1000) as usize;
    (0..count)
        .map(|index| {
            let theta = index as f32 * 2.0 * std::f32::consts::PI * hz as f32 / rate as f32;
            (theta.sin() * 8000.0) as i16
        })
        .collect()
}

fn play_samples(samples: &[i16], rate: u32) {
    #[cfg(target_os = "linux")]
    play_alsa(samples, rate);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (samples, rate);
    }
    #[cfg(target_os = "macos")]
    play_macos();
    #[cfg(windows)]
    play_windows();
}

#[cfg(target_os = "linux")]
fn play_alsa(samples: &[i16], rate: u32) {
    let Some(device) = open_playback(rate) else {
        return;
    };
    write_playback(&device, samples);
    close_playback(device);
}

#[cfg(target_os = "linux")]
struct Playback {
    pcm: *mut libc::c_void,
    writei: WriteiFn,
    prepare: PrepareFn,
    drain: DrainFn,
    close: CloseFn,
    lib: *mut libc::c_void,
}

#[cfg(target_os = "linux")]
type OpenFn = unsafe extern "C" fn(*mut *mut libc::c_void, *const libc::c_char, i32, i32) -> i32;
#[cfg(target_os = "linux")]
type SetParamsFn = unsafe extern "C" fn(*mut libc::c_void, i32, i32, u32, u32, i32, u32) -> i32;
#[cfg(target_os = "linux")]
type WriteiFn = unsafe extern "C" fn(*mut libc::c_void, *const libc::c_void, libc::c_ulong) -> i64;
#[cfg(target_os = "linux")]
type PrepareFn = unsafe extern "C" fn(*mut libc::c_void) -> i32;
#[cfg(target_os = "linux")]
type DrainFn = unsafe extern "C" fn(*mut libc::c_void) -> i32;
#[cfg(target_os = "linux")]
type CloseFn = unsafe extern "C" fn(*mut libc::c_void) -> i32;

#[cfg(target_os = "linux")]
fn open_playback(rate: u32) -> Option<Playback> {
    let lib = unsafe { libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_LAZY) };
    if lib.is_null() {
        return None;
    }
    let open = load_fn::<OpenFn>(lib, c"snd_pcm_open")?;
    let set_params = load_fn::<SetParamsFn>(lib, c"snd_pcm_set_params")?;
    let writei = load_fn::<WriteiFn>(lib, c"snd_pcm_writei")?;
    let prepare = load_fn::<PrepareFn>(lib, c"snd_pcm_prepare")?;
    let drain = load_fn::<DrainFn>(lib, c"snd_pcm_drain")?;
    let close = load_fn::<CloseFn>(lib, c"snd_pcm_close")?;
    let mut pcm = std::ptr::null_mut();
    let opened = unsafe { open(&mut pcm, c"default".as_ptr(), 0, 0) };
    if opened < 0 || pcm.is_null() {
        unsafe { libc::dlclose(lib) };
        return None;
    }
    // SND_PCM_FORMAT_S16_LE = 2, SND_PCM_ACCESS_RW_INTERLEAVED = 3.
    let params = unsafe { set_params(pcm, 2, 3, 1, rate, 1, 100_000) };
    if params < 0 {
        unsafe {
            close(pcm);
            libc::dlclose(lib);
        }
        return None;
    }
    Some(Playback {
        pcm,
        writei,
        prepare,
        drain,
        close,
        lib,
    })
}

#[cfg(target_os = "linux")]
fn load_fn<T>(lib: *mut libc::c_void, name: &std::ffi::CStr) -> Option<T> {
    let symbol = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if symbol.is_null() {
        unsafe { libc::dlclose(lib) };
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&symbol) })
    }
}

#[cfg(target_os = "linux")]
fn write_playback(device: &Playback, samples: &[i16]) {
    let mut offset = 0;
    let mut recovered = false;
    while offset < samples.len() {
        let wrote = unsafe {
            (device.writei)(
                device.pcm,
                samples[offset..].as_ptr().cast(),
                (samples.len() - offset) as libc::c_ulong,
            )
        };
        if wrote < 0 {
            if recovered {
                break;
            }
            recovered = true;
            unsafe { (device.prepare)(device.pcm) };
            continue;
        }
        offset += wrote as usize;
        recovered = false;
    }
    unsafe { (device.drain)(device.pcm) };
}

#[cfg(target_os = "linux")]
fn close_playback(device: Playback) {
    unsafe {
        (device.close)(device.pcm);
        libc::dlclose(device.lib);
    }
}

#[cfg(target_os = "macos")]
fn play_macos() {
    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioServicesPlayAlertSound(sound: u32);
    }
    unsafe { AudioServicesPlayAlertSound(0x0000_1000) };
}

#[cfg(windows)]
fn play_windows() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn Beep(frequency: u32, duration: u32) -> i32;
    }
    unsafe {
        Beep(HZ, MILLIS);
    }
}

#[cfg(windows)]
fn play_windows_file(path: &str) {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(name: *const u16, module: *mut core::ffi::c_void, flags: u32) -> i32;
    }
    let mut wide: Vec<u16> = path.encode_utf16().collect();
    wide.push(0);
    unsafe {
        PlaySoundW(wide.as_ptr(), core::ptr::null_mut(), 0x0002_0000 | 0x0002);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_has_the_requested_length_and_is_audible() {
        let samples = tone(22_050, 880, 140);
        assert_eq!(samples.len(), 22_050 * 140 / 1000);
        assert!(samples.iter().any(|sample| sample.abs() > 1000));
    }
}
