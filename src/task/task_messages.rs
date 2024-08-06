//! # Task messages of the system
//! This module contains the messages that we want to send between the tasks. We have two types of messages: events and commands.
//! Events are the messages that we want the orchestrator to react to. They contain the data that we need to react to the event.
//! Commands are the messages that we want the orchestrator to send to the other tasks that we want to control. They contain the data that we need to send to the other tasks.
//! The messages are sent through channels and signals. The channels are used for the events and the commands are sent through the signals.

use crate::task::state::AlarmSettings;
use defmt::Format;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

/// Events that we want to react to together with the data that we need to react to the event.
/// Works in conjunction with the `EVENT_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Events {
    /// The blue button was pressed, the data is the number of presses
    BlueBtn(u32),
    /// The green button was pressed, the data is the number of presses
    GreenBtn(u32),
    /// The yellow button was pressed, the data is the number of presses
    YellowBtn(u32),
    /// The usb power state has changed, the data is the new state of the usb power
    Vbus(bool),
    /// The system power state has changed, the data is the new voltage of the system power
    Vsys(f32),
    /// The alarm settings have been read from the flash memory, the data is the alarm settings
    AlarmSettingsReadFromFlash(AlarmSettings),
    /// The alarm settings need to be updated in the flash memory
    AlarmSettingsNeedUpdate,
    /// The scheduler has ticked, the data is the time in (hour, minute, second)
    Scheduler((u8, u8, u8)),
    /// The rtc has been updated
    RtcUpdated,
    /// The system must go to standby mode
    Standby,
    /// The system must wake up from standby mode
    WakeUp,
    /// The alarm must be raised
    Alarm,
    /// The alarm must be stopped
    AlarmStop,
    /// The light effect `sunrise` has finished
    SunriseEffectFinished,
}

/// Commands that we want to send from the orchestrator to the other tasks that we want to control.
/// Works in conjunction with the `COMMAND_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Commands {
    /// Write the alarm settings to the flash memory, the data is the alarm settings
    /// Since the alarm settings are small amd rarely changed, we can send them in the command option
    AlarmSettingsWriteToFlash(AlarmSettings),
    /// Update the display with the new state of the system
    /// Since we will need to update the display often and wizth a lot of data, we will not send the data in the command option
    DisplayUpdate,
    /// Update the neopixel. The data is the time in (hour, minute, second), which will be displayed on the neopixel ring in the analog clock mode.
    /// Since the neopixel task runs on a different core, we cannot access the rtc there directly, unless we put it into a mutex, which is overkill
    /// for this simple task. So we will send the time to the neopixel task.
    /// We could theoretically put the time into the state of the system, but that would be a bit of a hack, since the time is not really part of the state of the system.
    /// Having two mutexes for the state of the system and the time would expose us to the risk of deadlocks, so all in all, it is better to send the time here.
    LightFXUpdate((u8, u8, u8)),
    /// Update the sound task with the new state of the system
    /// ToDo: decide if and what data we need to send to the sound task
    SoundUpdate,
    /// Stop the minute timer
    MinuteTimerStop,
    /// Start the minute timer
    MinuteTimerStart,
}

/// For the events that we want the orchestrator to react to, all state events are of the type Enum Events.
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Events, 10> = Channel::new();

/// For the update commands that we want the orchestrator to send to the display task. Since we only ever want to display according to the state of
/// the system, we will not send any data in the command option and we can afford to work only with a simple state of "the display needs to be updated".
pub static DISPLAY_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// For the update commands that we want the orchestrator to send to the minute timer task. Since we only ever want to update the minute timer according to the state of
/// the system, we will not send any data in the command option and we can afford to work only with a simple state of "the minute timer needs to be stopped".
pub static TIMER_STOP_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();
pub static TIMER_START_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Channel for the update commands that we want the orchestrator to send to the flash task.
pub static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

/// Signal for the update commands that we want the orchestrator to send to the neopixel.
pub static LIGHTFX_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();
/// Signal for the stop command that we want the orchestrator to send to the neopixel.
pub static LIGHTFX_STOP_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Signal for the update commands that we want the orchestrator to send to the sound task.
pub static SOUND_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();
