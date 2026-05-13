//! BikeMeter Firmware library.
//!
//! Provides sensor/storage abstractions and recording business logic
//! that can be tested on the host with mock implementations.

#![cfg_attr(not(test), no_std)]

pub mod sensor;
pub mod storage;
pub mod business;

// Embedded-only adapters (not available in host tests)
#[cfg(target_os = "none")]
pub mod bno055_adapter;
#[cfg(target_os = "none")]
pub mod sdmmc_storage;

// Re-export primary types
pub use sensor::{Sensor, SensorReading, SensorError, Vector3};
pub use storage::{Storage, StorageError};
pub use business::{Recorder, RecorderEvent};
