# Logitech G435 Battery Indicator

A lightweight Windows tray and taskbar utility that displays the battery level of the **Logitech G435** wireless headset through its LIGHTSPEED USB dongle.

- **Two display modes:** Choose between a **System Tray icon** (headphone glyph) or a **Taskbar widget** pinned next to the notification area showing the headphone icon and battery percentage, with a hover highlight and Windows 11-style tooltip.
- **Theme-aware & low-battery alert:** Automatically adapts to Windows Light and Dark modes and turns red at `≤15%`.
- **Menu:** Refresh now, toggle *Show on Taskbar*, *Auto start with Windows*, a cheat sheet of headset button combinations (*Headset Commands*), and exit.
- **Automatic polling:** Reads the battery every 15 minutes and retries every minute after an error.
- **Lightweight & private:** No G HUB, no Logitech DLLs, and no network traffic.

> Unofficial project, not affiliated with Logitech.

## Supported dongles

| PID | Revision | Transport | Status |
|-----|----------|-----------|--------|
| `046d:0acb` | HDT647 (most common) | report 6 through the Realtek bridge ("bootloader") | **tested** |
| `046d:0adf` | newer revision | 64-byte reports with CRC-16 | ported from [portyporty][porty], untested |

To check which dongle revision you have, open **Device Manager** → **Human Interface Devices** (or **Sound, video and game controllers**) and check the **Hardware IDs** under **Properties → Details**.

## Installing

Download the latest release archive from the [Releases](https://github.com/SAPNXTDOOR/Logitech-G435-Battery-Level-Indicator/releases/latest) page, extract it, and run `Logitech-G435.exe`.

You can enable **Auto start with Windows** and toggle **Show on Taskbar** directly from the icon's menu.

## Building

Requires the Rust MSVC toolchain ([rustup.rs](https://rustup.rs/)). `hidapi` uses its native Rust Windows backend, so no additional system libraries are needed.

```powershell
cargo build --release
```

The compiled executable is created at `target\release\Logitech-G435.exe` with the embedded headphone application icon.

## Usage

```powershell
.\Logitech-G435.exe                        # start the tray/taskbar app
.\Logitech-G435.exe --once                 # print battery reading and exit
.\Logitech-G435.exe --interval 30          # read every 30 minutes
$env:G435_DEBUG="1"; .\Logitech-G435.exe --once   # dump the raw HID traffic
```

## Notes and limitations

- **The percentage is an estimate** based on the Li-Po battery voltage ($4.2\text{ V} = 100\%$, $3.4\text{ V} = 0\%$). While charging, the voltage rises and the estimate reads higher than the real charge.
- On the `0acb` dongle, every read briefly puts the USB bridge into "bootloader" mode, and **audio and microphone pause for about 0.6 seconds**. This is a firmware limitation of the dongle (see [docs/PROTOCOL.md](docs/PROTOCOL.md)), which is why the app polls every 15 minutes by default.
- The battery command only reaches the headset when the dongle is in *remote mode* 1. If it isn't, the program sets it automatically (the same value the official pairing tool uses).
- If the dongle stops answering (it acknowledges commands but returns no data), unplug it and plug it back in.
- Sidetone, the 85/100 dB volume limit, and LIGHTSPEED/Bluetooth switching are handled internally by the headset through button combinations, which are listed in the menu under **Headset Commands**.

## How it works (protocol)

Both dongles communicate with the same audio SoC (Airoha/Audiowise) using `"PA"` commands:

```text
50 41 <category> <seq> <len> <command...>     request
50 49 ...                                     reply
```

| Command | Bytes | Reply |
|---|---|---|
| REMOTE_GET_CONNECT_STATUS | `0e .. 01 e3` | `02 00 e3 <0/1>` |
| REMOTE_GET_MODE / SET_MODE | `0e .. 01 e0` / `0e .. 02 e1 01` | `02 00 e0 <mode>` |
| GET_BATTERY | `06 .. 01 00 31` | `07 00 06 <seq\|80> 00 <mV lo> <mV hi> ..` |

On the `0acb` dongle, the transport sequence is: enter bootloader with output report `06 01 03`, send the command with `06 04 03 <len lo> <len hi> <cmd>` (ack `06 5a` on the input endpoint), read the reply with `GET_FEATURE` on report 6, and exit with `06 08 03`. Details, timings, and protocol notes are in [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Other features found (not implemented yet)

Besides the battery, existing protocol research documents these commands:

- **Headset connection status** (on/off): works, and the program already uses it to report when the headset is off.
- **RSSI / radio link quality** (`GET_RSSI`, `LQ_GET_QUALTY`): candidates for a signal indicator, though unverified and known to occasionally hang the dongle.
- **Firmware version** (`OTA_CMD_GET_IMAGE_VERSION`).
- **LEDs** (`CMD_SET_LED`, `CMD_SET_LED_PWM`) and **EQ/DSP** (`EQ_ON_OFF`, `DSP_CMD_IIR_EQ`, ...): present in shared firmware, though they may have no effect on the G435.
- **Dongle ↔ headset pairing** (scan and connect by MAC address): used by the official re-pairing tool.
- **Dangerous** commands to avoid: OTA/flash/NVDS (`ERASE_LOAD_REGION`, `OTA_CMD_UPDATE`, `AW_HA_CLEAR_NVDS`, ...).

No USB commands have been found for sidetone, the volume limit, reading hardware mute state, or detecting charging.

## License

[MIT](LICENSE): anyone can use, modify, and redistribute the code, including commercially, as long as the license file is kept. Parts derived from other projects are listed in [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES).

## Sources

- [endes — "USB HID down the rabbit hole: Logitech G435 dongle"](https://dev.to/endes/usb-hid-down-the-rabbit-hole-logitech-g435-dongle-33if) and the [command list gist](https://gist.github.com/endes0/21193c55e2e38c9b3e3a95492380059c)
- [portyporty/g435-battery-percentage][porty]: Windows tray app, `0adf` dongle, voltage→percentage curve
- [GotardoN1/G435-Companion](https://github.com/GotardoN1/G435-Companion): Bluetooth battery through Windows PnP properties
- [CSaratakij/G435BatteryApproximate](https://github.com/CSaratakij/G435BatteryApproximate): estimate based on usage time, no real reading

[porty]: https://github.com/portyporty/g435-battery-percentage
