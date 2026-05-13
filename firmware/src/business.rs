//! Recording business logic.
//!
//! Generic recorder that operates on trait-based sensor and storage,
//! with CSV formatting and file rotation logic.

use core::fmt::Write;

use crate::sensor::{Sensor, SensorReading, Vector3};
use crate::storage::{Storage, StorageError};

// ============================================================================
// Constants
// ============================================================================

/// Maximum CSV file size before rotation (~50 MB).
pub const MAX_FILE_SIZE: u32 = 50_000_000;

/// Flush storage every N writes.
pub const FLUSH_INTERVAL: u32 = 10;

/// CSV header line matching the overlay app's expected format.
pub const CSV_HEADER: &[u8] =
    b"timestamp_ms,accel_x,accel_y,accel_z,pitch,roll,yaw,quat_w,quat_x,quat_y,quat_z\n";

// ============================================================================
// Recorder Event
// ============================================================================

#[derive(Debug)]
pub enum RecorderEvent {
    SampleRecorded {
        g_force: f32,
        pitch: f32,
        roll: f32,
    },
    FileRotated,
    SensorError,
    StorageError,
}

// ============================================================================
// Recorder
// ============================================================================

pub struct Recorder<S: Sensor, St: Storage> {
    sensor: S,
    storage: St,
    write_count: u32,
    bytes_written: u32,
}

impl<S: Sensor, St: Storage> Recorder<S, St> {
    pub fn new(sensor: S, storage: St) -> Self {
        Self {
            sensor,
            storage,
            write_count: 0,
            bytes_written: 0,
        }
    }

    /// Write CSV header. Call once after construction.
    pub fn init(&mut self) -> Result<(), StorageError> {
        self.storage.write(CSV_HEADER)?;
        self.bytes_written += CSV_HEADER.len() as u32;
        Ok(())
    }

    /// Execute one recording iteration.
    pub fn tick(&mut self, timestamp_ms: u64) -> RecorderEvent {
        let reading = match self.sensor.read() {
            Ok(r) => r,
            Err(_) => return RecorderEvent::SensorError,
        };

        let g_force = calc_g_force(&reading.linear_accel);

        let csv_line = format_csv_line(timestamp_ms, &reading);
        let line_len = csv_line.len() as u32;

        if self.storage.write(csv_line.as_bytes()).is_err() {
            return RecorderEvent::StorageError;
        }
        self.bytes_written += line_len;

        // Periodic flush
        self.write_count += 1;
        if self.write_count % FLUSH_INTERVAL == 0 {
            let _ = self.storage.flush();
        }

        let event = RecorderEvent::SampleRecorded {
            g_force,
            pitch: reading.pitch,
            roll: reading.roll,
        };

        // File rotation
        if self.bytes_written >= MAX_FILE_SIZE {
            if self.storage.flush().is_ok() && self.storage.rotate().is_ok() {
                self.bytes_written = CSV_HEADER.len() as u32;
                return RecorderEvent::FileRotated;
            }
        }

        event
    }

    /// Flush storage for graceful shutdown.
    pub fn shutdown(&mut self) {
        let _ = self.storage.flush();
    }
}

// ============================================================================
// Pure Functions
// ============================================================================

/// Calculate g-force magnitude from linear acceleration.
pub fn calc_g_force(lin_accel: &Vector3) -> f32 {
    libm::sqrtf(lin_accel.x * lin_accel.x + lin_accel.y * lin_accel.y + lin_accel.z * lin_accel.z)
}

/// Format a sensor reading as a CSV line.
pub fn format_csv_line(timestamp_ms: u64, reading: &SensorReading) -> heapless::String<256> {
    let mut line = heapless::String::<256>::new();
    let _ = write!(line, "{},", timestamp_ms);
    fmt_float(&mut line, reading.accel.x); line.push(',').ok();
    fmt_float(&mut line, reading.accel.y); line.push(',').ok();
    fmt_float(&mut line, reading.accel.z); line.push(',').ok();
    fmt_float(&mut line, reading.pitch);   line.push(',').ok();
    fmt_float(&mut line, reading.roll);    line.push(',').ok();
    fmt_float(&mut line, reading.yaw);     line.push(',').ok();
    fmt_float(&mut line, reading.quat_w);  line.push(',').ok();
    fmt_float(&mut line, reading.quat_x);  line.push(',').ok();
    fmt_float(&mut line, reading.quat_y);  line.push(',').ok();
    fmt_float(&mut line, reading.quat_z);
    line.push('\n').ok();
    line
}

/// Format a float with 3 decimal places.
pub fn fmt_float(s: &mut heapless::String<256>, val: f32) {
    let scaled = (val * 1000.0) as i32;
    let whole = scaled / 1000;
    let frac = (scaled % 1000).unsigned_abs();
    if whole < 0 {
        let _ = s.push('-');
    }
    let _ = write!(s, "{}.{:03}", whole.unsigned_abs(), frac);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::SensorError;

    // -- Mock implementations --

    struct MockSensor {
        reading: SensorReading,
        should_fail: bool,
    }

    impl Sensor for MockSensor {
        fn read(&mut self) -> Result<SensorReading, SensorError> {
            if self.should_fail {
                Err(SensorError::CommError)
            } else {
                Ok(self.reading)
            }
        }
    }

    struct MockStorage {
        written: Vec<u8>,
        flush_count: u32,
        rotate_count: u32,
        should_fail: bool,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                written: Vec::new(),
                flush_count: 0,
                rotate_count: 0,
                should_fail: false,
            }
        }
    }

    impl Storage for MockStorage {
        fn write(&mut self, data: &[u8]) -> Result<(), StorageError> {
            if self.should_fail {
                return Err(StorageError::WriteFailed);
            }
            self.written.extend_from_slice(data);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), StorageError> {
            self.flush_count += 1;
            Ok(())
        }
        fn rotate(&mut self) -> Result<(), StorageError> {
            self.rotate_count += 1;
            Ok(())
        }
    }

    fn test_reading() -> SensorReading {
        SensorReading {
            accel: Vector3 { x: 0.012, y: -0.025, z: 1.004 },
            linear_accel: Vector3 { x: 0.5, y: -0.3, z: 9.8 },
            pitch: 5.2,
            roll: -2.1,
            yaw: 180.3,
            quat_w: 0.999,
            quat_x: 0.001,
            quat_y: -0.002,
            quat_z: 0.045,
        }
    }

    // -- Tests --

    #[test]
    fn csv_format_matches_contract() {
        let reading = test_reading();
        let line = format_csv_line(100, &reading);
        // Header order: timestamp_ms,accel_x,accel_y,accel_z,pitch,roll,yaw,quat_w,quat_x,quat_y,quat_z
        assert!(line.starts_with("100,"));
        assert!(line.ends_with("\n"));
        // Check all 11 fields are present
        let parts: Vec<&str> = line.trim_end().split(',').collect();
        assert_eq!(parts.len(), 11);
        assert_eq!(parts[0], "100"); // timestamp
        // pitch=5.2, roll=-2.1, yaw=180.3
        assert!(parts[4].starts_with("5.2"));
        assert!(parts[5].starts_with("-2.1"));
        assert!(parts[6].starts_with("180.3"));
    }

    #[test]
    fn g_force_calculation() {
        let v = Vector3 { x: 3.0, y: 4.0, z: 0.0 };
        let g = calc_g_force(&v);
        assert!((g - 5.0).abs() < 0.001);
    }

    #[test]
    fn g_force_zero() {
        let v = Vector3 { x: 0.0, y: 0.0, z: 0.0 };
        assert_eq!(calc_g_force(&v), 0.0);
    }

    #[test]
    fn recorder_tick_writes_to_storage() {
        let sensor = MockSensor { reading: test_reading(), should_fail: false };
        let storage = MockStorage::new();
        let mut recorder = Recorder::new(sensor, storage);
        recorder.init().unwrap();

        let event = recorder.tick(0);
        match event {
            RecorderEvent::SampleRecorded { .. } => {}
            e => panic!("Expected SampleRecorded, got {:?}", e),
        }

        // Should have header + one CSV line
        let written = &recorder.storage.written;
        assert!(written.len() > CSV_HEADER.len());
        // First line is header
        assert!(written.starts_with(b"timestamp_ms,"));
    }

    #[test]
    fn recorder_sensor_error() {
        let sensor = MockSensor { reading: test_reading(), should_fail: true };
        let storage = MockStorage::new();
        let mut recorder = Recorder::new(sensor, storage);
        recorder.init().unwrap();

        let event = recorder.tick(0);
        assert!(matches!(event, RecorderEvent::SensorError));
    }

    #[test]
    fn recorder_storage_error() {
        let sensor = MockSensor { reading: test_reading(), should_fail: false };
        let storage = MockStorage::new();
        // init succeeds, then writes fail
        let mut recorder = Recorder::new(sensor, storage);
        recorder.init().unwrap();
        recorder.storage.should_fail = true;

        let event = recorder.tick(0);
        assert!(matches!(event, RecorderEvent::StorageError));
    }

    #[test]
    fn flush_interval() {
        let sensor = MockSensor { reading: test_reading(), should_fail: false };
        let storage = MockStorage::new();
        let mut recorder = Recorder::new(sensor, storage);
        recorder.init().unwrap();

        for i in 0..FLUSH_INTERVAL - 1 {
            recorder.tick(i as u64);
        }
        assert_eq!(recorder.storage.flush_count, 0);

        recorder.tick(FLUSH_INTERVAL as u64);
        assert_eq!(recorder.storage.flush_count, 1);
    }

    #[test]
    fn rotation_triggered_at_max_size() {
        let sensor = MockSensor { reading: test_reading(), should_fail: false };
        let storage = MockStorage::new();
        let mut recorder = Recorder::new(sensor, storage);
        recorder.init().unwrap();

        // Force bytes_written past the limit
        recorder.bytes_written = MAX_FILE_SIZE;

        let event = recorder.tick(0);
        assert!(matches!(event, RecorderEvent::FileRotated));
        assert_eq!(recorder.storage.rotate_count, 1);
    }
}
