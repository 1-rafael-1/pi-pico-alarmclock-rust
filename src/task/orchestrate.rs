//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use defmt::{Debug2Format, info, warn};
use embassy_futures::select::select;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Ticker, Timer};

use crate::{
    event::{Event, receive_event, send_event},
    state::{AlarmState, OperationMode, SYSTEM_STATE, SystemState},
    task::{
        alarm_settings::send_flash_write_command,
        alarm_trigger::{signal_alarm_schedule_disable, signal_alarm_schedule_update},
        button_leds::{ButtonLedCommand, signal_button_leds},
        buttons::Button,
        display::signal_display_update,
        light_effects::{signal_lightfx_start, signal_lightfx_stop},
        power::signal_vsys_wake,
        sound::{signal_sound_start, signal_sound_stop},
        time_updater::{RTC_MUTEX, signal_time_updater_resume, signal_time_updater_suspend},
        watchdog::{TaskId, report_task_success},
    },
};

/// Signal for stopping the scheduler
static SCHEDULER_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for starting the scheduler
static SCHEDULER_START_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for waking the scheduler early
static SCHEDULER_WAKE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for the alarm expiry command
static ALARM_EXPIRER_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the scheduler to stop
pub fn signal_scheduler_stop() {
    SCHEDULER_STOP_SIGNAL.signal(());
}

/// Signals the scheduler to start
pub fn signal_scheduler_start() {
    SCHEDULER_START_SIGNAL.signal(());
}

/// Signals the scheduler to wake up early
pub fn signal_scheduler_wake() {
    SCHEDULER_WAKE_SIGNAL.signal(());
}

/// Signals the alarm expirer to start
fn signal_alarm_expirer() {
    ALARM_EXPIRER_SIGNAL.signal(());
}

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrator() {
    info!("Orchestrate task starting");
    // initialize the system state and put it into the mutex
    {
        let system_state = SystemState::new();
        *(SYSTEM_STATE.lock().await) = Some(system_state);
    }

    loop {
        // receive the events, halting the task until an event is received
        let event = receive_event().await;

        // Lock the mutex to get a mutable reference to the system state
        let mut system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.as_mut() else {
            warn!("System state not initialized");
            continue;
        };

        // react to the events
        handle_event(event, system_state).await;

        // Report successful event handling to watchdog
        report_task_success(TaskId::Orchestrator).await;

        drop(system_state_guard);
    }
}

/// Handles a single event by updating the system state and signaling appropriate tasks.
async fn handle_event(event: Event, system_state: &mut SystemState) {
    match event {
        Event::BlueBtn => {
            handle_blue_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::GreenBtn => {
            handle_green_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::YellowBtn => {
            handle_yellow_button_press(system_state).await;
            signal_display_update();
            handle_button_led_on_button_press(system_state);
        }
        Event::Vbus(usb) => {
            info!("Vbus event, usb: {}", usb);
            system_state.power_state.set_usb_power(usb);
            if !system_state.power_state.get_usb_power() {
                signal_vsys_wake();
            }
            signal_display_update();
        }
        Event::Vsys(voltage) => {
            info!("Vsys event, voltage: {}", voltage);
            system_state.power_state.set_vsys(voltage);
            system_state.power_state.set_battery_level();
            signal_display_update();
        }
        Event::AlarmSettingsReadFromFlash(alarm_settings) => {
            info!("Alarm time read from flash: {:?}", alarm_settings);
            system_state.alarm_settings = alarm_settings;
        }
        Event::Scheduler((hour, minute, second)) => {
            info!("Scheduler event");
            handle_scheduler_event(system_state, hour, minute, second);
        }
        Event::RtcUpdated => {
            info!("RTC updated event");
            signal_display_update();
        }
        Event::AlarmSettingsNeedUpdate => {
            info!("Alarm settings must be updated event");
            handle_alarm_settings_update(system_state).await;
        }
        Event::Standby => {
            handle_standby_event();
        }
        Event::WakeUp => {
            handle_wakeup_event();
        }
        Event::Alarm => {
            handle_alarm_event(system_state);
        }
        Event::AlarmStop => {
            handle_alarm_stop_event(system_state);
        }
        Event::SunriseEffectFinished => {
            handle_sunrise_effect_finished_event(system_state);
        }
    }
}

/// Handles the scheduler event which updates display and light effects.
fn handle_scheduler_event(system_state: &SystemState, hour: u8, minute: u8, second: u8) {
    // update the light effects if the alarm is not enabled and the alarm state is None
    if system_state.alarm_state == AlarmState::None && !system_state.alarm_settings.get_enabled() {
        signal_lightfx_start(hour, minute, second);
    }
    // update the display
    signal_display_update();
}

/// Handles alarm settings update by writing to flash and coordinating with alarm task.
async fn handle_alarm_settings_update(system_state: &SystemState) {
    send_flash_write_command(system_state.alarm_settings.clone()).await;

    if system_state.alarm_settings.get_enabled() {
        // if the alarm is enabled, we must update the light effects and signal the alarm task to reschedule
        signal_lightfx_start(0, 0, 0);
        signal_alarm_schedule_update();
    } else {
        // if the alarm is disabled, we must signal the alarm task to disable and wake up the scheduler early
        signal_alarm_schedule_disable();
        signal_scheduler_wake();
    }
}

/// Handles the standby event by stopping scheduler and suspending time updater.
fn handle_standby_event() {
    info!("Standby event");
    signal_scheduler_stop();
    signal_display_update();
    signal_lightfx_start(0, 0, 0);
    signal_sound_stop();
    signal_time_updater_suspend();
}

/// Handles the wake up event by starting scheduler and resuming time updater.
fn handle_wakeup_event() {
    info!("Wake up event");
    signal_scheduler_start();
    signal_vsys_wake();
    signal_time_updater_resume();
}

/// Handles the alarm event by initializing alarm mode and starting effects.
fn handle_alarm_event(system_state: &mut SystemState) {
    info!("Alarm event");
    system_state.randomize_alarm_stop_button_sequence();
    system_state.set_alarm_mode();
    signal_display_update();
    signal_lightfx_start(0, 0, 0);
    signal_alarm_expirer();
    signal_button_leds(ButtonLedCommand::On);
}

/// Handles the alarm stop event by transitioning back to normal mode.
fn handle_alarm_stop_event(system_state: &mut SystemState) {
    info!("Alarm stop event");
    if system_state.alarm_state.is_active() {
        system_state.set_normal_mode();
        signal_display_update();
        signal_lightfx_stop();
        signal_lightfx_start(0, 0, 0);
        signal_sound_stop();
        signal_button_leds(ButtonLedCommand::Off);
    }
}

/// Handles the sunrise effect finished event by transitioning to noise phase.
fn handle_sunrise_effect_finished_event(system_state: &mut SystemState) {
    info!("Sunrise effect finished event");
    system_state.set_alarm_state(AlarmState::Noise);
    signal_sound_start();
    signal_lightfx_start(0, 0, 0);
}

/// Handle state changes when the green button is pressed
async fn handle_green_button_press(system_state: &mut SystemState) {
    match system_state.operation_mode {
        OperationMode::Normal => {
            system_state.toggle_alarm_enabled().await;
        }
        OperationMode::SetAlarmTime => {
            system_state.increment_alarm_hour();
        }
        OperationMode::Menu => system_state.set_system_info_mode(),
        OperationMode::SystemInfo => system_state.set_normal_mode(),
        OperationMode::Alarm => {
            if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Green {
                system_state.alarm_settings.erase_first_valid_stop_alarm_button();
            }
            if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                send_event(Event::AlarmStop).await;
            }
        }
        OperationMode::Standby => {
            system_state.wake_up().await;
        }
    }
}

/// Handles button LED control when a button is pressed (non-alarm mode only)
fn handle_button_led_on_button_press(system_state: &SystemState) {
    // Only trigger the timeout if not in alarm mode
    // During alarm mode, button LEDs are controlled by alarm start/stop
    if system_state.operation_mode != OperationMode::Alarm {
        signal_button_leds(ButtonLedCommand::OnWithTimeout);
    }
}

/// Handle state changes when the blue button is pressed
async fn handle_blue_button_press(system_state: &mut SystemState) {
    match system_state.operation_mode {
        OperationMode::Normal => {
            system_state.set_set_alarm_time_mode();
        }
        OperationMode::SetAlarmTime => {
            system_state.save_alarm_settings().await;
            system_state.set_normal_mode();
        }
        OperationMode::Menu => {
            system_state.set_standby_mode().await;
        }
        OperationMode::SystemInfo => system_state.set_normal_mode(),
        OperationMode::Alarm => {
            if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Blue {
                system_state.alarm_settings.erase_first_valid_stop_alarm_button();
            }
            if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                send_event(Event::AlarmStop).await;
            }
        }
        OperationMode::Standby => {
            system_state.wake_up().await;
        }
    }
}

/// Handle state changes when the yellow button is pressed
async fn handle_yellow_button_press(system_state: &mut SystemState) {
    match system_state.operation_mode {
        OperationMode::Normal => {
            system_state.set_menu_mode();
        }
        OperationMode::Menu | OperationMode::SystemInfo => {
            system_state.set_normal_mode();
        }
        OperationMode::SetAlarmTime => system_state.increment_alarm_minute(),
        OperationMode::Alarm => {
            if system_state.alarm_settings.get_first_valid_stop_alarm_button() == Button::Yellow {
                system_state.alarm_settings.erase_first_valid_stop_alarm_button();
            }
            if system_state.alarm_settings.is_alarm_stop_button_sequence_complete() {
                send_event(Event::AlarmStop).await;
            }
        }
        OperationMode::Standby => {
            system_state.wake_up().await;
        }
    }
}

/// This task handles scheduling periodic display and LED updates by sending events to the Event Channel.
/// Alarm scheduling and triggering is now handled by the dedicated `alarm_trigger_task`.
#[embassy_executor::task]
pub async fn scheduler() {
    info!("scheduler task started");
    // Start with a ticker for the default update rate when alarm is disabled
    let mut ticker = Ticker::every(Duration::from_millis(3740));
    let mut last_alarm_enabled_state: Option<bool> = None;

    'mainloop: loop {
        // see if we must halt the task, then wait for the start signal
        if SCHEDULER_STOP_SIGNAL.signaled() {
            SCHEDULER_STOP_SIGNAL.reset();
            SCHEDULER_START_SIGNAL.wait().await;
        }

        // get the current time
        let dt: DateTime;
        '_rtc_mutex: {
            let rtc_guard = RTC_MUTEX.lock().await;
            let Some(rtc) = rtc_guard.as_ref() else {
                warn!("RTC not initialized");
                drop(rtc_guard);
                Timer::after(Duration::from_secs(3)).await;
                continue 'mainloop;
            };
            dt = match rtc.now() {
                Ok(dt) => dt,
                Err(e) => {
                    info!("RTC not running: {:?}", Debug2Format(&e));
                    // Return an empty DateTime
                    DateTime {
                        year: 0,
                        month: 0,
                        day: 0,
                        day_of_week: DayOfWeek::Monday,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    }
                }
            };
        };

        send_event(Event::Scheduler((dt.hour, dt.minute, dt.second))).await;

        // Report successful scheduler iteration to watchdog
        report_task_success(TaskId::Orchestrator).await;

        // get the alarm enabled state to determine update frequency
        let alarm_enabled: bool;
        '_system_state_mutex: {
            let system_state_guard = SYSTEM_STATE.lock().await;
            let Some(system_state) = system_state_guard.as_ref() else {
                warn!("System state not initialized");
                drop(system_state_guard);
                Timer::after(Duration::from_secs(1)).await;
                continue 'mainloop;
            };
            alarm_enabled = system_state.alarm_settings.get_enabled();
        }

        // Check if the alarm enabled state changed and recreate ticker if needed
        if last_alarm_enabled_state != Some(alarm_enabled) {
            let update_period = if alarm_enabled {
                // When alarm is enabled, we can wait longer since the RTC will handle the alarm
                Duration::from_secs(60)
            } else {
                // if the alarm is not enabled, we will be using the neopixel analog clock effect, which will need to be updated often
                // so we must wait for 3.75 seconds (60s / 16leds -> 3.75s until we must update the leds). To avoid visual glitches, we reduce that time by 10ms
                Duration::from_millis(3740)
            };
            ticker = Ticker::every(update_period);
            last_alarm_enabled_state = Some(alarm_enabled);
        }

        // Wait for either the next tick or an early wake-up signal, whichever comes first
        select(ticker.next(), SCHEDULER_WAKE_SIGNAL.wait()).await;
    }
}

/// This task handles the expiration of the alarm after 5 minutes.
#[embassy_executor::task]
pub async fn alarm_expirer() {
    info!("Alarm expirer task started");
    '_mainloop: loop {
        // wait for the alarm expiry watcher signal
        ALARM_EXPIRER_SIGNAL.wait().await;
        // wait for 5 minutes
        Timer::after(Duration::from_secs(300)).await;
        // send the alarm stop event
        send_event(Event::AlarmStop).await;
        // Report successful alarm expiry to watchdog
        report_task_success(TaskId::Orchestrator).await;
    }
}
