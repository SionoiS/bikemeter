//! embedded-sdmmc storage adapter.

use core::fmt::Write;

use embedded_sdmmc::{BlockDevice, Mode, RawDirectory, RawFile, TimeSource, Timestamp, VolumeManager};

use crate::business::CSV_HEADER;
use crate::storage::{Storage, StorageError};

// ============================================================================
// Time Source
// ============================================================================

pub struct DummyTimeSource;

impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 55,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

// ============================================================================
// SDMMC Storage
// ============================================================================

pub struct SdmmcStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    volume_mgr: VolumeManager<D, T>,
    raw_dir: RawDirectory,
    raw_file: RawFile,
    file_index: u32,
}

impl<D, T> SdmmcStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    pub fn new(
        volume_mgr: VolumeManager<D, T>,
        raw_dir: RawDirectory,
        raw_file: RawFile,
        file_index: u32,
    ) -> Self {
        Self { volume_mgr, raw_dir, raw_file, file_index }
    }
}

impl<D, T> Storage for SdmmcStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    fn write(&mut self, data: &[u8]) -> Result<(), StorageError> {
        self.volume_mgr
            .write(self.raw_file, data)
            .map_err(|_| StorageError::WriteFailed)
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        self.volume_mgr
            .flush_file(self.raw_file)
            .map_err(|_| StorageError::WriteFailed)
    }

    fn rotate(&mut self) -> Result<(), StorageError> {
        self.volume_mgr
            .close_file(self.raw_file)
            .map_err(|_| StorageError::WriteFailed)?;

        self.file_index += 1;
        let name = build_filename(self.file_index);

        self.raw_file = self.volume_mgr
            .open_file_in_dir(self.raw_dir, name.as_str(), Mode::ReadWriteCreate)
            .map_err(|_| StorageError::OpenFailed)?;

        self.volume_mgr
            .write(self.raw_file, CSV_HEADER)
            .map_err(|_| StorageError::WriteFailed)?;

        Ok(())
    }
}

fn build_filename(index: u32) -> heapless::String<16> {
    let mut name = heapless::String::<16>::new();
    let _ = write!(name, "ride_{:03}.csv", index);
    name
}
