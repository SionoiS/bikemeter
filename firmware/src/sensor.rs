//! Sensor abstraction layer.

/// 3D vector used for acceleration and other spatial data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Complete sensor reading from an IMU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorReading {
    pub accel: Vector3,
    pub linear_accel: Vector3,
    /// Forward/backward tilt in degrees.
    pub pitch: f32,
    /// Left/right tilt in degrees.
    pub roll: f32,
    /// Heading in degrees.
    pub yaw: f32,
    pub quat_w: f32,
    pub quat_x: f32,
    pub quat_y: f32,
    pub quat_z: f32,
}

#[derive(Debug)]
pub enum SensorError {
    CommError,
}

/// Trait for reading IMU sensor data.
pub trait Sensor {
    fn read(&mut self) -> Result<SensorReading, SensorError>;
}
