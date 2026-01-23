//! BikeMeter Firmware
//!
//! Embedded firmware for Raspberry Pi Pico 2 + BNO055 IMU sensor.
//! Records telemetry data to SD card for later video overlay processing.

#![no_std]
#![no_main]

use core::cell::RefCell;

use bno055::{Bno055, Bno055Sensor, Vector3};
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::i2c::I2c;
use embedded_sdmmc::{Controller, Mode, TimeSource, VolumeIdx};
use panic_probe as _;

// RP2350 HAL imports
use rp235x_hal::{
    self as hal,
    gpio::{self, Pin},
    i2c::I2C as HalI2C,
    rom_data::reset_to_usb_boot,
    spi::Spi as HalSpi,
    {clocks, watchdog},
};

// Re-export defmt for the binary
use defmt::*;

// Embedded IO traits
use embedded_io::Write;

// Critical section
use critical_section::Mutex;

// ============================================================================
// Type Aliases & Pin Definitions
// ============================================================================

/// Global state for shared resources
struct GlobalState {
    led: Option<gpio::Pin<gpio::Bank0, 25, gpio::PushPullOutput>>,
}

static STATE: Mutex<RefCell<GlobalState>> = Mutex::new(RefCell::new(GlobalState { led: None }));

/// I2C on GP4 (SDA) and GP5 (SCL) for BNO055
type BnoI2C = HalI2C<
    rp235x_hal::pio::PIO0,
    rp235x_hal::pio::SM0,
    gpio::Pin<gpio::Bank0, 4, gpio::Function<2>>,
    gpio::Pin<gpio::Bank0, 5, gpio::Function<2>>,
>;

/// SPI for SD Card on GP11 (SCK), GP12 (MOSI), GP13 (MISO)
/// CS on GP15
type SdSpi = HalSpi<
    rp235x_hal::spi::Enabled,
    rp235x_hal::spi::Spi<rp235x_hal::spi::Spi1,>,
    gpio::Pin<gpio::Bank0, 11, gpio::Function<2>>,
    gpio::Pin<gpio::Bank0, 12, gpio::Function<2>>,
    gpio::Pin<gpio::Bank0, 13, gpio::Function<2>>,
>;

type SdCs = gpio::Pin<gpio::Bank0, 15, gpio::PushPullOutput>;

// ============================================================================
// LED Control
// ============================================================================

struct Led;

impl Led {
    fn on() {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref(cs).led.as_mut() {
                let _ = led.set_high();
            }
        });
    }

    fn off() {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref(cs).led.as_mut() {
                let _ = led.set_low();
            }
        });
    }

    fn toggle() {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref(cs).led.as_mut() {
                let _ = embedded_hal::digital::OutputPin::toggle(led);
            }
        });
    }
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Format)]
enum Error {
    I2c(bno055::Error),
    SdCard(embedded_sdmmc::Error<rp235x_hal::spi::Error>),
    Io(embedded_io::ErrorKind),
}

impl From<bno055::Error> for Error {
    fn from(e: bno055::Error) -> Self {
        Error::I2c(e)
    }
}

impl From<embedded_sdmmc::Error<rp235x_hal::spi::Error>> for Error {
    fn from(e: embedded_sdmmc::Error<rp235x_hal::spi::Error>) -> Self {
        Error::SdCard(e)
    }
}

// ============================================================================
// BNO055 Async Wrapper
// ============================================================================

struct AsyncI2c<'a, I2C>(Mutex<RefCell<&'a mut I2C>>);

impl<'a, I2C> embedded_hal_async::i2c::I2c for AsyncI2c<'a, I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    async fn read(&mut self, address: u8, buffer: &mut [u8]) -> Result<(), Self::Error> {
        critical_section::with(|cs| {
            self.0
                .borrow_ref(cs)
                .read(address, buffer)
                .map_err(|_| embedded_hal_async::i2c::ErrorKind::Other)?;
            Ok(())
        })
    }

    async fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        critical_section::with(|cs| {
            self.0
                .borrow_ref(cs)
                .write(address, bytes)
                .map_err(|_| embedded_hal_async::i2c::ErrorKind::Other)?;
            Ok(())
        })
    }

    async fn write_read(
        &mut self,
        address: u8,
        bytes: &[u8],
        buffer: &mut [u8],
    ) -> Result<(), Self::Error> {
        critical_section::with(|cs| {
            self.0
                .borrow_ref(cs)
                .write_read(address, bytes, buffer)
                .map_err(|_| embedded_hal_async::i2c::ErrorKind::Other)?;
            Ok(())
        })
    }
}

// ============================================================================
// Main Function
// ============================================================================

#[cortex_m_rt::entry]
fn main() -> ! {
    info!("BikeMeter Firmware v{}", env!("CARGO_PKG_VERSION"));

    // ============================================================================
    // Hardware Initialization
    // ============================================================================

    let mut pac = hal::pac::Peripherals::take().unwrap();
    let core = hal::pac::CorePeripherals::take().unwrap();

    // Initialize watchdog
    let mut watchdog = watchdog::Watchdog::new(pac.WATCHDOG);

    // Configure clocks - run at 150MHz
    let clocks = clocks::init_clocks_and_plls(
        rp235x_hal::clocks::ExternalOscillator::new(pac.XOSC, &mut pac.PLL_SYS, &mut pac.PLL_USB),
        rp235x_hal::clocks::SystemClockConfig::default(),
        &mut pac.CLOCKS,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();

    // Initialize embassy executor
    let executor = embassy_executor::Executor::new(core.SYST, clocks);

    // Run the async main
    executor.run(|spawner| {
        unwrap!(spawner.start(async_main(
            pac.PIO0,
            pac.SPI1,
            pac.IO_BANK0,
            pac.PADS_BANK0,
            pac.RESETS,
        )))
    })
}

#[embassy_executor::main(entry = "cortex_m_rt::entry")]
async fn async_main(
    pio0: rp235x_hal::pio::PIO0,
    spi1: rp235x_hal::spi::Spi1,
    io_bank0: rp235x_hal::gpio::IO_BANK0,
    pads_bank0: rp235x_hal::gpio::PADS_BANK0,
    mut resets: rp235x_hal::resets::SubsystemReset<BlockMask>,
) -> ! {
    info!("Starting BikeMeter firmware...");

    // Init GPIO
    let mut pins = hal::gpio::Pins::new(
        io_bank0,
        pads_bank0,
        resets.into_reset_mask(),
    );

    // Initialize LED (GP25 - onboard LED)
    let mut led = pins.gp25.into_push_pull_output();
    Led::off();

    // Store LED in global state
    critical_section::with(|cs| {
        STATE.borrow_ref(cs).led = Some(led);
    });

    // Blink LED to indicate startup
    for _ in 0..3 {
        Led::on();
        Timer::after(Duration::from_millis(100)).await;
        Led::off();
        Timer::after(Duration::from_millis(100)).await;
    }

    // ========================================================================
    // Initialize I2C for BNO055
    // ========================================================================

    info!("Initializing I2C for BNO055...");

    // Configure PIO pins for I2C
    let sda = pins.gp4.into_function::<2>();
    let scl = pins.gp5.into_function::<2>();

    // Initialize I2C using PIO (Programmable I/O)
    // Note: Using standard I2C initialization for RP2350
    let mut i2c = {
        // Create I2C instance - simplified for rp235x-hal
        // The actual implementation depends on the HAL version
        // For now, we'll use a placeholder approach
        let i2c = unsafe { core::mem::zeroed() };
        i2c
    };

    // For actual implementation, the rp235x-hal provides I2C through PIO
    // The exact initialization depends on the HAL version

    // ========================================================================
    // Initialize BNO055 Sensor
    // ========================================================================

    info!("Initializing BNO055 sensor...");

    let mut bno055 = Bno055::new(Default::default())
        .with_alternative_address(false) // Use default address 0x28
        .init()
        .await;

    match bno055 {
        Ok(ref mut sensor) => {
            info!("BNO055 initialized successfully!");
            let chip_id = sensor.chip_id().await.unwrap();
            info!("BNO055 Chip ID: 0x{:02x}", chip_id);

            // Set to NDOF mode (9 degrees of freedom fusion)
            sensor.set_mode(bno055::BnoMode::NDOF).await.unwrap();
            info!("BNO055 mode: NDOF");
        }
        Err(e) => {
            error!("Failed to initialize BNO055: {:?}", e);
            // Fast blink to indicate error
            loop {
                Led::toggle();
                Timer::after(Duration::from_millis(50)).await;
            }
        }
    }

    // ========================================================================
    // Initialize SPI for SD Card
    // ========================================================================

    info!("Initializing SD card...");

    // Configure SPI pins
    let sck = pins.gp11.into_function::<2>();
    let mosi = pins.gp12.into_function::<2>();
    let miso = pins.gp13.into_function::<2>();
    let cs = pins.gp15.into_push_pull_output();

    let _sd_spi = {
        // Create SPI instance at 400kHz (initialization speed)
        // Will be increased later for faster transfers
        let spi = unsafe { core::mem::zeroed() };
        spi
    };

    let mut sd_cs = cs;

    // ========================================================================
    // Initialize SD Card and File System
    // ========================================================================

    let mut sd_controller = unsafe { core::mem::zeroed() };
    let sd_available = false;

    // For now, we'll continue without SD card and log via USB serial
    info!("SD card initialization skipped (not yet fully implemented)");

    // ========================================================================
    // Main Recording Loop
    // ========================================================================

    info!("Starting main recording loop...");
    Led::on(); // LED on = recording

    let mut start_time = embassy_time::Instant::now();

    // CSV data buffer
    let mut csv_line_buf = heapless::String<256>;
    let mut file_open = false;

    loop {
        let elapsed = start_time.elapsed().as_millis();

        // Read sensor data
        let sensor_data = if let Ok(ref mut sensor) = bno055 {
            // Try to read all sensor data
            let accel = sensor.accel().await;
            let mag = sensor.mag().await;
            let gyro = sensor.gyro().await;
            let euler = sensor.euler().await;
            let quat = sensor.quat().await;
            let lin_accel = sensor.linear_accel().await;

            match (accel, euler, quat, lin_accel) {
                (Ok(a), Ok(e), Ok(q), Ok(l)) => Some((a, e, q, l)),
                _ => None,
            }
        } else {
            None
        };

        // Process sensor data
        if let Some((accel, euler, quat, lin_accel)) = sensor_data {
            // Calculate g-force magnitude from linear acceleration (gravity removed)
            let g_force = (lin_accel.x * lin_accel.x
                + lin_accel.y * lin_accel.y
                + lin_accel.z * lin_accel.z)
                .sqrt();

            // Log data via defmt (available via USB serial with probe-rs)
            info!(
                "{},{},{},{},{},{},{},{},{},{},{}",
                elapsed,
                accel.x,
                accel.y,
                accel.z,
                euler.pitch,
                euler.roll,
                euler.yaw,
                quat.w,
                quat.x,
                quat.y,
                quat.z
            );

            // Blink LED on high g-forces (> 2g)
            if g_force > 2.0 {
                Led::toggle();
                Timer::after(Duration::from_millis(10)).await;
                Led::on();
            }
        }

        // Sample rate: 100Hz (10ms)
        Timer::after(Duration::from_millis(10)).await;
    }
}

// ============================================================================
// Panic Handler
// ============================================================================

#[defmt::panic_handler]
fn panic() -> ! {
    error!("PANIC!");
    Led::on();
    loop {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref(cs).led.as_mut() {
                let _ = embedded_hal::digital::OutputPin::toggle(led);
            }
        });
        Timer::after(Duration::from_millis(100)).await;
    }
}

// ============================================================================
// Interrupt Handlers
// ============================================================================

#[cortex_m_rt::interrupt]
unsafe fn SYSIRQCTRL() {
    // Handle system interrupts
}

// Required for defmt-rtt
#[cortex_m_rt::exception]
unsafe fn HardFrameBufferError(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    cortex_m::asm::udf()
}

#[cortex_m_rt::exception]
unsafe fn DefaultHandler(_irqn: i16) {
    cortex_m::asm::udf()
}
