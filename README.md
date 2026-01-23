# BikeMeter

> Mountain bike telemetry recorder with video overlay capabilities

## Project Overview

BikeMeter is a hardware + software project that captures sensor data during mountain bike rides and creates video overlays with telemetry for first-person action camera footage. The system records g-forces, pitch angles, and other orientation data using a Raspberry Pi Pico 2 and BNO055 IMU sensor, then processes this data to create informative video overlays.

## Hardware Requirements

| Component | Description |
|-----------|-------------|
| **Raspberry Pi Pico 2** | RP2350 microcontroller |
| **BNO055** | 9-axis Absolute Orientation Sensor |
| **MicroSD Card Module** | SPI interface |
| **MicroSD Card** | For data logging (Class 10 recommended) |
| **Connecting wires** | Jumper wires or custom PCB |
| **Power source** | Battery pack for portability |

## Architecture

```
┌─────────────┐     I2C      ┌─────────┐
│ Pico 2      │ ───────────>│ BNO055  │
│ (RP2350)    │ <───────────│ Sensor  │
└──────┬──────┘              └─────────┘
       │ SPI
       ↓
┌─────────────┐
│ MicroSD     │
│ Card        │
└─────────────┘

Firmware logs CSV data → Desktop app reads CSV + video → Overlay output
```

## Research Summary

### Embedded Rust Crates (Firmware)

| Crate | Purpose | Source |
|-------|---------|--------|
| `rp235x-hal` | Hardware Abstraction Layer for RP2350/Pico 2 | [docs.rs](https://docs.rs/rp235x-hal) |
| `embassy-rp` | Async embedded framework for RP chips | [embassy.dev](https://embassy.dev) |
| `bno055` | Driver for BNO055 IMU sensor | [crates.io](https://crates.io/crates/bno055) |
| `embedded-sdmmc` | FAT filesystem for SD cards (no_std) | [GitHub](https://github.com/rust-embedded-community/embedded-sdmmc-rs) |
| `embedded-hal` | Hardware abstraction traits | [embedded.rs](https://embedded.rs) |

### Desktop Application Crates (Video Overlay)

| Crate | Purpose | Source |
|-------|---------|--------|
| `ffmpeg-next` | Safe FFmpeg wrapper for video processing | [crates.io](https://crates.io/crates/ffmpeg-next) |
| `plotters` | Chart/graph drawing library | [docs.rs](https://docs.rs/plotters) |
| `csv` + `serde` | CSV parsing and serialization | [docs.rs/csv](https://docs.rs/csv) |
| `clap` | Command-line argument parsing | [docs.rs/clap](https://docs.rs/clap) |
| `indicatif` | Progress bar display | [crates.io](https://crates.io/crates/indicatif) |

### BNO055 Sensor Specifications

- **Type**: 9-axis Absolute Orientation Sensor (SiP)
- **Components**: Triaxial 14-bit accelerometer, triaxial 16-bit gyroscope, magnetometer
- **Acceleration Ranges**: ±2g, ±4g, ±8g, ±16g (configurable)
- **Communication**: I2C (default address: 0x28, alt: 0x29), SPI, UART
- **Output**: Euler angles (pitch, roll, yaw), quaternions, linear acceleration
- **Limitation**: Euler angles accurate only for pitch/roll < 45° (use quaternions for larger angles)

## Installation

### Prerequisites

**For firmware:**
- Rust stable toolchain with `thumbv8m.main-none-eabihf` target
- `probe-rs` CLI tool for flashing

```bash
rustup target add thumbv8m.main-none-eabihf
cargo install probe-rs --features cli
```

**For overlay application:**
- FFmpeg libraries installed on your system

```bash
# Ubuntu/Debian
sudo apt install ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswscale-dev

# macOS
brew install ffmpeg
```

### Build

```bash
# Build firmware
cd firmware
cargo build --release

# Build overlay app
cd overlay
cargo build --release
```

## Usage

### Flashing Firmware

```bash
cd firmware
cargo embed --chip RP2350 --release
```

Or using probe-rs directly:

```bash
probe-rs run --chip RP2350 target/thumbv8m.main-none-eabihf/release/bikemeter-firmware
```

### Running Overlay Application

```bash
cd overlay
cargo run --release -- \
    --input footage.mp4 \
    --data ride_data.csv \
    --output output.mp4
```

#### Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--input` | `-i` | Input video file | Required |
| `--data` | `-d` | CSV data file with sensor readings | Required |
| `--output` | `-o` | Output video file | Required |
| `--time-offset` | `-t` | Time offset in ms to sync with video | 0 |
| `--opacity` | `-o` | Overlay opacity (0.0 - 1.0) | 0.85 |
| `--font-size` | | Font size for text overlay | 24 |
| `--preview` | | Show preview frames | false |
| `--sample-rate` | | Sampling rate of sensor data (Hz) | 100 |

## Data Format

### CSV Format

```csv
timestamp_ms,accel_x,accel_y,accel_z,pitch,roll,yaw,quat_w,quat_x,quat_y,quat_z
0,0.012,-0.025,1.004,5.2,-2.1,180.3,0.999,0.001,-0.002,0.045
100,0.015,-0.018,0.998,5.5,-1.9,180.5,0.999,0.001,-0.002,0.044
...
```

### Field Descriptions

| Field | Description | Units |
|-------|-------------|-------|
| `timestamp_ms` | Time since recording started | milliseconds |
| `accel_x/y/z` | Raw acceleration | g |
| `pitch` | Forward/backward tilt | degrees |
| `roll` | Left/right tilt | degrees |
| `yaw` | Heading/rotation | degrees |
| `quat_w/x/y/z` | Quaternion orientation | unitless |

## Wiring Diagram

### Pico 2 to BNO055 (I2C)

| Pico 2 | BNO055 |
|--------|--------|
| GP4 | SDA |
| GP5 | SCL |
| 3V3 | VDD |
| GND | GND |

### Pico 2 to SD Card Module (SPI)

| Pico 2 | SD Module |
|--------|-----------|
| GP11 | CLK/SCK |
| GP12 | MOSI/DI |
| GP13 | MISO/DO |
| GP15 | CS/CD |
| 3V3 | VCC |
| GND | GND |

## Implementation Plan

### Phase 1: Firmware Development (Complete)

- [x] Create Cargo workspace with `firmware/` and `overlay/` members
- [x] Configure `firmware/Cargo.toml` with embedded dependencies
- [x] Set up memory layout and build for RP2350
- [x] Initialize I2C bus on Pico 2
- [x] Configure BNO055 in NDOF (9 degrees of freedom) mode
- [x] Read acceleration and orientation data
- [ ] Complete SD card logging with CSV format
- [ ] Add file rotation for long rides
- [ ] Add LED indicators for recording status
- [ ] Implement safe shutdown handling

### Phase 2: Desktop Overlay Application (Complete)

- [x] Parse CSV files from SD card
- [x] Handle timestamp synchronization
- [x] Create gauge graphics for g-forces
- [x] Generate pitch/roll indicators
- [ ] Complete FFmpeg video compositing
- [ ] Add configurable overlay styles
- [ ] Export final video file

### Phase 3: Future Enhancements

- [ ] Web-based configuration interface
- [ ] Real-time Bluetooth data streaming
- [ ] Support for multiple sensors (GPS, speed, cadence)
- [ ] Custom overlay themes
- [ ] Mobile app for live preview

## Troubleshooting

### Firmware

**BNO055 not detected:**
- Check I2C wiring (SDA/SCL not swapped)
- Verify pull-up resistors on I2C lines
- Try alternative I2C address (0x29) in firmware

**SD card errors:**
- Ensure SPI pins are correctly wired
- Check SD card formatting (FAT32 recommended)
- Try slower SPI clock speed in firmware

### Overlay Application

**FFmpeg not found:**
- Install FFmpeg development libraries
- Set `FFMPEG_DIR` environment variable if needed

**Video out of sync:**
- Adjust `--time-offset` to align sensor data with video
- Check that video frame rate matches recording

## References

- [Rust on Pi Pico 2 Guide](https://murraytodd.medium.com/rust-with-the-raspberry-pi-pico-2-rp2350-e5f537af1c25)
- [rp-hal Repository](https://github.com/rp-rs/rp-hal)
- [BNO055 Datasheet](https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bno055-ds000.pdf)
- [Embassy Framework](https://embassy.dev)
- [Embassy I2C Tutorial](https://dev.to/theembeddedrustacean/embedded-rust-embassy-i2c-temperature-sensing-with-bmp180-6on)

## License

MIT OR Apache-2.0
