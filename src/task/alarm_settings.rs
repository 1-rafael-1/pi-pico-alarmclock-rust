//! # Alarm Settings
//! This module contains the functionality to persist the alarm settings in the flash memory.
//!
//! The alarm settings are stored in the flash memory as three separate key/value pairs.
use crate::event::{Event, send_event};
use crate::task::state::AlarmSettings;
use core::ops::Range;
use defmt::{Debug2Format, info, warn};
use embassy_rp::flash::{Async, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use sequential_storage;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{fetch_item, store_item};

/// Channel for flash write commands
static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, AlarmSettings, 1> = Channel::new();

/// Sends alarm settings to be written to flash
pub async fn send_flash_write_command(settings: AlarmSettings) {
    FLASH_CHANNEL.sender().send(settings).await;
}

/// Waits for the next flash write command
async fn wait_for_flash_write_command() -> AlarmSettings {
    FLASH_CHANNEL.receiver().receive().await
}

/// The size of the flash memory in bytes.
const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// This struct is used to persist the alarm settings in the flash memory.
pub struct PersistedAlarmSettings<'a> {
    /// The flash peripheral used to read and write the alarm settings.
    flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>,
    /// The range of the flash memory used to store the alarm settings.
    flash_range: Range<u32>,
    /// A buffer used for reading and writing data to the flash memory.
    data_buffer: [u8; 128],
}

impl<'a> PersistedAlarmSettings<'a> {
    /// This function creates a new instance of the `PersistedAlarmTime` struct.
    /// It takes a Flash peripheral as an argument and returns a `PersistedAlarmTime` struct.
    pub const fn new(flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>) -> Self {
        Self {
            flash_range: 0x1F_9000..0x1FC_000,
            data_buffer: [0; 128],
            flash,
        }
    }

    /// this function reads the alarm time from the flash memory.
    /// Returns None if there's a critical error reading the settings.
    pub async fn read_alarm_settings_from_flash(&mut self) -> Option<AlarmSettings> {
        let keys: [u8; 3] = [0, 1, 2];
        let mut values = [None; 3];
        let mut has_any_value = false;

        for (i, key) in keys.iter().enumerate() {
            match fetch_item::<u8, u8, _>(
                &mut self.flash,
                self.flash_range.clone(),
                &mut NoCache::new(),
                &mut self.data_buffer,
                key,
            )
            .await
            {
                Ok(Some(value)) => {
                    values[i] = Some(value);
                    has_any_value = true;
                }
                Ok(None) => {
                    info!("No value found for key {:?}", &key);
                }
                Err(e) => {
                    warn!(
                        "Failed to fetch value for key {:?}: {:?}",
                        &key,
                        Debug2Format(&e)
                    );
                }
            }
        }

        // If we didn't read any values successfully, return None
        if !has_any_value {
            warn!("No alarm settings found in flash");
            return None;
        }

        info!("Read alarm settings: {:?}", &values);
        let mut alarm_settings = AlarmSettings::new_empty();
        alarm_settings.set_time((values[0].unwrap_or(0), values[1].unwrap_or(0)));
        alarm_settings.set_enabled(values[2].unwrap_or(0) != 0);
        Some(alarm_settings)
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
                Ok(()) => {
                    info!(
                        "Alarm settings key {:?} value {:?} stored successfully",
                        &key, &value
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to store alarm settings key {:?} value {:?}: {:?}",
                        &key,
                        &value,
                        Debug2Format(&e)
                    );
                    // Continue trying to store other values even if one fails
                }
            }
        }
    }
}

/// This task reads the alarm settings from the flash memory on startup and sends it to the event channel.
/// After that, it waits for commands to update the alarm settings.
#[embassy_executor::task]
pub async fn alarm_settings_handler(flash: Flash<'static, FLASH, Async, { FLASH_SIZE }>) {
    let mut persisted_alarm_settings = PersistedAlarmSettings::new(flash);

    // Read the alarm settings from the flash memory only once at the start of the task
    // and send them to the event channel.
    if let Some(alarm_settings) = persisted_alarm_settings
        .read_alarm_settings_from_flash()
        .await
    {
        send_event(Event::AlarmSettingsReadFromFlash(alarm_settings)).await;
    } else {
        warn!("Failed to read alarm settings from flash on startup");
    }

    // and then we wait for commands to update the alarm settings
    loop {
        let alarm_settings = wait_for_flash_write_command().await;
        info!(
            "Received alarm settings write command: {:?}",
            &alarm_settings
        );
        persisted_alarm_settings
            .write_alarm_settings_to_flash(alarm_settings)
            .await;
    }
}
