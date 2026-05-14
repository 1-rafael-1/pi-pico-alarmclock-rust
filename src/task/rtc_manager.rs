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
//! - Other tasks send requests via channels to:
//!   - Get current time
//!   - Set time
//!   - Schedule alarms
//!   - Wait for alarms (via signal)
//! - The manager task services requests while also waiting for alarm interrupts
//! - Each request type has its own response signal to avoid contention

use defmt::{info, warn};
use embassy_rp::{
    peripherals,
    rtc::{DateTime, DateTimeFilter, Rtc},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, with_timeout};

/// Maximum number of pending RTC requests in the queue
const RTC_REQUEST_QUEUE_SIZE: usize = 5;

/// Requests that can be sent to the RTC manager
#[derive(Debug)]
pub enum RtcRequest {
    /// Get the current date and time
    GetTime,
    /// Set the current date and time
    SetTime(DateTime),
    /// Schedule an alarm with the given filter
    ScheduleAlarm(DateTimeFilter),
    /// Clear the alarm interrupt and disable the alarm
    ClearAndDisableAlarm,
    /// Get the scheduled alarm filter
    GetScheduledAlarm,
}

/// Channel for sending requests to the RTC manager
static RTC_REQUEST_CHANNEL: Channel<CriticalSectionRawMutex, RtcRequest, RTC_REQUEST_QUEUE_SIZE> = Channel::new();

/// Signal with the response for `GetTime` requests (None if RTC not running)
static GET_TIME_RESPONSE: Signal<CriticalSectionRawMutex, Option<DateTime>> = Signal::new();

/// Signal with the response for `SetTime` requests (true = success, false = failure)
static SET_TIME_RESPONSE: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// Signal with the response for `ScheduleAlarm` requests (true = success, false = failure)
static SCHEDULE_ALARM_RESPONSE: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// Signal with the response for `ClearAndDisableAlarm` requests
static CLEAR_ALARM_RESPONSE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal with the response for `GetScheduledAlarm` requests (None if no alarm scheduled)
static GET_SCHEDULED_ALARM_RESPONSE: Signal<CriticalSectionRawMutex, Option<DateTimeFilter>> = Signal::new();

/// Mutex to serialize RTC API calls - prevents concurrent callers from stealing each other's responses
/// Only ONE task can have a pending RTC request at a time
static RTC_API_MUTEX: Mutex<CriticalSectionRawMutex, ()> = Mutex::new(());

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

            // Reset the stop signal to ensure we don't immediately exit if it was set previously
            STOP_ALARM_WAIT_SIGNAL.reset();

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

/// Handle an RTC request and send the response via the appropriate signal
fn handle_request(rtc: &mut Rtc<'static, peripherals::RTC>, request: RtcRequest) {
    match request {
        RtcRequest::GetTime => {
            // Reset signal BEFORE processing to prevent stale responses
            GET_TIME_RESPONSE.reset();
            let time = rtc.now().map_or_else(
                |_| {
                    // warn!("RTC manager: GetTime request -> RTC not running");
                    None
                },
                |dt| {
                    // info!(
                    //     "RTC manager: GetTime request -> {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    //     dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
                    // );
                    Some(dt)
                },
            );
            GET_TIME_RESPONSE.signal(time);
        }
        RtcRequest::SetTime(dt) => {
            SET_TIME_RESPONSE.reset();
            let result = rtc.set_datetime(dt).is_ok();
            if !result {
                warn!("Failed to set RTC time");
            }
            SET_TIME_RESPONSE.signal(result);
        }
        RtcRequest::ScheduleAlarm(filter) => {
            SCHEDULE_ALARM_RESPONSE.reset();
            rtc.schedule_alarm(filter);
            SCHEDULE_ALARM_RESPONSE.signal(true);
        }
        RtcRequest::ClearAndDisableAlarm => {
            CLEAR_ALARM_RESPONSE.reset();
            rtc.clear_interrupt();
            rtc.disable_alarm();
            CLEAR_ALARM_RESPONSE.signal(());
        }
        RtcRequest::GetScheduledAlarm => {
            GET_SCHEDULED_ALARM_RESPONSE.reset();
            let scheduled = rtc.alarm_scheduled();
            info!("RTC manager: GetScheduledAlarm request -> {:?}", scheduled);
            GET_SCHEDULED_ALARM_RESPONSE.signal(scheduled);
        }
    }
}

// ============================================================================
// Public API for other tasks to use
// ============================================================================

/// Get the current date and time from the RTC
/// Returns None if the RTC is not running or if the request times out
pub async fn rtc_get_time() -> Option<DateTime> {
    // Lock the API mutex to ensure only one caller at a time
    // This prevents concurrent requests from stealing each other's responses
    let _lock = RTC_API_MUTEX.lock().await;

    // info!("rtc_get_time: sending request");
    // Send request with timeout to prevent blocking during startup
    if with_timeout(
        Duration::from_millis(200),
        RTC_REQUEST_CHANNEL.send(RtcRequest::GetTime),
    )
    .await
    .is_err()
    {
        warn!("RTC get time request send timed out (RTC manager not ready?)");
        return None;
    }

    // info!("rtc_get_time: waiting for response");
    // Wait for response signal with timeout
    with_timeout(Duration::from_millis(200), GET_TIME_RESPONSE.wait())
        .await
        .ok()
        .flatten()
    // Mutex is released here when _lock drops
}

/// Set the RTC date and time
/// Returns Ok(()) on success, Err(()) on failure or timeout
pub async fn rtc_set_time(dt: DateTime) -> Result<(), ()> {
    // Lock the API mutex to ensure only one caller at a time
    let _lock = RTC_API_MUTEX.lock().await;

    // Send request with timeout
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::SetTime(dt)),
    )
    .await
    .is_err()
    {
        warn!("RTC set time request timed out");
        return Err(());
    }

    // Wait for response signal with timeout
    with_timeout(Duration::from_secs(1), SET_TIME_RESPONSE.wait())
        .await
        .map_or_else(
            |_| {
                warn!("RTC set time response timed out");
                Err(())
            },
            |success| if success { Ok(()) } else { Err(()) },
        )
    // Mutex is released here when _lock drops
}

/// Set the RTC time manually from hour and minute values
/// This gets the current date from RTC and only updates the hour and minute, setting seconds to 0
pub async fn rtc_set_time_manual(hour: u8, minute: u8) {
    info!("Setting RTC time manually to {:02}:{:02}", hour, minute);

    // Get current time to preserve the date
    if let Some(mut current_dt) = rtc_get_time().await {
        current_dt.hour = hour;
        current_dt.minute = minute;
        current_dt.second = 0; // Reset seconds to :00

        if let Err(_e) = rtc_set_time(current_dt).await {
            warn!("Failed to set RTC time manually");
        } else {
            info!("RTC time set manually to {:02}:{:02}", hour, minute);
        }
    } else {
        warn!("Cannot set time manually - RTC not running");
    }
}

/// Schedule an alarm with the given filter
/// Returns Ok(()) on success, Err(()) on failure or timeout
pub async fn rtc_schedule_alarm(filter: DateTimeFilter) -> Result<(), ()> {
    // Lock the API mutex to ensure only one caller at a time
    let _lock = RTC_API_MUTEX.lock().await;

    // Send request with timeout
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::ScheduleAlarm(filter)),
    )
    .await
    .is_err()
    {
        warn!("RTC schedule alarm request timed out");
        return Err(());
    }

    // Wait for response signal with timeout
    with_timeout(Duration::from_secs(1), SCHEDULE_ALARM_RESPONSE.wait())
        .await
        .map_or_else(
            |_| {
                warn!("RTC schedule alarm response timed out");
                Err(())
            },
            |success| if success { Ok(()) } else { Err(()) },
        )
    // Mutex is released here when _lock drops
}

/// Clear the RTC alarm interrupt and disable the alarm
pub async fn rtc_clear_and_disable_alarm() {
    // Lock the API mutex to ensure only one caller at a time
    let _lock = RTC_API_MUTEX.lock().await;

    // Send request with timeout
    if with_timeout(
        Duration::from_secs(1),
        RTC_REQUEST_CHANNEL.send(RtcRequest::ClearAndDisableAlarm),
    )
    .await
    .is_err()
    {
        warn!("RTC clear alarm request timed out");
        return;
    }

    // Wait for response signal with timeout
    let _ = with_timeout(Duration::from_secs(1), CLEAR_ALARM_RESPONSE.wait()).await;
    // Mutex is released here when _lock drops
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
    // Lock the API mutex to ensure only one caller at a time
    let _lock = RTC_API_MUTEX.lock().await;

    info!("rtc_get_scheduled_alarm: sending request");
    // Send request with timeout
    if with_timeout(
        Duration::from_millis(200),
        RTC_REQUEST_CHANNEL.send(RtcRequest::GetScheduledAlarm),
    )
    .await
    .is_err()
    {
        warn!("RTC get scheduled alarm request send timed out");
        return None;
    }

    info!("rtc_get_scheduled_alarm: waiting for response");
    // Wait for response signal with timeout
    with_timeout(Duration::from_millis(200), GET_SCHEDULED_ALARM_RESPONSE.wait())
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
    // Mutex is released here when _lock drops
}
