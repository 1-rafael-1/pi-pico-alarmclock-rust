//! # Alarm Settings
//! This module contains the functionality to persist the alarm settings in the flash memory.
//!
//! The alarm settings are stored in the flash memory as three separate key/value pairs.
use crate::task::resources::FlashResources;
use crate::task::state::{AlarmSettings, Commands, Events, EVENT_CHANNEL, FLASH_CHANNEL};
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
pub struct PersistedAlarmSettings<'a> {
    flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>,
    flash_range: Range<u32>,
    data_buffer: [u8; 128],
}

impl<'a> PersistedAlarmSettings<'a> {
    /// This function creates a new instance of the PersistedAlarmTime struct.
    /// It takes a FlashResources struct as an argument and returns a PersistedAlarmTime struct.
    pub fn new(r: FlashResources) -> Self {
        let flash = Flash::<_, Async, { FLASH_SIZE }>::new(r.flash, r.dma_ch);
        Self {
            flash_range: 0x1F9000..0x1FC000,
            data_buffer: [0; 128],
            flash,
        }
    }

    /// this function reads the alarm time from the flash memory.
    pub async fn read_alarm_settings_from_flash(&mut self) -> AlarmSettings {
        let keys: [u8; 3] = [0, 1, 2];
        let mut values = [0u8; 3];
        let mut alarm_settings = AlarmSettings::new_empty();

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
        info!("Read alarm settings: {:?}", &values);
        alarm_settings.set_time((values[0], values[1]));
        alarm_settings.set_enabled(values[2] != 0);
        alarm_settings
    }

    /// this function writes the alarm settings to the flash memory.
    /// These values are written to the flash memory in three separate key/value pairs.
    pub async fn write_alarm_settings_to_flash(&mut self, alarm_settings: AlarmSettings) {
        let keys: [u8; 3] = [0, 1, 2];
        let values = [
            alarm_settings.get_hour(),
            alarm_settings.get_minute(),
            alarm_settings.get_enabled().into(),
        ];

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
                        "Alarm settings key {:?} value {:?} stored successfully",
                        &key, &value
                    );
                }
                Err(e) => {
                    // Panic here, because this should not happen and would disrupt the system
                    self::panic!(
                        "Failed to store alarm settings key {:?} value {:?}: {:?}",
                        &key,
                        &value,
                        Debug2Format(&e)
                    );
                }
            }
        }
    }
}

/// This task reads the alarm settings from the flash memory on startup and sends it to the event channel.
/// After that, it waits for commands to update the alarm settings.
#[embassy_executor::task]
pub async fn manage_alarm_settings(_spawner: Spawner, r: FlashResources) {
    let mut persisted_alarm_settings = PersistedAlarmSettings::new(r);
    let receiver = FLASH_CHANNEL.receiver();

    '_read_alarm_settings: {
        // Read the alarm settings from the flash memory only once at the start of the task
        // and send them to the event channel. After that, we can drop this scope.
        let alarm_settings = persisted_alarm_settings
            .read_alarm_settings_from_flash()
            .await;
        let sender = EVENT_CHANNEL.sender();
        sender
            .send(Events::AlarmSettingsReadFromFlash(alarm_settings))
            .await;
    }

    // and then we wait for commands to update the alarm settings
    loop {
        let command = receiver.receive().await;
        match command {
            Commands::AlarmSettingsWriteToFlash(alarm_settings) => {
                info!(
                    "Received alarm settings write command: {:?}",
                    &alarm_settings
                );
                persisted_alarm_settings
                    .write_alarm_settings_to_flash(alarm_settings)
                    .await;
            }
            _ => {
                self::panic!("Unexpected command received: {:?}", command);
            }
        }
    }
}
