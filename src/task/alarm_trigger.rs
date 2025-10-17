//! # Alarm Trigger Task
//! This module contains the task that handles RTC alarm scheduling and triggering.
//! It uses the embassy-rp RTC alarm API to schedule alarms and await their triggering,
//! replacing the previous busy-polling approach.

use crate::task::state::STATE_MANAGER_MUTEX;
use crate::task::task_messages::{Commands, EVENT_CHANNEL, Events};
use crate::task::time_updater::RTC_MUTEX;
use defmt::{Debug2Format, info, warn};
use embassy_rp::peripherals;
use embassy_rp::rtc::{DateTime, DateTimeFilter, DayOfWeek, Rtc};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};

/// Signal to update the alarm schedule when alarm settings change
pub static ALARM_SCHEDULE_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Signal to disable the alarm schedule
pub static ALARM_SCHEDULE_DISABLE_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Delay after alarm triggers to prevent immediate re-triggering
const POST_ALARM_COOLDOWN: Duration = Duration::from_secs(65);

/// Delay when waiting for initialization
const INIT_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Initial startup delay to allow state manager initialization
const STARTUP_DELAY: Duration = Duration::from_millis(500);

/// Represents the alarm configuration read from state
struct AlarmConfig {
    /// Whether the alarm is enabled
    enabled: bool,
    /// Hour of the alarm (0-23)
    hour: u8,
    /// Minute of the alarm (0-59)
    minute: u8,
}

/// Result of waiting for alarm events
enum AlarmWaitResult {
    /// The RTC alarm triggered
    Triggered,
    /// Alarm settings were changed
    SettingsChanged,
    /// Alarm was disabled
    Disabled,
}

/// This task manages the RTC alarm scheduling based on alarm settings.
/// It schedules an RTC alarm when the alarm is enabled and waits for it to trigger.
/// When the alarm settings change or the alarm is disabled, it updates or disables the schedule accordingly.
#[embassy_executor::task]
pub async fn alarm_trigger_task() {
    info!("Alarm trigger task started");

    // Wait for the state manager to initialize with alarm settings from flash
    Timer::after(STARTUP_DELAY).await;

    loop {
        // Step 1: Get current alarm configuration
        let Some(config) = get_alarm_config().await else {
            // State manager not ready, retry
            Timer::after(INIT_RETRY_DELAY).await;
            continue;
        };

        // Step 2: If alarm is disabled, wait for enable signal
        if !config.enabled {
            info!("Alarm is disabled, waiting for enable signal");
            wait_for_enable_signal().await;
            continue;
        }

        // Step 3: Schedule the alarm in RTC
        if !schedule_alarm(&config).await {
            // Failed to schedule, retry
            Timer::after(INIT_RETRY_DELAY).await;
            continue;
        }

        info!(
            "Alarm scheduled for {:02}:{:02}, waiting for trigger or settings change",
            config.hour, config.minute
        );

        // Step 4: Wait for alarm trigger or configuration change
        let result = wait_for_alarm_event().await;

        // Step 5: Clean up RTC state
        cleanup_rtc_alarm().await;

        // Step 6: Handle the result
        match result {
            AlarmWaitResult::SettingsChanged => {
                info!("Alarm settings changed, rescheduling");
            }
            AlarmWaitResult::Disabled => {
                info!("Alarm disabled by user");
            }
            AlarmWaitResult::Triggered => {
                info!("Alarm triggered! Sending alarm event");
                handle_alarm_triggered().await;
            }
        }
    }
}

/// Reads the current alarm configuration from the state manager
async fn get_alarm_config() -> Option<AlarmConfig> {
    let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
    let state_manager = state_manager_guard.as_ref()?;

    let config = AlarmConfig {
        enabled: state_manager.alarm_settings.get_enabled(),
        hour: state_manager.alarm_settings.get_hour(),
        minute: state_manager.alarm_settings.get_minute(),
    };

    // Explicitly drop the guard to release the lock early
    drop(state_manager_guard);

    Some(config)
}

/// Waits for the alarm to be enabled via signal
async fn wait_for_enable_signal() {
    ALARM_SCHEDULE_UPDATE_SIGNAL.wait().await;
    ALARM_SCHEDULE_UPDATE_SIGNAL.reset();
}

/// Schedules the alarm in the RTC based on the provided configuration
/// Returns true if successful, false if RTC is not available
async fn schedule_alarm(config: &AlarmConfig) -> bool {
    let mut rtc_guard = RTC_MUTEX.lock().await;
    let Some(rtc) = rtc_guard.as_mut() else {
        warn!("RTC not initialized");
        return false;
    };

    // Get current time
    let now = match rtc.now() {
        Ok(dt) => dt,
        Err(e) => {
            warn!(
                "Failed to get current time from RTC: {:?}",
                Debug2Format(&e)
            );
            return false;
        }
    };

    // Determine if we need to schedule for today or tomorrow
    let alarm_already_passed = is_alarm_time_in_past(&now, config.hour, config.minute);

    if alarm_already_passed {
        schedule_alarm_for_tomorrow(rtc, &now, config.hour, config.minute);
    } else {
        schedule_alarm_for_today(rtc, config.hour, config.minute);
    }

    // Explicitly drop the guard to release the lock early
    drop(rtc_guard);

    true
}

/// Checks if the alarm time has already passed today
const fn is_alarm_time_in_past(now: &DateTime, alarm_hour: u8, alarm_minute: u8) -> bool {
    (alarm_hour < now.hour) || (alarm_hour == now.hour && alarm_minute <= now.minute)
}

/// Schedules the alarm for today at the specified time
fn schedule_alarm_for_today(rtc: &mut Rtc<'static, peripherals::RTC>, hour: u8, minute: u8) {
    info!("Scheduling alarm for today at {:02}:{:02}", hour, minute);

    let filter = DateTimeFilter::default()
        .hour(hour)
        .minute(minute)
        .second(0);

    rtc.schedule_alarm(filter);
}

/// Schedules the alarm for tomorrow at the specified time
fn schedule_alarm_for_tomorrow(
    rtc: &mut Rtc<'static, peripherals::RTC>,
    now: &DateTime,
    hour: u8,
    minute: u8,
) {
    let tomorrow = calculate_tomorrow(now);

    info!(
        "Scheduling alarm for tomorrow: {:04}-{:02}-{:02} at {:02}:{:02}",
        tomorrow.year, tomorrow.month, tomorrow.day, hour, minute
    );

    let filter = DateTimeFilter::default()
        .year(tomorrow.year)
        .month(tomorrow.month)
        .day(tomorrow.day)
        .hour(hour)
        .minute(minute)
        .second(0);

    rtc.schedule_alarm(filter);
}

/// Waits for any alarm-related event (trigger, settings change, or disable)
async fn wait_for_alarm_event() -> AlarmWaitResult {
    // Wait for one of three events
    let result = embassy_futures::select::select3(
        wait_for_rtc_alarm(),
        ALARM_SCHEDULE_UPDATE_SIGNAL.wait(),
        ALARM_SCHEDULE_DISABLE_SIGNAL.wait(),
    )
    .await;

    // Determine which event occurred based on select result
    match result {
        embassy_futures::select::Either3::First(()) => AlarmWaitResult::Triggered,
        embassy_futures::select::Either3::Second(_) => {
            ALARM_SCHEDULE_UPDATE_SIGNAL.reset();
            AlarmWaitResult::SettingsChanged
        }
        embassy_futures::select::Either3::Third(_) => {
            ALARM_SCHEDULE_DISABLE_SIGNAL.reset();
            AlarmWaitResult::Disabled
        }
    }
}

/// Helper function to wait for the RTC alarm to trigger
async fn wait_for_rtc_alarm() {
    let mut rtc_guard = RTC_MUTEX.lock().await;
    if let Some(rtc) = rtc_guard.as_mut() {
        rtc.wait_for_alarm().await;
    }
}

/// Clears the RTC alarm interrupt and disables the alarm
async fn cleanup_rtc_alarm() {
    let mut rtc_guard = RTC_MUTEX.lock().await;
    if let Some(rtc) = rtc_guard.as_mut() {
        rtc.clear_interrupt();
        rtc.disable_alarm();
    }
}

/// Handles the alarm trigger event by sending notification and cooling down
async fn handle_alarm_triggered() {
    // Send alarm event to orchestrator
    EVENT_CHANNEL.sender().send(Events::Alarm).await;

    // Cool down period to prevent immediate re-trigger if user stops alarm quickly
    // The alarm will be rescheduled in the next loop iteration if still enabled
    Timer::after(POST_ALARM_COOLDOWN).await;
}

/// Calculate tomorrow's date based on the current datetime
fn calculate_tomorrow(now: &DateTime) -> DateTime {
    let mut tomorrow = now.clone();
    tomorrow.day += 1;

    // Handle month rollover
    if tomorrow.day > 28 {
        let days_in_month = get_days_in_month(tomorrow.month, tomorrow.year);

        if tomorrow.day > days_in_month {
            tomorrow.day = 1;
            tomorrow.month += 1;

            // Handle year rollover
            if tomorrow.month > 12 {
                tomorrow.month = 1;
                tomorrow.year += 1;
            }
        }
    }

    // Update day of week
    tomorrow.day_of_week = next_day_of_week(tomorrow.day_of_week);

    tomorrow
}

/// Get the number of days in a given month and year
const fn get_days_in_month(month: u8, year: u16) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30, // all other months
    }
}

/// Get the next day of the week
const fn next_day_of_week(day: DayOfWeek) -> DayOfWeek {
    match day {
        DayOfWeek::Monday => DayOfWeek::Tuesday,
        DayOfWeek::Tuesday => DayOfWeek::Wednesday,
        DayOfWeek::Wednesday => DayOfWeek::Thursday,
        DayOfWeek::Thursday => DayOfWeek::Friday,
        DayOfWeek::Friday => DayOfWeek::Saturday,
        DayOfWeek::Saturday => DayOfWeek::Sunday,
        DayOfWeek::Sunday => DayOfWeek::Monday,
    }
}

/// Check if a year is a leap year
/// A year is a leap year if it is divisible by 4, but not by 100, unless it is also divisible by 400.
const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}
