//! BNO055 IMU sensor adapter.

use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::I2c;

use crate::sensor::{Sensor, SensorError, SensorReading, Vector3};

pub struct Bno055Sensor<I> {
    imu: bno055::Bno055<I>,
}

impl<I> Bno055Sensor<I>
where
    I: I2c,
{
    pub fn new(i2c: I) -> Self {
        Self {
            imu: bno055::Bno055::new(i2c),
        }
    }

    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), bno055::Error<I::Error>> {
        self.imu.init(delay)
    }

    pub fn set_mode(
        &mut self,
        mode: bno055::BNO055OperationMode,
        delay: &mut impl DelayNs,
    ) -> Result<(), bno055::Error<I::Error>> {
        self.imu.set_mode(mode, delay)
    }

    pub fn id(&mut self) -> Result<u8, bno055::Error<I::Error>> {
        self.imu.id()
    }
}

impl<I> Sensor for Bno055Sensor<I>
where
    I: I2c,
{
    fn read(&mut self) -> Result<SensorReading, SensorError> {
        let accel = self.imu.accel_data().map_err(|_| SensorError::CommError)?;
        let euler = self.imu.euler_angles().map_err(|_| SensorError::CommError)?;
        let quat = self.imu.quaternion().map_err(|_| SensorError::CommError)?;
        let lin_accel = self.imu.linear_acceleration().map_err(|_| SensorError::CommError)?;

        // bno055 euler_angles() returns mint::EulerAngles::from([roll, pitch, heading])
        // so a=roll, b=pitch, c=heading
        Ok(SensorReading {
            accel: Vector3 { x: accel.x, y: accel.y, z: accel.z },
            linear_accel: Vector3 { x: lin_accel.x, y: lin_accel.y, z: lin_accel.z },
            pitch: euler.b,
            roll: euler.a,
            yaw: euler.c,
            quat_w: quat.s,
            quat_x: quat.v.x,
            quat_y: quat.v.y,
            quat_z: quat.v.z,
        })
    }
}
