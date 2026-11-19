#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod autostart;
mod device;
mod icon;
mod protocol;
#[cfg(target_os = "linux")]
mod tray_linux;
#[cfg(windows)]
mod tray_windows;
#[cfg(windows)]
mod taskbar_windows;

use device::{Reader, Reading};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// While something is wrong (dongle unplugged, headset off) retry sooner.
const ERROR_RETRY: Duration = Duration::from_secs(60);
/// While audio is playing, check again for silence this often...
const BUSY_RECHECK: Duration = Duration::from_secs(30);
/// ...but read anyway once a scheduled read is this late.
const MAX_DEFER: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug)]
pub enum Status {
    Pending,
    Ok(Reading),
    Err(String),
}

impl Status {
    pub fn percent(&self) -> Option<u8> {
        match self {
            Status::Ok(r) => Some(r.percent),
            _ => None,
        }
    }

    pub fn text(&self) -> String {
        match self {
            Status::Pending => "G435: reading in progress…".into(),
            Status::Ok(r) => format!("Logitech G435: {}%", r.percent),
            Status::Err(e) => format!("G435: {e}"),
        }
    }
}

/// Reads the battery every `interval` (or when something arrives on
/// `refresh`) and reports each result. Scheduled reads wait until the headset
/// is silent, up to `MAX_DEFER`; manual ones happen right away.
/// Returns when `refresh` is closed.
pub fn poll_loop(refresh: Receiver<()>, interval: Duration, on_update: impl Fn(Status)) {
    let mut reader = Reader::new();
    let mut due = Instant::now();
    // The first read happens right away: games and browsers keep the stream
    // open even when silent, and the icon would otherwise stay empty.
    let mut manual = true;
    loop {
        let wait = if !manual && due.elapsed() < MAX_DEFER && audio::headset_busy() {
            device::debug(|| "audio in use, playback postponed".into());
            BUSY_RECHECK
        } else {
            let status = match reader.read_battery() {
                Ok(r) => Status::Ok(r),
                Err(e) => Status::Err(e.to_string()),
            };
            let wait = if matches!(status, Status::Ok(_)) {
                interval
            } else {
                ERROR_RETRY.min(interval)
            };
            on_update(status);
            due = Instant::now() + wait;
            wait
        };
        manual = match refresh.recv_timeout(wait) {
            Ok(()) => {
                while refresh.try_recv().is_ok() {}
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
    }
}

/// Headset-side functions from the G435 quick start guide. They are handled
/// inside the headset and send nothing over USB, so the tray can only list them.
pub const BUTTON_COMBOS: &[(&str, &str)] = &[
    ("Mute Microphone", "Mute, short press"),
    (
        "Sidetone (hear your voice) on/off",
        "Mute, double press",
    ),
    ("Sidetone Level", "Mute + Volume +/−"),
    ("Max Volume 100 dB", "hold Mute + Volume+ for 3s"),
    ("Max Volume 85 dB", "hold Mute + Volume− for 3s"),
    ("Battery Level on LED", "Power, short press"),
    ("LIGHTSPEED ↔ Bluetooth", "hold Mute for 3s"),
    ("Bluetooth pairing", "press Power + Mute for 3s"),
    ("Power On/Off", "hold Power for 5s"),
];

pub fn combo_labels() -> impl Iterator<Item = String> {
    BUTTON_COMBOS
        .iter()
        .map(|(what, how)| format!("{what}: {how}"))
}

const USAGE: &str = "\
g435-tray: system tray icon for the Logitech G435 (LIGHTSPEED) battery status

Usage: g435-tray [options]
  --once            read the battery level once, print it, and exit
  --interval <min>  minutes between readings (default: 15)
  -h, --help        show this help message

Automatic readings wait until the headset is not playing audio
(for up to 30 minutes), as each reading interrupts both audio and the microphone for approximately 0.6 seconds.

Set the environment variable G435_DEBUG=1 to print HID traffic to stderr.";

fn main() {
    let mut once = false;
    let mut interval = Duration::from_secs(15 * 60);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--once" => once = true,
            "--interval" => match args.next().and_then(|v| v.parse::<u64>().ok()) {
                Some(min) if min > 0 => interval = Duration::from_secs(min * 60),
                _ => fail("--interval requires a number of minutes > 0"),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => fail(&format!("unknown option: {other}")),
        }
    }

    if once {
        match Reader::new().read_battery() {
            Ok(r) => println!("{}% ({} mV)", r.percent, r.millivolts),
            Err(e) => fail(&e.to_string()),
        }
        return;
    }

    #[cfg(target_os = "linux")]
    tray_linux::run(interval);
    #[cfg(windows)]
    tray_windows::run(interval);
}

fn fail(msg: &str) -> ! {
    eprintln!("g435-tray: {msg}");
    std::process::exit(1);
}
