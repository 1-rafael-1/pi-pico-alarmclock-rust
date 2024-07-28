//! This module keeps the state of the system.
//! This module is responsible for the state transitions of the system, receiving events from the various tasks and reacting to them.
//! Reacting to the events will involve changing the state of the system and triggering actions like updating the display, playing sounds, etc.
use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

/// Events that we want to react to together with the data that we need to react to the event.
/// Works in conjunction with the `EVENT_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Events {
    BlueBtn(u32),
    GreenBtn(u32),
    YellowBtn(u32),
    Vbus(bool),
    Vsys(f32),
    AlarmSettingsReadFromFlash(AlarmSettings),
}

/// Commands that we want to send from the orchestrator to the other tasks that we want to control.
/// Works in conjunction with the `COMMAND_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Commands {
    AlarmSettingsWriteToFlash(AlarmSettings),
    DisplayUpdate(StateManager),
    NeopixelUpdate(StateManager),
    SoundUpdate(StateManager),
}

/// Channel for the events that we want the orchestrator to react to, all state events are of the type Enum Events.
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Events, 10> = Channel::new();
/// Channel for the update commands that we want the orchestrator to send to the display task.
pub static DISPLAY_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();
/// Channel for the update commands that we want the orchestrator to send to the neopixel.
pub static NEOPIXEL_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 3> = Channel::new();
/// Channel for the update commands that we want the orchestrator to send to the flash task.
pub static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();
/// Channel for the update commands that we want the orchestrator to send to the mp3-player task.
pub static SOUND_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

/// All the states of the system are kept in this struct.
#[derive(PartialEq, Debug, Format)]
pub struct StateManager {
    /// The operation mode of the system
    pub operation_mode: OperationMode,
    pub alarm_settings: AlarmSettings,
    pub alarm_state: AlarmState,
    pub power_state: PowerState,
    // more
}

impl StateManager {
    /// Create a new StateManager.             
    /// We will get the actual data pretty early in the system startup, so we can set all this to inits here
    pub fn new() -> Self {
        let manager = StateManager {
            operation_mode: OperationMode::Normal,
            alarm_settings: AlarmSettings::new_empty(),
            alarm_state: AlarmState::None,
            power_state: PowerState {
                usb_power: false,
                vsys: 0.0,
                battery_level: BatteryLevel::Bat000,
            },
        };
        manager
    }

    fn toggle_alarm_enabled(&mut self) {
        self.alarm_settings.enabled = !self.alarm_settings.enabled;
    }

    /// Handle presses of the green button
    fn handle_green_button_press(&mut self) {
        match self.operation_mode {
            OperationMode::Normal => {
                self.toggle_alarm_enabled();
            }
            _ => {
                // ToDo: handle the green button press in other operation modes
            }
        }
    }

    // ToDo: handle the other button presses
}

/// The operation mode of the system
#[derive(PartialEq, Debug, Format)]
pub enum OperationMode {
    /// The regular operation mode, displaying the time, the alarm status, etc. Showing the analog clock on the neopixel
    /// ring, if the alarm is active.
    Normal,
    /// Setting the alarm time, displaying the alarm time and allowing the user to set the new alarm time.
    SetAlarmTime,
    /// The alarm is active, starting with the sunrise effect on the neopixel ring, then playing the alarm sound and displaying the waker effect on the neopixel ring.
    /// on the neopixel ring. Also display and await the color sequence of buttons that need to be pressed to stop the alarm.
    Alarm,
    /// The menu is active, displaying the menu options and allowing the user to select the menu options.
    Menu,
    /// Displaying the system info
    SystemInfo,
}

/// The settings for the alarm
#[derive(PartialEq, Debug, Format, Clone)]
pub struct AlarmSettings {
    /// The alarm time is set to the specified time
    time: (u8, u8),
    /// The alarm is enabled or disabled
    enabled: bool,
}

impl AlarmSettings {
    pub fn new_empty() -> Self {
        AlarmSettings {
            time: (0, 0),
            enabled: false,
        }
    }

    pub fn set_time(&mut self, time: (u8, u8)) {
        self.time = time;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn get_hour(&self) -> u8 {
        self.time.0
    }

    pub fn get_minute(&self) -> u8 {
        self.time.1
    }

    pub fn get_enabled(&self) -> bool {
        self.enabled
    }
}

/// The state of the alarm
#[derive(PartialEq, Debug, Format)]
pub enum AlarmState {
    /// The alarm is not active, the alarm time has not been reached
    None,
    /// The alarm time has been reached, the alarm is active and the sunrise effect is displayed on the neopixel ring. The user
    /// can stop the alarm by pressing the buttons in the correct sequence.
    Sunrise,
    /// We are past the sunrise effect. The alarm sound is playing, the neopixel waker effect is playing. The user can stop the alarm by pressing
    /// the buttons in the correct sequence.
    Noise,
    /// The alarm is being stopped after the correct button sequence has been pressed. The next state will be None.
    StopAlarm,
}

/// The battery level of the system in steps of 20% from 0 to 100. One additional state is provided for charging.
#[derive(PartialEq, Debug, Format)]
pub enum BatteryLevel {
    Charging,
    Bat000,
    Bat020,
    Bat040,
    Bat060,
    Bat080,
    Bat100,
}

/// The power state of the system
#[derive(PartialEq, Debug, Format)]
pub struct PowerState {
    /// The system is running on usb power
    usb_power: bool,
    /// The voltage of the system power supply
    vsys: f32,
    /// The battery level of the system
    /// The battery level is provided in steps of 20% from 0 to 100. One additional state is provided for charging.
    battery_level: BatteryLevel,
}

impl PowerState {
    pub fn set_battery_level(&mut self) {
        if self.usb_power {
            self.battery_level = BatteryLevel::Charging;
        } else {
            // battery level is calculated based on the voltage of the battery, these are values measured on a LiPo battery on this system
            let upper_bound_voltage = 4.1; // fully charged battery
            let lower_bound_voltage = 2.6; // empty battery

            // Calculate battery level based on voltage
            let battery_percent = ((self.vsys - lower_bound_voltage)
                / (upper_bound_voltage - lower_bound_voltage)
                * 100.0) as u8;
            // set the battery level
            self.battery_level = match battery_percent {
                0..=5 => BatteryLevel::Bat000,
                6..=29 => BatteryLevel::Bat020,
                30..=49 => BatteryLevel::Bat040,
                50..=69 => BatteryLevel::Bat060,
                70..=89 => BatteryLevel::Bat080,
                _ => BatteryLevel::Bat100,
            };
        }
    }
}

/// The menu mode of the system
#[derive(PartialEq, Debug, Format)]
pub enum MenuMode {
    /// The default state: the clock is displayed
    None, // the default state: the clock is displayed
    /// The system info menu is being displayed
    SystemInfoMenu,
}

/// options for the system info
#[derive(PartialEq, Debug, Format)]
pub enum SystemInfoMenuMode {
    /// select to either display the system info or shutdown the system
    Select,
    /// display the system info
    Info,
    /// shutdown the system into a low power state
    ShutdownLowPower,
}

/// Task to orchestrate the states of the system
/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// ToDo: in general we will be reacting to a number of event
/// - button presses, multiple things depending on the button and the state of the system
/// - alarm time reached
/// - plugging in usb power
/// and once we reached states we will need to trigger display updates, sound, etc.
#[embassy_executor::task]
pub async fn orchestrate(_spawner: Spawner, rtc_ref: &'static RefCell<Rtc<'static, RTC>>) {
    let mut state_manager = StateManager::new();
    let event_receiver = EVENT_CHANNEL.receiver();
    let flash_sender = FLASH_CHANNEL.sender();

    info!("Orchestrate task started");

    // // just testing: set the alarm time to 7:30 and enable the alarm
    // state_manager.alarm_settings.enabled = true;
    // state_manager.alarm_settings.time = (7, 30);
    // flash_sender
    //     .send(Commands::AlarmSettingsWriteToFlash(
    //         state_manager.alarm_settings.clone(),
    //     ))
    //     .await;

    loop {
        // receive the events, halting the task until an event is received
        let event = event_receiver.receive().await;

        // react to the events
        match event {
            Events::BlueBtn(presses) => {
                info!("Blue button pressed, presses: {}", presses);
            }
            Events::GreenBtn(presses) => {
                state_manager.handle_green_button_press();
            }
            Events::YellowBtn(presses) => {
                info!("Yellow button pressed, presses: {}", presses);
            }
            Events::Vbus(usb) => {
                info!("Vbus event, usb: {}", usb);
                state_manager.power_state.usb_power = usb;
            }
            Events::Vsys(voltage) => {
                info!("Vsys event, voltage: {}", voltage);
                state_manager.power_state.vsys = voltage;
                state_manager.power_state.set_battery_level();
            }
            Events::AlarmSettingsReadFromFlash(alarm_settings) => {
                info!("Alarm time read from flash: {:?}", alarm_settings);
                state_manager.alarm_settings = alarm_settings;
            }
        }

        // at this point we have altered the state of the system, we can now trigger actions based on the state
        // for now we will just log the state
        info!("StateManager: {:?}", state_manager);
        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "orhestrate loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }

        // ToDo: send the state to the display task. This will be straightforward, as we will design the display task to
        // receive the state and update the display accordingly.

        // ToDo: send the state to the sound task. This will be straightforward, as there is only one sound to play, the alarm sound.

        // ToDo: send the state to the neopixel task. This will need a little thinking, as the neopixel hs different effects to display
    }
}
