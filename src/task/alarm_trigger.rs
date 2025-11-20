//! # Alarm Trigger Task
//! This module contains the task that handles RTC alarm scheduling and triggering.
//! It uses the embassy-rp RTC alarm API to schedule alarms and await their triggering,
//! replacing the previous busy-polling approach.

use defmt::{info, warn};
use embassy_rp::rtc::{DateTime, DateTimeFilter, DayOfWeek};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

use crate::{
    event::{Event, send_event},
    state::SYSTEM_STATE,
    task::{
        rtc_manager::{
            rtc_clear_and_disable_alarm, rtc_get_time, rtc_schedule_alarm, rtc_start_alarm_wait, rtc_stop_alarm_wait,
            rtc_wait_for_alarm_signal,
        },
        watchdog::{TaskId, report_task_success},
    },
};

/// Signal to update the alarm schedule when alarm settings change
static ALARM_SCHEDULE_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal to disable the alarm schedule
static ALARM_SCHEDULE_DISABLE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals that the alarm schedule should be updated
pub fn signal_alarm_schedule_update() {
    ALARM_SCHEDULE_UPDATE_SIGNAL.signal(());
}

/// Signals that the alarm schedule should be disabled
pub fn signal_alarm_schedule_disable() {
    ALARM_SCHEDULE_DISABLE_SIGNAL.signal(());
}

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

        // Report successful alarm scheduling to watchdog
        report_task_success(TaskId::AlarmTrigger).await;

        // Tell the RTC manager to start waiting for the alarm
        rtc_start_alarm_wait();

        // Step 4: Wait for alarm trigger or configuration change
        let result = wait_for_alarm_event().await;

        // Tell the RTC manager to stop waiting (we're rescheduling or disabling)
        rtc_stop_alarm_wait();

        // Step 5: Clean up RTC state
        cleanup_rtc_alarm().await;

        // Step 6: Handle the result
        match result {
            AlarmWaitResult::SettingsChanged => {
                info!("Alarm settings changed, rescheduling");
                report_task_success(TaskId::AlarmTrigger).await;
            }
            AlarmWaitResult::Disabled => {
                info!("Alarm disabled by user");
                report_task_success(TaskId::AlarmTrigger).await;
            }
            AlarmWaitResult::Triggered => {
                info!("Alarm triggered! Sending alarm event");
                handle_alarm_triggered().await;
                report_task_success(TaskId::AlarmTrigger).await;
            }
        }
    }
}

/// Reads the current alarm configuration from the system state
async fn get_alarm_config() -> Option<AlarmConfig> {
    let system_state_guard = SYSTEM_STATE.lock().await;
    let system_state = system_state_guard.as_ref()?;

    let config = AlarmConfig {
        enabled: system_state.alarm_settings.get_enabled(),
        hour: system_state.alarm_settings.get_hour(),
        minute: system_state.alarm_settings.get_minute(),
    };

    // Explicitly drop the guard to release the lock early
    drop(system_state_guard);

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
    // Get current time from RTC manager
    let Some(now) = rtc_get_time().await else {
        warn!("Failed to get current time from RTC");
        return false;
    };

    // Determine if we need to schedule for today or tomorrow
    // The alarm fires at HH:MM:00, so we need to check if that exact time has passed
    let alarm_already_passed = is_alarm_time_in_past(&now, config.hour, config.minute);

    let filter = if alarm_already_passed {
        create_alarm_filter_for_tomorrow(&now, config.hour, config.minute)
    } else {
        create_alarm_filter_for_today(config.hour, config.minute).await
    };

    // Schedule the alarm via RTC manager
    rtc_schedule_alarm(filter).await.is_ok()
}

/// Checks if the alarm time has already passed today
/// The alarm triggers at HH:MM:00, so we need to check if that specific second has passed
/// This ensures alarms are ALWAYS scheduled for a time in the future
const fn is_alarm_time_in_past(now: &DateTime, alarm_hour: u8, alarm_minute: u8) -> bool {
    // If alarm hour is in the past, definitely schedule for tomorrow
    if alarm_hour < now.hour {
        return true;
    }

    // If alarm hour is in the future, definitely schedule for today
    if alarm_hour > now.hour {
        return false;
    }

    // Same hour - need to check minutes and seconds
    // The alarm fires at alarm_minute:00 (second 0)

    // If alarm minute is in the past, schedule for tomorrow
    if alarm_minute < now.minute {
        return true;
    }

    // If alarm minute is in the future, schedule for today
    if alarm_minute > now.minute {
        return false;
    }

    // Same hour and minute - the alarm fires at second 0
    // If we're past second 0 in this minute, we've missed it for today
    // We consider it "in the past" even at second 0 to ensure we're always scheduling
    // for a future time (the next occurrence)
    true
}

/// Creates an alarm filter for today at the specified time
async fn create_alarm_filter_for_today(hour: u8, minute: u8) -> DateTimeFilter {
    // Get current time to set the specific date for today
    // If RTC time is unavailable, fall back to time-only filter (shouldn't happen as we check before calling)
    rtc_get_time().await.map_or_else(
        || {
            warn!("RTC time unavailable when creating today's alarm filter, using time-only filter");
            DateTimeFilter::default().hour(hour).minute(minute).second(0)
        },
        |now| {
            info!(
                "Scheduling alarm for today: {:04}-{:02}-{:02} at {:02}:{:02}",
                now.year, now.month, now.day, hour, minute
            );

            DateTimeFilter::default()
                .year(now.year)
                .month(now.month)
                .day(now.day)
                .hour(hour)
                .minute(minute)
                .second(0)
        },
    )
}

/// Creates an alarm filter for tomorrow at the specified time
fn create_alarm_filter_for_tomorrow(now: &DateTime, hour: u8, minute: u8) -> DateTimeFilter {
    let tomorrow = calculate_tomorrow(now);

    info!(
        "Scheduling alarm for tomorrow: {:04}-{:02}-{:02} at {:02}:{:02}",
        tomorrow.year, tomorrow.month, tomorrow.day, hour, minute
    );

    DateTimeFilter::default()
        .year(tomorrow.year)
        .month(tomorrow.month)
        .day(tomorrow.day)
        .hour(hour)
        .minute(minute)
        .second(0)
}

/// Waits for any alarm-related event (trigger, settings change, or disable)
/// This waits on signals from the RTC manager and user settings changes
/// Reports health periodically while waiting to prevent watchdog timeout
async fn wait_for_alarm_event() -> AlarmWaitResult {
    loop {
        // Wait for one of four events: alarm trigger, settings change, disable, or health report timeout
        let result = embassy_futures::select::select4(
            rtc_wait_for_alarm_signal(),
            ALARM_SCHEDULE_UPDATE_SIGNAL.wait(),
            ALARM_SCHEDULE_DISABLE_SIGNAL.wait(),
            Timer::after(Duration::from_secs(240)), // Report health every 4 minutes
        )
        .await;

        // Determine which event occurred based on select result
        match result {
            embassy_futures::select::Either4::First(()) => return AlarmWaitResult::Triggered,
            embassy_futures::select::Either4::Second(()) => {
                ALARM_SCHEDULE_UPDATE_SIGNAL.reset();
                return AlarmWaitResult::SettingsChanged;
            }
            embassy_futures::select::Either4::Third(()) => {
                ALARM_SCHEDULE_DISABLE_SIGNAL.reset();
                return AlarmWaitResult::Disabled;
            }
            embassy_futures::select::Either4::Fourth(()) => {
                // Timeout - report health and continue waiting
                report_task_success(TaskId::AlarmTrigger).await;
            }
        }
    }
}

/// Clears the RTC alarm interrupt and disables the alarm
async fn cleanup_rtc_alarm() {
    rtc_clear_and_disable_alarm().await;
}

/// Handles the alarm trigger event by sending notification and cooling down
async fn handle_alarm_triggered() {
    // Send alarm event to orchestrator
    send_event(Event::Alarm).await;

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
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
