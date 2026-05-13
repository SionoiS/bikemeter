//! Storage abstraction layer.

#[cfg_attr(target_os = "none", derive(defmt::Format))]
#[derive(Debug)]
pub enum StorageError {
    WriteFailed,
    OpenFailed,
}

/// Trait for append-only data logging with rotation support.
pub trait Storage {
    fn write(&mut self, data: &[u8]) -> Result<(), StorageError>;
    fn flush(&mut self) -> Result<(), StorageError>;
    fn rotate(&mut self) -> Result<(), StorageError>;
}
