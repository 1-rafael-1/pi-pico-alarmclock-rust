//! # RTC Manager Task
//!
//! This module provides a centralized task that owns the RTC peripheral and handles
//! all RTC operations via message passing. This eliminates the need for a mutex around
//! the RTC, which was causing issues where waiting for alarms would block other tasks
//! from reading the current time.
//!
//! ## Architecture
//!
//! - The `rtc_manager_task` owns the RTC exclusively
//! - Other tasks send requests via a channel with embedded response channels
//! - The manager task services requests while also waiting for alarm interrupts
//! - Each request carries its own response channel (oneshot pattern)

use defmt::{info, warn};
use embassy_rp::{
    peripherals,
    rtc::{DateTime, DateTimeFilter, Rtc},
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, Sender},
    signal::Signal,
};
use embassy_time::{Duration, with_timeout};

/// Maximum number of pending RTC requests in the queue
const RTC_REQUEST_QUEUE_SIZE: usize = 5;

/// Response type for `GetTime` requests
pub type GetTimeResponse = Option<DateTime>;

/// Response type for `SetTime` requests
pub type SetTimeResponse = Result<(), ()>;

/// Response type for `ScheduleAlarm` requests
pub type ScheduleAlarmResponse = Result<(), ()>;

/// Response type for `GetScheduledAlarm` requests
pub type GetScheduledAlarmResponse = Option<DateTimeFilter>;

/// Requests that can be sent to the RTC manager
/// Each variant carries a response channel for sending the result back
pub enum RtcRequest {
    /// Get the current date and time
    GetTime {
        /// Response channel for sending back the result
        response: Sender<'static, CriticalSectionRawMutex, GetTimeResponse, 1>,
    },
    /// Set the current date and time
    SetTime {
        /// The datetime to set
        dt: DateTime,
        /// Response channel for sending back the result
        response: Sender<'static, CriticalSectionRawMutex, SetTimeResponse, 1>,
    },
    /// Schedule an alarm with the given filter
    ScheduleAlarm {
        /// The alarm filter to schedule
        filter: DateTimeFilter,
        /// Response channel for sending back the result
        response: Sender<'static, CriticalSectionRawMutex, ScheduleAlarmResponse, 1>,
    },
    /// Clear the alarm interrupt and disable the alarm
    ClearAndDisableAlarm,
    /// Get the scheduled alarm filter
    GetScheduledAlarm {
        /// Response channel for sending back the result
        response: Sender<'static, CriticalSectionRawMutex, GetScheduledAlarmResponse, 1>,
    },
}

/// Channel for sending requests to the RTC manager
static RTC_REQUEST_CHANNEL: Channel<CriticalSectionRawMutex, RtcRequest, RTC_REQUEST_QUEUE_SIZE> = Channel::new();

/// Signal that fires when the RTC alarm is triggered
static RTC_ALARM_TRIGGERED_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal to start waiting for an alarm
static START_ALARM_WAIT_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal to stop waiting for an alarm (for rescheduling)
static STOP_ALARM_WAIT_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// The main RTC manager task that owns the RTC peripheral
#[embassy_executor::task]
pub async fn rtc_manager_task(mut rtc: Rtc<'static, peripherals::RTC>) {
    info!("RTC manager task started");

    // Main loop: handle requests and alarm waiting
    loop {
        // Check if we should start waiting for an alarm
        if START_ALARM_WAIT_SIGNAL.signaled() {
            START_ALARM_WAIT_SIGNAL.reset();

            // First, drain any pending requests in the queue before entering alarm wait mode
            // This prevents requests from timing out if they arrived just before the signal
            while let Ok(request) = RTC_REQUEST_CHANNEL.try_receive() {
                handle_request(&mut rtc, request);
            }

            info!("RTC manager: starting alarm wait");

            // Wait for either the alarm to trigger or a stop signal
            wait_for_alarm_or_requests(&mut rtc).await;
        } else {
            // Just handle requests without alarm waiting
            handle_single_request(&mut rtc).await;
        }
    }
}

/// Wait for an alarm to trigger while also handling requests
async fn wait_for_alarm_or_requests(rtc: &mut Rtc<'static, peripherals::RTC>) {
    loop {
        // Use select to handle either alarm trigger, stop signal, or requests
        let result = embassy_futures::select::select3(
            rtc.wait_for_alarm(),
            STOP_ALARM_WAIT_SIGNAL.wait(),
            RTC_REQUEST_CHANNEL.receive(),
        )
        .await;

        match result {
            // Alarm triggered
            embassy_futures::select::Either3::First(()) => {
                info!("RTC manager: alarm triggered!");
                RTC_ALARM_TRIGGERED_SIGNAL.signal(());
                // Exit alarm waiting mode
                break;
            }
            // Stop signal received
            embassy_futures::select::Either3::Second(()) => {
                info!("RTC manager: stop alarm wait signal received");
                STOP_ALARM_WAIT_SIGNAL.reset();
                // Exit alarm waiting mode
                break;
            }
            // Request received - handle it
            embassy_futures::select::Either3::Third(request) => {
                info!("RTC manager: handling request while in alarm wait mode");
                handle_request(rtc, request);
            }
        }
    }
}

/// Handle a single request without alarm waiting
async fn handle_single_request(rtc: &mut Rtc<'static, peripherals::RTC>) {
    let request = RTC_REQUEST_CHANNEL.receive().await;
    handle_request(rtc, request);
}

/// Handle an RTC request and send the response via the request's embedded channel
fn handle_request(rtc: &mut Rtc<'static, peripherals::RTC>, request: RtcRequest) {
    match request {
        RtcRequest::GetTime { response } => {
            let time = rtc.now().map_or_else(
                |_| {
                    warn!("RTC manager: GetTime request -> RTC not running");
                    None
                },
                |dt| {
                    info!(
                        "RTC manager: GetTime request -> {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
                    );
                    Some(dt)
                },
            );
            // Send response back through the oneshot channel
            let _ = response.try_send(time);
        }
        RtcRequest::SetTime { dt, response } => {
            let result = rtc.set_datetime(dt).map_err(|_| ());
            if result.is_err() {
                warn!("Failed to set RTC time");
            }
            let _ = response.try_send(result);
        }
        RtcRequest::ScheduleAlarm { filter, response } => {
            rtc.schedule_alarm(filter);
            let _ = response.try_send(Ok(()));
        }
        RtcRequest::ClearAndDisableAlarm => {
            rtc.clear_interrupt();
            rtc.disable_alarm();
            // No response needed for this fire-and-forget operation
        }
        RtcRequest::GetScheduledAlarm { response } => {
            let scheduled = rtc.alarm_scheduled();
            info!("RTC manager: GetScheduledAlarm request -> {:?}", scheduled);
            let _ = response.try_send(scheduled);
        }
    }
}

// ============================================================================
// Public API for other tasks to use
// ============================================================================

/// Get the current date and time from the RTC
/// Returns None if the RTC is not running or if the request times out
pub async fn rtc_get_time() -> Option<DateTime> {
    // Create a oneshot channel for the response
    static RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, GetTimeResponse, 1> = Channel::new();
    let response_sender = RESPONSE_CHANNEL.sender();

    info!("rtc_get_time: sending request");
    // Send request with timeout to prevent blocking during startup
    if with_timeout(
        Duration::from_millis(200),
        RTC_REQUEST_CHANNEL.send(RtcRequest::GetTime {
            response: response_sender,
        }),
    )
    .await
    .is_err()
    {
        warn!("RTC get time request send timed out (RTC manager not ready?)");
        return None;
    }

    info!("rtc_get_time: waiting for response");
    // Wait for response with timeout
    with_timeout(Duration::from_millis(200), RESPONSE_CHANNEL.receive())
        .await
        .map_or_else(
            |_| {
                warn!("rtc_get_time: timeout waiting for response");
                None
            },
            |dt| {
                info!("rtc_get_time: got response");
                dt
            },
        )
}

/// Set the RTC date and time
/// Returns Ok(()) on success, Err(()) on failure or timeout
pub async fn rtc_set_time(dt: DateTime) -> Result<(), ()> {
    static RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, SetTimeResponse, 1> = Channel::new();
    let response_sender = RESPONSE_CHANNEL.sender();

    // Send request with timeout
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::SetTime {
            dt,
            response: response_sender,
        }),
    )
    .await
    .is_err()
    {
        warn!("RTC set time request timed out");
        return Err(());
    }

    // Wait for response with timeout
    with_timeout(Duration::from_secs(1), RESPONSE_CHANNEL.receive())
        .await
        .unwrap_or_else(|_| {
            warn!("RTC set time response timed out");
            Err(())
        })
}

/// Schedule an alarm with the given filter
/// Returns Ok(()) on success, Err(()) on failure or timeout
pub async fn rtc_schedule_alarm(filter: DateTimeFilter) -> Result<(), ()> {
    static RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, ScheduleAlarmResponse, 1> = Channel::new();
    let response_sender = RESPONSE_CHANNEL.sender();

    // Send request with timeout
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::ScheduleAlarm {
            filter,
            response: response_sender,
        }),
    )
    .await
    .is_err()
    {
        warn!("RTC schedule alarm request timed out");
        return Err(());
    }

    // Wait for response with timeout
    with_timeout(Duration::from_secs(1), RESPONSE_CHANNEL.receive())
        .await
        .unwrap_or_else(|_| {
            warn!("RTC schedule alarm response timed out");
            Err(())
        })
}

/// Clear the RTC alarm interrupt and disable the alarm
pub async fn rtc_clear_and_disable_alarm() {
    // Send request with timeout - this is fire-and-forget, no response needed
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::ClearAndDisableAlarm),
    )
    .await
    .is_err()
    {
        warn!("RTC clear alarm request timed out");
    }
}

/// Signal the RTC manager to start waiting for an alarm
pub fn rtc_start_alarm_wait() {
    START_ALARM_WAIT_SIGNAL.signal(());
}

/// Signal the RTC manager to stop waiting for an alarm (for rescheduling)
pub fn rtc_stop_alarm_wait() {
    STOP_ALARM_WAIT_SIGNAL.signal(());
}

/// Wait for the RTC alarm to trigger
/// This should be used in a select with other signals (like settings change)
pub async fn rtc_wait_for_alarm_signal() {
    RTC_ALARM_TRIGGERED_SIGNAL.wait().await;
    RTC_ALARM_TRIGGERED_SIGNAL.reset();
}

/// Get the currently scheduled alarm filter from the RTC
/// Returns None if no alarm is scheduled or if the request times out
pub async fn rtc_get_scheduled_alarm() -> Option<DateTimeFilter> {
    static RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, GetScheduledAlarmResponse, 1> = Channel::new();
    let response_sender = RESPONSE_CHANNEL.sender();

    info!("rtc_get_scheduled_alarm: sending request");
    // Send request with timeout
    if with_timeout(
        Duration::from_millis(200),
        RTC_REQUEST_CHANNEL.send(RtcRequest::GetScheduledAlarm {
            response: response_sender,
        }),
    )
    .await
    .is_err()
    {
        warn!("RTC get scheduled alarm request send timed out");
        return None;
    }

    info!("rtc_get_scheduled_alarm: waiting for response");
    // Wait for response with timeout
    with_timeout(Duration::from_millis(200), RESPONSE_CHANNEL.receive())
        .await
        .map_or_else(
            |_| {
                warn!("rtc_get_scheduled_alarm: timeout waiting for response");
                None
            },
            |filter| {
                info!("rtc_get_scheduled_alarm: got response");
                filter
            },
        )
}
