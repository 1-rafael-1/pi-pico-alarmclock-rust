use crate::task::resources::FlashResources;
use core::ops::Range;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::flash::Async;
use embassy_rp::flash::Flash;
use embassy_rp::peripherals::FLASH;
use sequential_storage;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{fetch_item, store_item};

/// # FLASH_SIZE
/// The size of the flash memory in bytes.
const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// # PersistedAlarmTime
/// This struct is used to persist the alarm time in the flash memory.
pub struct PersistedAlarmTime<'a> {
    flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>,
    flash_range: Range<u32>,
    data_buffer: [u8; 128],
}

impl<'a> PersistedAlarmTime<'a> {
    /// # new
    /// This function creates a new instance of the PersistedAlarmTime struct.
    /// It takes a FlashResources struct as an argument and returns a PersistedAlarmTime struct.
    pub fn new(r: FlashResources) -> Self {
        let flash = Flash::<_, Async, { FLASH_SIZE }>::new(r.flash, r.dma_ch);
        info!("Flash initialized");
        info!("Flash size: {:?}", FLASH_SIZE);
        Self {
            flash_range: 0x1F9000..0x1FC000,
            data_buffer: [0; 128],
            flash,
        }
    }

    /// this function reads the alarm time from the flash memory.
    /// It returns a tuple of two u8 values representing the hour and minute of the alarm time.
    pub async fn read_alarm_time_from_flash(&mut self) -> (u8, u8) {
        let keys: [u8; 2] = [0, 1];
        let mut values = [0u8; 2];

        for (i, key) in keys.iter().enumerate() {
            values[i] = match fetch_item::<u8, u8, _>(
                &mut self.flash,
                self.flash_range.clone(),
                &mut NoCache::new(),
                &mut self.data_buffer,
                key,
            )
            .await
            {
                Ok(Some(value)) => value,
                Ok(None) => {
                    error!("No value found for key {:?}", &key);
                    // Default to 0, we do not want to panic here, bacaue maybe no value has been stored yet
                    0
                }
                Err(e) => {
                    error!(
                        "Failed to fetch value for key {:?}: {:?}",
                        &key,
                        Debug2Format(&e)
                    );
                    // Default to 0, we do not want to panic here, bacaue maybe no value has been stored yet
                    0
                }
            };
        }
        info!("Fetched alarm time: {:?}:{:?}", &values[0], &values[1]);
        (values[0], values[1])
    }

    /// this function writes the alarm time to the flash memory.
    /// It takes a tuple of two u8 values representing the hour and minute of the alarm time as an argument.
    /// These values are written to the flash memory in two separate key/value pairs.
    pub async fn write_alarm_time_to_flash(&mut self, alarm_time: (u8, u8)) {
        let keys: [u8; 2] = [0, 1];
        let values = [alarm_time.0, alarm_time.1];

        for (key, value) in keys.iter().zip(values.iter()) {
            match store_item::<u8, u8, _>(
                &mut self.flash,
                self.flash_range.clone(),
                &mut NoCache::new(),
                &mut self.data_buffer,
                key,
                value,
            )
            .await
            {
                Ok(_) => {
                    info!(
                        "Alarm time key {:?} value {:?} stored successfully",
                        &key, &value
                    );
                }
                Err(e) => {
                    // Panic here, because this should not happen and would disrupt the system
                    self::panic!(
                        "Failed to store alarm time key {:?} value {:?}: {:?}",
                        &key,
                        &value,
                        Debug2Format(&e)
                    );
                }
            }
        }
    }
}
