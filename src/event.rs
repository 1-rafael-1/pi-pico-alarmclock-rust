//! Events and system channel for sending and receiving events

use crate::task::state::AlarmSettings;
use defmt::Format;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

/// System event channel for sending and receiving events
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Event, EVENT_CHANNEL_CAPACITY> =
    Channel::new();

/// The capacity of the event channel
const EVENT_CHANNEL_CAPACITY: usize = 10;

/// Sends an event to the system channel
pub async fn send_event(event: Event) {
    EVENT_CHANNEL.sender().send(event).await;
}

/// Receives the next event from the system channel
pub async fn receive_event() -> Event {
    EVENT_CHANNEL.receiver().receive().await
}

/// The event type used in the system, representing various system events
#[derive(PartialEq, Debug, Format, Clone)]
pub enum Event {
    /// The blue button was pressed
    BlueBtn,
    /// The green button was pressed
    GreenBtn,
    /// The yellow button was pressed
    YellowBtn,
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
