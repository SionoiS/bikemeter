//! BikeMeter Firmware
//!
//! Embedded firmware for Raspberry Pi Pico 2 + BNO055 IMU sensor.
//! Records telemetry data to SD card for later video overlay processing.

#![no_std]
#![no_main]

use core::cell::RefCell;

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Input, Level, Output, Pull},
    i2c::{self, Config as I2cConfig, I2c},
    spi::{Config as SpiConfig, Spi},
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Delay, Duration, Timer};
use embedded_sdmmc::{Mode, VolumeIdx};
use panic_probe as _;

use bikemeter_firmware::bno055_adapter::Bno055Sensor;
use bikemeter_firmware::business::RecorderEvent;
use bikemeter_firmware::sdmmc_storage::{DummyTimeSource, SdmmcStorage};
use bikemeter_firmware::Recorder;

use critical_section::Mutex;

// ============================================================================
// System State
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum SystemState {
    Initializing,
    Recording,
    Stopped,
    SensorError,
    SdCardError,
}

static STATE_SIGNAL: Signal<CriticalSectionRawMutex, SystemState> = Signal::new();

// ============================================================================
// LED Control
// ============================================================================

struct GlobalState {
    led: Option<Output<'static>>,
}

static STATE: Mutex<RefCell<GlobalState>> = Mutex::new(RefCell::new(GlobalState { led: None }));

struct Led;

impl Led {
    fn on() {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref_mut(cs).led.as_mut() {
                let _ = led.set_high();
            }
        });
    }

    fn off() {
        critical_section::with(|cs| {
            if let Some(led) = STATE.borrow_ref_mut(cs).led.as_mut() {
                let _ = led.set_low();
            }
        });
    }
}

#[embassy_executor::task]
async fn led_task() {
    let mut current = SystemState::Initializing;
    let mut blink_on = false;

    loop {
        if let Some(new_state) = STATE_SIGNAL.try_take() {
            current = new_state;
            blink_on = false;
        }

        match current {
            SystemState::Initializing => {
                for _ in 0..3 {
                    Led::on();
                    Timer::after(Duration::from_millis(80)).await;
                    Led::off();
                    Timer::after(Duration::from_millis(80)).await;
                }
                Timer::after(Duration::from_millis(500)).await;
            }
            SystemState::Recording => {
                Led::on();
                Timer::after(Duration::from_millis(200)).await;
            }
            SystemState::Stopped => {
                Led::off();
                Timer::after(Duration::from_millis(200)).await;
            }
            SystemState::SensorError => {
                blink_on = !blink_on;
                if blink_on { Led::on() } else { Led::off() }
                Timer::after(Duration::from_millis(100)).await;
            }
            SystemState::SdCardError => {
                blink_on = !blink_on;
                if blink_on { Led::on() } else { Led::off() }
                Timer::after(Duration::from_millis(500)).await;
            }
        }
    }
}

// ============================================================================
// Main
// ============================================================================

bind_interrupts!(struct Irqs {
    I2C0_IRQ => i2c::InterruptHandler<embassy_rp::peripherals::I2C0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("BikeMeter Firmware v{}", env!("CARGO_PKG_VERSION"));
    info!("Starting...");

    defmt::unwrap!(spawner.spawn(led_task()));

    let p = embassy_rp::init(Default::default());

    // LED
    let led = Output::new(p.PIN_25, Level::Low);
    critical_section::with(|cs| { STATE.borrow_ref_mut(cs).led = Some(led); });

    // Stop button
    let button = Input::new(p.PIN_9, Pull::Up);

    // I2C + BNO055 sensor
    let i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let mut delay = Delay;
    let mut sensor = Bno055Sensor::new(i2c);

    match sensor.init(&mut delay) {
        Ok(()) => {
            info!("BNO055 initialized!");
            if let Ok(id) = sensor.id() {
                info!("Chip ID: 0x{:02x}", id);
            }
            if let Err(e) = sensor.set_mode(bno055::BNO055OperationMode::NDOF, &mut delay) {
                defmt::error!("NDOF mode failed: {:?}", defmt::Debug2Format(&e));
                STATE_SIGNAL.signal(SystemState::SensorError);
                loop { Timer::after(Duration::from_secs(1)).await; }
            }
        }
        Err(e) => {
            defmt::error!("BNO055 init failed: {:?}", defmt::Debug2Format(&e));
            STATE_SIGNAL.signal(SystemState::SensorError);
            loop { Timer::after(Duration::from_secs(1)).await; }
        }
    }

    // SPI + SD card
    let spi = Spi::new_blocking(p.SPI1, p.PIN_10, p.PIN_11, p.PIN_12, SpiConfig::default());
    let cs = Output::new(p.PIN_15, Level::High);
    let sd_spi = embedded_hal_bus::spi::ExclusiveDevice::new(spi, cs, Delay)
        .expect("SPI device init failed");
    let sdcard = embedded_sdmmc::SdCard::new(sd_spi, Delay);

    match sdcard.num_bytes() {
        Ok(size) => info!("SD card: {} bytes", size),
        Err(_) => {
            defmt::error!("SD card not detected");
            STATE_SIGNAL.signal(SystemState::SdCardError);
            loop { Timer::after(Duration::from_secs(1)).await; }
        }
    }

    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(sdcard, DummyTimeSource);

    // Extract raw handles in a block so typed wrappers are dropped
    // before we move volume_mgr into SdmmcStorage
    let (raw_dir, raw_file) = {
        let mut volume = match volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(v) => v,
            Err(_) => {
                defmt::error!("Volume open failed");
                STATE_SIGNAL.signal(SystemState::SdCardError);
                loop { Timer::after(Duration::from_secs(1)).await; }
            }
        };
        let mut root_dir = match volume.open_root_dir() {
            Ok(d) => d,
            Err(_) => {
                defmt::error!("Root dir open failed");
                STATE_SIGNAL.signal(SystemState::SdCardError);
                loop { Timer::after(Duration::from_secs(1)).await; }
            }
        };
        let filename = "ride_001.csv";
        let file = match root_dir.open_file_in_dir(filename, Mode::ReadWriteCreateOrAppend) {
            Ok(f) => f,
            Err(_) => {
                defmt::error!("File open failed");
                STATE_SIGNAL.signal(SystemState::SdCardError);
                loop { Timer::after(Duration::from_secs(1)).await; }
            }
        };
        let raw_file = file.to_raw_file();
        let raw_dir = root_dir.to_raw_directory();
        (raw_dir, raw_file)
    };

    let storage = SdmmcStorage::new(volume_mgr, raw_dir, raw_file, 1);

    // Recorder
    let mut recorder = Recorder::new(sensor, storage);
    defmt::unwrap!(recorder.init());

    STATE_SIGNAL.signal(SystemState::Recording);
    info!("Recording started!");

    // Main loop
    loop {
        if button.is_low() {
            info!("Stop button pressed. Shutting down...");
            recorder.shutdown();
            STATE_SIGNAL.signal(SystemState::Stopped);
            info!("Recording stopped. Data saved.");
            loop { Timer::after(Duration::from_secs(1)).await; }
        }

        let elapsed_ms = embassy_time::Instant::now().as_millis();
        let event = recorder.tick(elapsed_ms);

        match event {
            RecorderEvent::SampleRecorded { g_force, pitch, roll } => {
                info!("t={} g={} p={} r={}", elapsed_ms, g_force as u32, pitch as i32, roll as i32);
            }
            RecorderEvent::FileRotated => {
                info!("File rotated");
            }
            RecorderEvent::SensorError => {
                defmt::error!("Sensor read error");
            }
            RecorderEvent::StorageError => {
                defmt::error!("Storage write error");
            }
        }

        Timer::after(Duration::from_millis(10)).await;
    }
}
