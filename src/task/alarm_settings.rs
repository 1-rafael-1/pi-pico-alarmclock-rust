//! # Alarm Settings and System Settings Persistence
//! This module contains the functionality to persist the alarm settings and system settings in the flash memory.
//!
//! The alarm settings are stored in the flash memory as three separate key/value pairs (keys 0-2).
//! The system settings are stored as two additional key/value pairs (keys 3-4).
use core::ops::Range;

use defmt::{Debug2Format, Format, info, warn};
use embassy_rp::{
    flash::{Async, Flash},
    peripherals::FLASH,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use sequential_storage::{
    self,
    cache::NoCache,
    map::{fetch_item, store_item},
};

use crate::{
    event::{Event, send_event},
    state::{AlarmSettings, SystemSettings},
};

/// Commands for writing settings to flash
#[derive(Clone, Debug, Format)]
pub enum SettingsWriteCommand {
    /// Write alarm settings to flash
    AlarmSettings(AlarmSettings),
    /// Write system settings to flash
    SystemSettings(SystemSettings),
}

/// Channel for flash write commands (now handles both alarm and system settings)
static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, SettingsWriteCommand, 2> = Channel::new();

/// Sends a settings write command to flash
pub async fn send_flash_write_command(command: SettingsWriteCommand) {
    FLASH_CHANNEL.sender().send(command).await;
}

/// Waits for the next flash write command
async fn wait_for_flash_write_command() -> SettingsWriteCommand {
    FLASH_CHANNEL.receiver().receive().await
}

/// The size of the flash memory in bytes.
const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// This struct is used to persist both alarm settings and system settings in the flash memory.
pub struct PersistedSettings<'a> {
    /// The flash peripheral used to read and write the alarm settings.
    flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>,
    /// The range of the flash memory used to store the alarm settings.
    flash_range: Range<u32>,
    /// A buffer used for reading and writing data to the flash memory.
    data_buffer: [u8; 128],
}

impl<'a> PersistedSettings<'a> {
    /// This function creates a new instance of the `PersistedSettings` struct.
    /// It takes a Flash peripheral as an argument and returns a `PersistedSettings` struct.
    pub const fn new(flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>) -> Self {
        Self {
            flash_range: 0x1F_9000..0x1FC_000,
            data_buffer: [0; 128],
            flash,
        }
    }

    /// This function reads the alarm settings from the flash memory.
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
                    warn!("Failed to fetch value for key {:?}: {:?}", &key, Debug2Format(&e));
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
                    info!("Alarm settings key {:?} value {:?} stored successfully", &key, &value);
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

    /// This function reads the system settings from the flash memory.
    /// Returns None if there's a critical error reading the settings.
    pub async fn read_system_settings_from_flash(&mut self) -> Option<SystemSettings> {
        let keys: [u8; 2] = [3, 4]; // volume, clock_brightness
        let mut values = [None; 2];
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
                    info!("No value found for system settings key {:?}", &key);
                }
                Err(e) => {
                    warn!(
                        "Failed to fetch value for system settings key {:?}: {:?}",
                        &key,
                        Debug2Format(&e)
                    );
                }
            }
        }

        // If we didn't read any values successfully, return None
        if !has_any_value {
            warn!("No system settings found in flash");
            return None;
        }

        info!("Read system settings: {:?}", &values);
        let mut system_settings = SystemSettings::new_default();
        system_settings.set_volume(values[0].unwrap_or(13));
        system_settings.set_clock_brightness(values[1].unwrap_or(1));
        Some(system_settings)
    }

    /// This function writes the system settings to the flash memory.
    /// These values are written to the flash memory in two separate key/value pairs.
    pub async fn write_system_settings_to_flash(&mut self, system_settings: SystemSettings) {
        let keys: [u8; 2] = [3, 4];
        let values = [system_settings.get_volume(), system_settings.get_clock_brightness()];

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
                    info!("System settings key {:?} value {:?} stored successfully", &key, &value);
                }
                Err(e) => {
                    warn!(
                        "Failed to store system settings key {:?} value {:?}: {:?}",
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

/// This task reads both alarm settings and system settings from the flash memory on startup
/// and sends them to the event channel. After that, it waits for commands to update the settings.
#[embassy_executor::task]
pub async fn alarm_settings_handler(flash: Flash<'static, FLASH, Async, { FLASH_SIZE }>) {
    let mut persisted_settings = PersistedSettings::new(flash);

    // Read the alarm settings from the flash memory only once at the start of the task
    // and send them to the event channel.
    let alarm_settings = persisted_settings
        .read_alarm_settings_from_flash()
        .await
        .unwrap_or_else(|| {
            warn!("No alarm settings found in flash on startup, using defaults");
            AlarmSettings::new_empty()
        });

    // Always send the event, even if we're using defaults
    send_event(Event::AlarmSettingsReadFromFlash(alarm_settings)).await;

    // Read the system settings from the flash memory
    let system_settings = persisted_settings
        .read_system_settings_from_flash()
        .await
        .unwrap_or_else(|| {
            warn!("No system settings found in flash on startup, using defaults");
            SystemSettings::new_default()
        });

    // Always send the event, even if we're using defaults
    send_event(Event::SystemSettingsReadFromFlash(system_settings)).await;

    // Wait for commands to update either alarm settings or system settings
    loop {
        let command = wait_for_flash_write_command().await;
        match command {
            SettingsWriteCommand::AlarmSettings(alarm_settings) => {
                info!("Received alarm settings write command: {:?}", &alarm_settings);
                persisted_settings.write_alarm_settings_to_flash(alarm_settings).await;
            }
            SettingsWriteCommand::SystemSettings(system_settings) => {
                info!("Received system settings write command: {:?}", &system_settings);
                persisted_settings.write_system_settings_to_flash(system_settings).await;
            }
        }
    }
}
